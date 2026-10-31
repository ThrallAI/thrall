//! Helpers (sub-agents): a task handed off by Thrall, worked on with a fresh context, the final
//! answer handed back. In the foreground a helper's steps show in the turn and ask permission like
//! any other; in the background it works read-only (plan mode, so nothing asks) and reports when
//! done.

use std::path::Path;

use futures::StreamExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};
use thrall_core::constants::id_prefix;
use thrall_core::ids::{new_id, now_ms};
use thrall_gates::Mode;
use thrall_llm::{ChatMessage, ChatRequest, ChatRole, StreamEvent, ToolCallAssembler, ToolSpec};
use thrall_protocol::{Event, Usage};
use thrall_store::AgentRow;
use thrall_tools::tools::NOT_FOR_HELPERS;
use thrall_tools::{Cancel, ToolOutput};

use crate::checks::Track;
use crate::constants::{
    ASK_THINKER, BACKGROUND_HELPER_NOTE, HELPER_PROMPT, MAX_HELPER_CALLS, MAX_MODEL_CALLS_PER_TURN,
    SPLIT_DOER_NOTE,
};
use crate::error::AgentError;
use crate::state::{AppState, BackgroundTask};
use crate::steps::run_step;
use crate::turn::{CallBy, Emitter, today};
use thrall_core::constants::usage_role;

/// The `agent` tool: run a helper and hand back its answer (or start one in the background).
pub(crate) fn delegate<'a>(
    emitter: &'a Emitter<'a>,
    state: &'a AppState,
    workspace: &'a Path,
    args: &'a Value,
    mode: Mode,
    cancel: &'a Cancel,
) -> BoxFuture<'a, Result<ToolOutput, AgentError>> {
    Box::pin(async move {
        let task = args
            .get("task")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        let title = args
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("a task")
            .to_owned();
        if task.is_empty() {
            return Ok(ToolOutput::error("give the helper a task"));
        }
        if args.get("background").and_then(Value::as_bool) == Some(true) {
            return start_background(emitter, state, workspace, title, task).await;
        }
        let answer = run_helper(emitter, state, workspace, &task, mode, cancel, None).await?;
        Ok(ToolOutput::ok(answer, format!("Helper finished: {title}")))
    })
}

async fn start_background(
    emitter: &Emitter<'_>,
    state: &AppState,
    workspace: &Path,
    title: String,
    task: String,
) -> Result<ToolOutput, AgentError> {
    if !state.can_run_background() {
        return Ok(ToolOutput::error(
            "background helpers only run in the terminal app; do it in the foreground instead \
             (background: false)",
        ));
    }
    let agent_id = new_id(id_prefix::AGENT);
    state
        .store
        .add_agent(
            &state.user_id,
            AgentRow {
                id: agent_id.clone(),
                session_id: emitter.session_id.to_owned(),
                title: title.clone(),
                status: "running".to_owned(),
                state: "{}".to_owned(),
                started_at: now_ms(),
            },
        )
        .await?;
    emitter
        .emit(Event::AgentStarted {
            agent: agent_id.clone(),
            title: title.clone(),
        })
        .await?;
    state.queue_background(BackgroundTask {
        user_id: state.user_id.clone(),
        agent_id: agent_id.clone(),
        session_id: emitter.session_id.to_owned(),
        workspace: workspace.to_path_buf(),
        title,
        task,
    });
    Ok(ToolOutput::ok(
        format!(
            "Started background helper {agent_id}. Carry on; check it later with agent_result. \
             The user is told when it finishes. Background helpers can't change files: if its \
             task asks for one, its answer holds the contents under a 'File: <name>' line, and you \
             save it with write_file when you collect the result."
        ),
        format!("Started {agent_id}"),
    ))
}

fn helper_tools(state: &AppState) -> Vec<ToolSpec> {
    state
        .tools
        .iter()
        .filter(|t| !NOT_FOR_HELPERS.contains(&t.name()))
        .map(|t| ToolSpec {
            name: t.name().to_owned(),
            description: t.description().to_owned(),
            parameters: t.parameters(),
        })
        .collect()
}

/// Work on `task` until the helper answers. Its conversation stays its own; only the answer comes
/// back. With `doer`, the helper is split mode's Doer: its model, its step limit, and a way to ask
/// the Thinker.
pub(crate) async fn run_helper(
    emitter: &Emitter<'_>,
    state: &AppState,
    workspace: &Path,
    task: &str,
    mode: Mode,
    cancel: &Cancel,
    mut doer: Option<&mut crate::split::Doer>,
) -> Result<String, AgentError> {
    let mut mode = mode;
    let mut track = Track::default();
    track.in_helper = true;
    let mut tools = helper_tools(state);
    let (model, role, max_calls) = match &doer {
        Some(d) => {
            tools.push(crate::split::ask_thinker_spec());
            (d.model.clone(), usage_role::DOER, MAX_MODEL_CALLS_PER_TURN)
        }
        None => (state.model(), usage_role::CHAT, MAX_HELPER_CALLS),
    };
    let mut messages = vec![
        ChatMessage::new(
            ChatRole::System,
            format!(
                "{HELPER_PROMPT}{}{}\n\nWorking folder: {}\nOperating system: {}\nToday: {}{}",
                if mode == Mode::Plan && !emitter.records() {
                    BACKGROUND_HELPER_NOTE
                } else {
                    ""
                },
                if doer.is_some() { SPLIT_DOER_NOTE } else { "" },
                workspace.display(),
                std::env::consts::OS,
                today(),
                crate::extras::standing_instructions(state, workspace)
            ),
        ),
        ChatMessage::new(ChatRole::User, task),
    ];
    let mut last = String::new();
    for _ in 0..max_calls {
        if cancel.is_cancelled() {
            return Ok(format!("{last}\n\n(The helper was stopped.)"));
        }
        let request = ChatRequest {
            model: model.clone(),
            messages: messages.clone(),
            max_tokens: state.config.provider.max_tokens,
            tools: tools.clone(),
        };
        let mut text = String::new();
        let mut calls = ToolCallAssembler::default();
        let mut usage = Usage::default();
        let mut cached_tokens = 0;
        let mut stream = state.llm.stream(request);
        let mut failed = None;
        while let Some(item) = stream.next().await {
            match item {
                Ok(StreamEvent::Text(t)) => text.push_str(&t),
                Ok(StreamEvent::ToolCall(d)) => calls.push(d),
                Ok(StreamEvent::Usage(u)) => {
                    usage = Usage {
                        input_tokens: u.input_tokens,
                        output_tokens: u.output_tokens,
                        cost_usd: u.cost_usd,
                    };
                    cached_tokens = u.cached_tokens;
                }
                Ok(_) => {}
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        let by = CallBy {
            model: model.clone(),
            role,
            cached_tokens,
        };
        emitter.record_side_usage_by(state, usage, by).await?;
        if let Some(e) = failed {
            return Ok(format!("The helper couldn't finish: {e}"));
        }
        let calls = calls.finish();
        if calls.is_empty() {
            return Ok(if text.trim().is_empty() {
                "(The helper finished without an answer.)".to_owned()
            } else {
                thrall_gates::redact(&text).0
            });
        }
        last.clone_from(&text);
        let mut asked = ChatMessage::new(ChatRole::Assistant, text);
        asked.tool_calls = calls.clone();
        messages.push(asked);
        for call in &calls {
            let content = if let Some(d) = doer.as_deref_mut()
                && call.name == ASK_THINKER
            {
                crate::split::ask(emitter, state, d, &call.arguments).await?
            } else if NOT_FOR_HELPERS.contains(&call.name.as_str()) {
                format!("Error: helpers can't use {}.", call.name)
            } else {
                run_step(
                    emitter, state, workspace, call, &mut mode, &mut track, cancel,
                )
                .await?
            };
            if let Some(d) = doer.as_deref_mut() {
                d.edited.clone_from(&track.edited);
            }
            messages.push(ChatMessage::tool_result(call.id.clone(), content));
        }
    }
    Ok(format!(
        "{last}\n\n(The helper stopped after {max_calls} steps without finishing.)"
    ))
}

/// Run a queued background helper to the end and report.
pub(crate) async fn run_background(state: &AppState, task: BackgroundTask) {
    let (sink, _ignored) = tokio::sync::mpsc::channel(1);
    // Background steps are neither shown nor recorded; plan mode keeps them read-only.
    let emitter = Emitter::new(state, &task.session_id, &task.agent_id, &sink, false);
    let result = run_helper(
        &emitter,
        state,
        &task.workspace,
        &task.task,
        Mode::Plan,
        &Cancel::never(),
        None,
    )
    .await;
    let (status, answer) = match result {
        Ok(answer) => ("done", answer),
        Err(e) => ("failed", e.to_string()),
    };
    let summary: String = answer
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .chars()
        .take(160)
        .collect();
    if let Err(e) = state
        .store
        .finish_agent(
            &state.user_id,
            &task.agent_id,
            status,
            &json!({ "result": answer }).to_string(),
            now_ms(),
        )
        .await
    {
        tracing::warn!(%e, "couldn't record a helper's result");
    }
    crate::runners::announce(
        state,
        &task.session_id,
        Event::AgentCompleted {
            agent: task.agent_id,
            title: task.title,
            status: status.to_owned(),
            summary,
        },
    )
    .await;
}
