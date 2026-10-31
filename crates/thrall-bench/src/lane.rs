//! One lane: its own copy of the task, its own throwaway database and engine, its turn watched
//! live (tokens, cost per model, steps), stopped at its budget, then the task's check.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use thrall_agent::{AppState, Reply, session, turn};
use thrall_core::Config;
use thrall_core::config::SplitConfig;
use thrall_core::constants::{LOCAL_USER_ID, LOCAL_USER_NAME, person_role};
use thrall_core::ids::now_ms;
use thrall_gates::Mode;
use thrall_llm::OpenRouter;
use thrall_protocol::{Event, PermissionChoice, StepStatus, TurnStatus};
use thrall_store::{SqliteStore, Store, UsageRecord};
use tokio::sync::mpsc;

use crate::app::Bench;
use crate::constants::{
    CHECK_OUTPUT_MAX_CHARS, CHECK_SHELL, CHECK_TIMEOUT_SECS, EVENT_BUFFER, FEED_LINE_MAX_CHARS,
    FEED_MAX_LINES, HIDDEN_DIR, LANE_DB, REFRESH_INTERVAL, SCORE_PREFIX, SKIP_COPY, TASK_FILE,
    WORK_DIR,
};
use crate::model::{CheckResult, LaneKind, LaneSpec, LaneView, ModelCost, Task};

/// A lane ready to start.
pub struct Lane {
    /// Its engine.
    pub state: AppState,
    /// Its session.
    pub session_id: String,
    /// Where its turn works.
    pub work: PathBuf,
}

/// Status names the page shows.
pub mod status {
    pub const PREPARING: &str = "preparing";
    pub const RUNNING: &str = "running";
    pub const CHECKING: &str = "checking";
    pub const DONE: &str = "done";
    pub const OVER_BUDGET: &str = "over budget";
    pub const STOPPING: &str = "stopping";
    pub const STOPPED: &str = "stopped";
    pub const FAILED: &str = "failed";
}

/// Thrall's settings for a lane: the person's provider, checks and memory settings, but none of
/// their hooks, MCP servers or other machinery; conversations aren't remembered (that would cost
/// embeddings and isn't part of the work).
pub fn lane_config(base: &Config, spec: &LaneSpec) -> Config {
    let mut config = Config {
        provider: base.provider.clone(),
        agent: base.agent.clone(),
        memory: base.memory.clone(),
        writing: base.writing.clone(),
        ..Config::default()
    };
    config.memory.remember_conversations = false;
    match spec.kind {
        LaneKind::Single => spec.model.trim().clone_into(&mut config.provider.model),
        LaneKind::Split => {
            // The title is written by the chat model: the cheap one.
            spec.doer.trim().clone_into(&mut config.provider.model);
            config.split = SplitConfig {
                enabled: true,
                thinker_model: spec.thinker.trim().to_owned(),
                doer_model: spec.doer.trim().to_owned(),
            };
        }
    }
    config
}

/// Make the lane's folder, copy the task in, and start its engine and session (full auto: the
/// only questions left are "always ask" ones, which the bench answers no).
pub async fn prepare(
    bench: &Bench,
    lane_dir: &Path,
    spec: &LaneSpec,
    task: Option<&Task>,
) -> anyhow::Result<Lane> {
    let work = lane_dir.join(WORK_DIR);
    std::fs::create_dir_all(&work)?;
    if let Some(task) = task {
        copy_dir(&bench.tasks_dir.join(&task.name), &work, true)?;
    }
    let store = SqliteStore::open(&lane_dir.join(LANE_DB)).await?;
    store
        .ensure_user(LOCAL_USER_ID, LOCAL_USER_NAME, now_ms())
        .await?;
    store.set_role(LOCAL_USER_ID, person_role::OWNER).await?;
    let config = lane_config(&bench.base, spec);
    let llm = OpenRouter::new(bench.api_key.clone(), &config.provider.base_url)?;
    let mut state = AppState::new(
        config,
        Arc::new(store),
        Arc::new(llm),
        LOCAL_USER_ID.to_owned(),
    )?;
    state.load_terms().await?;
    state.data_dir = Some(lane_dir.to_path_buf());
    let session = session::create(&state, Some(work.to_string_lossy().into_owned())).await?;
    session::set_mode(&state, &session.id, Mode::FullAuto).await?;
    Ok(Lane {
        state,
        session_id: session.id,
        work,
    })
}

/// Run the lane's turn from `start` (shared by every lane), keeping its view up to date, then
/// the check.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    bench: &Bench,
    run_id: &str,
    index: usize,
    lane: &Lane,
    prompt: &str,
    budget_usd: f64,
    start: Instant,
    task: Option<&Task>,
) {
    let update = |f: &mut dyn FnMut(&mut LaneView)| bench.update(run_id, index, f);
    update(&mut |v| v.status = status::RUNNING.to_owned());
    let state = &lane.state;
    let session_id = lane.session_id.as_str();
    let (tx, mut rx) = mpsc::channel(EVENT_BUFFER);
    let go = async move {
        let result = turn::run(state, session_id, prompt, &tx).await;
        drop(tx);
        result
    };
    let watch = async {
        let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
        let mut turn_id: Option<String> = None;
        let mut over = false;
        loop {
            tokio::select! {
                frame = rx.recv() => {
                    let Some(frame) = frame else { break };
                    if let Event::TurnStarted { turn, .. } = &frame.event {
                        turn_id = Some(turn.clone());
                    }
                    if let Event::PermissionRequest { request, .. } = &frame.event {
                        // Only "always ask" actions get here in full auto: nobody's watching.
                        state.prompts.answer(
                            request,
                            &state.user_id,
                            Reply::Permission(PermissionChoice::No),
                            "bench",
                        );
                    }
                    update(&mut |v| note_event(v, &frame.event));
                }
                _ = ticker.tick() => {
                    let rows = usage_rows(state, turn_id.as_deref()).await;
                    let elapsed = elapsed_ms(start);
                    let mut cost = 0.0;
                    update(&mut |v| {
                        apply_usage(v, &rows);
                        v.elapsed_ms = elapsed;
                        cost = v.cost_usd;
                    });
                    if !over && cost > budget_usd {
                        over = true;
                        state.interrupt(session_id);
                        update(&mut |v| {
                            v.status = status::OVER_BUDGET.to_owned();
                            push_feed(v, format!("Stopped: passed the ${budget_usd:.2} budget"));
                        });
                    }
                }
            }
        }
        turn_id
    };
    let (result, turn_id) = tokio::join!(go, watch);
    let elapsed = elapsed_ms(start);
    let rows = usage_rows(state, turn_id.as_deref()).await;
    let mut stopped_by_person = false;
    update(&mut |v| {
        apply_usage(v, &rows);
        v.elapsed_ms = elapsed;
        match &result {
            Ok(summary) => {
                v.reply.clone_from(&summary.reply);
                if v.status == status::OVER_BUDGET {
                    return;
                }
                v.status = match summary.status {
                    TurnStatus::Interrupted => {
                        stopped_by_person = true;
                        status::STOPPED
                    }
                    TurnStatus::Failed => status::FAILED,
                    TurnStatus::Done => status::DONE,
                }
                .to_owned();
            }
            Err(e) => {
                v.status = status::FAILED.to_owned();
                v.error = Some(e.to_string());
            }
        }
    });
    let Some(task) = task.filter(|t| !t.check.trim().is_empty()) else {
        return;
    };
    if stopped_by_person || result.is_err() {
        return;
    }
    let final_status = bench
        .lane(run_id, index)
        .map(|v| v.status)
        .unwrap_or_default();
    update(&mut |v| v.status = status::CHECKING.to_owned());
    let hidden = bench.tasks_dir.join(&task.name).join(HIDDEN_DIR);
    if hidden.is_dir()
        && let Err(e) = copy_dir(&hidden, &lane.work, false)
    {
        update(&mut |v| push_feed(v, format!("Couldn't copy the hidden tests: {e}")));
    }
    let checked = check(&lane.work, &task.check).await;
    update(&mut |v| {
        push_feed(
            v,
            format!(
                "Check {}{}",
                if checked.passed { "passed" } else { "failed" },
                checked
                    .score
                    .map(|(p, t)| format!(" ({p}/{t})"))
                    .unwrap_or_default()
            ),
        );
        v.check = Some(checked.clone());
        v.status.clone_from(&final_status);
    });
}

fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

async fn usage_rows(state: &AppState, turn_id: Option<&str>) -> Vec<UsageRecord> {
    match turn_id {
        Some(turn) => state
            .store
            .turn_usage(&state.user_id, turn)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Totals, and the same per model and role (in the order they first appear).
pub fn apply_usage(view: &mut LaneView, rows: &[UsageRecord]) {
    let mut by_model: Vec<ModelCost> = Vec::new();
    for row in rows {
        let slot = match by_model
            .iter()
            .position(|m| m.model == row.model && m.role == row.role)
        {
            Some(i) => i,
            None => {
                by_model.push(ModelCost {
                    model: row.model.clone(),
                    role: row.role.clone(),
                    ..ModelCost::default()
                });
                by_model.len() - 1
            }
        };
        let m = &mut by_model[slot];
        m.calls += 1;
        m.input_tokens += row.input_tokens;
        m.cached_tokens += row.cached_tokens;
        m.output_tokens += row.output_tokens;
        m.cost_usd += row.cost_usd.unwrap_or(0.0);
    }
    view.model_calls = by_model.iter().map(|m| m.calls).sum();
    view.input_tokens = by_model.iter().map(|m| m.input_tokens).sum();
    view.cached_tokens = by_model.iter().map(|m| m.cached_tokens).sum();
    view.output_tokens = by_model.iter().map(|m| m.output_tokens).sum();
    view.cost_usd = by_model.iter().map(|m| m.cost_usd).sum();
    view.by_model = by_model;
}

/// Count and describe what the turn did.
fn note_event(view: &mut LaneView, event: &Event) {
    match event {
        Event::StepStarted { tool, title, .. } => {
            view.steps += 1;
            match tool.as_str() {
                "hand_off" => view.hand_offs += 1,
                "ask_thinker" => view.asks += 1,
                _ => {}
            }
            push_feed(view, format!("> {title}"));
        }
        Event::StepCompleted {
            status: s, summary, ..
        } => match s {
            StepStatus::Denied => {
                view.denied += 1;
                push_feed(view, format!("  refused: {summary}"));
            }
            StepStatus::Error => push_feed(view, format!("  error: {summary}")),
            _ => {}
        },
        Event::PermissionRequest { summary, .. } => {
            push_feed(view, format!("  asked \"{summary}\": answered no"));
        }
        Event::MessageCompleted { text, .. } if !text.trim().is_empty() => {
            push_feed(view, format!("Thrall: {}", text.trim()));
        }
        Event::Notice { text, .. } => push_feed(view, format!("note: {text}")),
        Event::Error { message, .. } => {
            push_feed(view, format!("error: {message}"));
            view.error = Some(message.clone());
        }
        _ => {}
    }
}

fn push_feed(view: &mut LaneView, line: String) {
    let line: String = line
        .replace('\n', " ")
        .chars()
        .take(FEED_LINE_MAX_CHARS)
        .collect();
    view.feed.push(line);
    if view.feed.len() > FEED_MAX_LINES {
        let extra = view.feed.len() - FEED_MAX_LINES;
        view.feed.drain(..extra);
    }
}

/// Run the task's check in the lane's folder.
async fn check(work: &Path, command: &str) -> CheckResult {
    let (shell, args) = CHECK_SHELL;
    let mut cmd = tokio::process::Command::new(shell);
    cmd.args(args)
        .arg(command)
        .current_dir(work)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null());
    let timeout = std::time::Duration::from_secs(CHECK_TIMEOUT_SECS);
    let (passed, text) = match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(out)) => (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        ),
        Ok(Err(e)) => (false, format!("The check couldn't start: {e}")),
        Err(_) => (
            false,
            format!("The check didn't finish within {CHECK_TIMEOUT_SECS} seconds."),
        ),
    };
    let total = text.chars().count();
    let output: String = text
        .chars()
        .skip(total.saturating_sub(CHECK_OUTPUT_MAX_CHARS))
        .collect();
    CheckResult {
        passed,
        score: score(&text),
        output,
    }
}

/// The last `SCORE passed/total` line a check printed.
pub fn score(output: &str) -> Option<(u32, u32)> {
    output.lines().rev().find_map(|line| {
        let (passed, total) = line.trim().strip_prefix(SCORE_PREFIX)?.split_once('/')?;
        Some((passed.trim().parse().ok()?, total.trim().parse().ok()?))
    })
}

/// Copy a folder's contents, leaving out build output and dependencies; at the top of a task
/// (`task_root`), also its `task.toml` and hidden tests.
fn copy_dir(from: &Path, to: &Path, task_root: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if SKIP_COPY.contains(&name_str.as_ref())
            || (task_root && (name_str == TASK_FILE || name_str == HIDDEN_DIR))
        {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target, false)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(model: &str, role: &str, input: u64, cached: u64, cost: f64) -> UsageRecord {
        UsageRecord {
            id: String::new(),
            user_id: String::new(),
            session_id: None,
            turn_id: None,
            provider: String::new(),
            model: model.into(),
            role: role.into(),
            input_tokens: input,
            cached_tokens: cached,
            output_tokens: 10,
            cost_usd: Some(cost),
            at: 0,
        }
    }

    #[test]
    fn usage_adds_up_per_model_and_role() {
        let mut view = LaneView::default();
        let rows = [
            row("big", "thinker", 1000, 0, 0.05),
            row("cheap", "doer", 2000, 1500, 0.01),
            row("cheap", "doer", 3000, 2500, 0.02),
            row("big", "ask", 500, 0, 0.02),
        ];
        apply_usage(&mut view, &rows);
        assert_eq!(view.model_calls, 4);
        assert_eq!(view.input_tokens, 6500);
        assert_eq!(view.cached_tokens, 4000);
        assert!((view.cost_usd - 0.10).abs() < 1e-9);
        assert_eq!(view.by_model.len(), 3);
        assert_eq!(view.by_model[1].calls, 2);
        assert_eq!(view.by_model[1].role, "doer");
    }

    #[test]
    fn the_last_score_line_counts() {
        assert_eq!(score("SCORE 1/9\nmore\nSCORE 7/9\n"), Some((7, 9)));
        assert_eq!(score("  SCORE 3 / 4  "), Some((3, 4)));
        assert_eq!(score("no score here"), None);
        assert_eq!(score("SCORE x/4"), None);
    }

    #[test]
    fn a_split_lane_runs_split_mode_and_a_single_lane_doesnt() {
        let base = Config::default();
        let split = LaneSpec {
            label: String::new(),
            kind: LaneKind::Split,
            model: String::new(),
            thinker: " big/one ".into(),
            doer: "cheap/one".into(),
        };
        let c = lane_config(&base, &split);
        assert!(c.split.enabled);
        assert_eq!(c.split.thinker_model, "big/one");
        assert_eq!(c.provider.model, "cheap/one");
        assert!(!c.memory.remember_conversations);
        let single = LaneSpec {
            kind: LaneKind::Single,
            model: "solo/one".into(),
            ..split
        };
        let c = lane_config(&base, &single);
        assert!(!c.split.enabled);
        assert_eq!(c.provider.model, "solo/one");
        assert!(c.hooks.is_empty() && c.mcp.servers.is_empty());
    }

    #[test]
    fn task_files_and_hidden_tests_stay_out_of_the_lane() {
        let root = std::env::temp_dir().join(format!("thrall-bench-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let task = root.join("task");
        std::fs::create_dir_all(task.join("src")).unwrap();
        std::fs::create_dir_all(task.join(HIDDEN_DIR)).unwrap();
        std::fs::create_dir_all(task.join("target")).unwrap();
        std::fs::write(task.join(TASK_FILE), "prompt = 'x'").unwrap();
        std::fs::write(task.join("src").join("a.py"), "a").unwrap();
        std::fs::write(task.join(HIDDEN_DIR).join("test_a.py"), "t").unwrap();
        let work = root.join("work");
        copy_dir(&task, &work, true).unwrap();
        assert!(work.join("src").join("a.py").exists());
        assert!(!work.join(TASK_FILE).exists());
        assert!(!work.join(HIDDEN_DIR).exists());
        assert!(!work.join("target").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
