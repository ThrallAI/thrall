//! One tool call as a step: precheck, the tool gate (by permission mode), asking the user, the
//! plan flow, live output, the context gate, and the audit log.

use std::path::Path;

use serde_json::{Value, json};
use thrall_core::constants::id_prefix;
use thrall_core::ids::{new_id, now_ms};
use thrall_gates::constants::{decision, gate};
use thrall_gates::{Decision, EDIT_GRANT, Effect, Mode, tool_gate};
use thrall_llm::ToolCall;
use thrall_protocol::{Event, PermissionChoice, Risk, StepStatus};
use thrall_store::{AuditEntry, NewStep};
use thrall_tools::companion::{Call, Checked, Ran};
use thrall_tools::tool::arg_str;
use thrall_tools::tools::{
    ASK_USER, DELEGATE, EXIT_PLAN_MODE, EXPERIENCE_NOTE, MAKE_IMAGE, MAKE_VIDEO, MEMORY_SAVE,
    SPEAK, TRANSCRIBE, USE_SKILL, VIDEO_TRANSCRIPT, VIEW_IMAGE, find_skill, skill_text,
};
use thrall_tools::{Cancel, ToolContext, ToolOutput};
use tokio::sync::mpsc;

use crate::checks::Track;
use crate::constants::{
    BACKGROUND_CANT_CHANGE, BLOCKED_BY_HOOK, BLOCKED_BY_PLAN, BLOCKED_UNATTENDED, CHECK_IN_BLOCKED,
    CHECK_IN_READ_ONLY, NOT_LOOKING, SKILL_HELPER_DONE, SKILL_HELPER_TASK, THINKER_BLOCKED,
    THINKER_CANT, THINKER_LOOK_ONLY, TOOL_MAX_SECS, TOOL_STOP_GRACE_SECS, step_status,
};
use crate::error::AgentError;
use crate::hooks;
use crate::prompts::Reply;
use crate::state::AppState;
use crate::turn::Emitter;
use thrall_core::config::{HookEvent, Humanise};

/// Write a gate decision to the audit log. Details are scrubbed of secrets first.
pub(crate) async fn audit(
    state: &AppState,
    gate_name: &str,
    action: &str,
    decided: &str,
    detail: Value,
) -> Result<(), AgentError> {
    let (detail, _) = thrall_gates::redact(&detail.to_string());
    state
        .store
        .append_audit(AuditEntry {
            id: new_id(id_prefix::AUDIT),
            user_id: state.user_id.clone(),
            at: now_ms(),
            gate: gate_name.to_owned(),
            action: action.to_owned(),
            decision: decided.to_owned(),
            detail,
        })
        .await?;
    Ok(())
}

/// What asking the user came to.
enum Asked {
    Yes,
    Always,
    No,
    Interrupted,
}

/// Run one tool call as a step. Returns the result for the model's context (already through the
/// context gate); the caller stores it.
pub(crate) async fn run_step(
    emitter: &Emitter<'_>,
    state: &AppState,
    workspace: &Path,
    call: &ToolCall,
    mode: &mut Mode,
    track: &mut Track,
    cancel: &Cancel,
) -> Result<String, AgentError> {
    let step_id = new_id(id_prefix::STEP);
    let args: Value = if call.arguments.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(&call.arguments).unwrap_or(Value::Null)
    };
    let tool = state.tool(&call.name).cloned();
    let title = match &tool {
        Some(t) => t.title(&args),
        None => format!("Unknown tool {}", call.name),
    };
    emitter
        .emit(Event::StepStarted {
            turn: emitter.turn_id.to_owned(),
            step: step_id.clone(),
            tool: call.name.clone(),
            title: title.clone(),
            helper: track.helper_step.clone(),
        })
        .await?;
    if emitter.records() {
        state
            .store
            .start_step(NewStep {
                id: step_id.clone(),
                user_id: state.user_id.clone(),
                turn_id: emitter.turn_id.to_owned(),
                tool: call.name.clone(),
                title,
                args: thrall_gates::redact(&call.arguments).0,
                at: now_ms(),
            })
            .await?;
    }

    let (output_tx, mut output_rx) = mpsc::unbounded_channel();
    let ctx = ToolContext {
        workspace: workspace.to_path_buf(),
        store: state.store.clone(),
        user_id: state.user_id.clone(),
        session_id: emitter.session_id.to_owned(),
        http: state.http.clone(),
        output: output_tx,
        reads: state.reads_for(emitter.session_id),
        cancel: cancel.clone(),
        // The tools' data folder is the person's own (their uploads, skills, browser profile).
        data_dir: state.my_dir(),
        mcp: (!state.mcp.is_empty()).then(|| state.mcp.clone()),
        memory: Some(state.memory.clone()),
        browser: state.browsers.clone(),
        disabled_skills: state.config.skills.disabled.clone(),
    };

    // A session working on a companion PC checks and runs its PC tools there.
    let pc = tool
        .as_ref()
        .filter(|t| thrall_tools::companion::runs_on_pc(t.name()))
        .and_then(|_| state.companions.pc_of(emitter.session_id));

    let mut status: Option<StepStatus> = None;
    let mut step_summary: Option<String> = None;
    let output = match &tool {
        None => ToolOutput::error(format!(
            "there is no tool called {}. Use one of the tools you were given.",
            call.name
        )),
        Some(_) if args.is_null() => ToolOutput::error(format!(
            "the arguments for {} weren't valid JSON. Send a JSON object.",
            call.name
        )),
        // Split mode's Thinker isn't offered these; refused if it calls one anyway.
        Some(t) if track.look_only && THINKER_CANT.contains(&t.name()) => {
            status = Some(StepStatus::Denied);
            step_summary = Some(THINKER_BLOCKED.to_owned());
            ToolOutput::error(THINKER_LOOK_ONLY)
        }
        Some(t) if t.name() == DELEGATE && emitter.records() => {
            crate::workers::delegate(emitter, state, workspace, &args, *mode, cancel, &step_id)
                .await?
        }
        Some(t) if t.name() == SPEAK => {
            match crate::audio::speak_tool(
                emitter, state, workspace, &step_id, &args, *mode, cancel,
            )
            .await?
            {
                Ok(output) => output,
                Err((why, s, shown)) => {
                    status = Some(s);
                    step_summary = Some(shown);
                    ToolOutput::error(why)
                }
            }
        }
        Some(t) if t.name() == TRANSCRIBE => {
            crate::audio::transcribe_tool(emitter, state, workspace, &step_id, &args, *mode, cancel)
                .await?
        }
        Some(t) if t.name() == VIDEO_TRANSCRIPT => match t.precheck(&ctx, &args) {
            Some(why) => ToolOutput::error(why),
            None => {
                let effect = t.effect(&ctx, &args);
                match crate::audio::video_transcript_tool(
                    emitter, state, workspace, &step_id, &args, &effect, *mode, cancel,
                )
                .await?
                {
                    Ok(output) => output,
                    Err((why, s, shown)) => {
                        status = Some(s);
                        step_summary = Some(shown);
                        ToolOutput::error(why)
                    }
                }
            }
        },
        Some(t) if t.name() == VIEW_IMAGE => {
            crate::images::view_image(emitter, state, workspace, &args).await?
        }
        Some(t) if t.name() == MAKE_VIDEO => {
            match crate::videos::make_video(
                emitter, state, workspace, &step_id, &args, *mode, cancel,
            )
            .await?
            {
                Ok(output) => output,
                Err((why, s, shown)) => {
                    status = Some(s);
                    step_summary = Some(shown);
                    ToolOutput::error(why)
                }
            }
        }
        Some(t) if t.name() == MAKE_IMAGE => {
            match crate::images::make_image(
                emitter, state, workspace, &step_id, &args, *mode, cancel,
            )
            .await?
            {
                Ok(output) => output,
                Err((why, s, shown)) => {
                    status = Some(s);
                    step_summary = Some(shown);
                    ToolOutput::error(why)
                }
            }
        }
        Some(t) if t.name() == ASK_USER => {
            crate::questions::ask(emitter, state, &args, *mode, track, cancel).await?
        }
        Some(t) if t.name() == EXPERIENCE_NOTE => {
            match crate::experience::note(state, arg_str(&args, "lesson")) {
                Ok(lesson) => ToolOutput::ok(
                    format!("Noted: {}", lesson.text),
                    format!("Noted a lesson: {}", lesson.text),
                ),
                Err(why) => ToolOutput::error(why),
            }
        }
        Some(t) if t.name() == EXIT_PLAN_MODE && *mode == Mode::Plan => {
            propose_plan(emitter, state, &args, mode, cancel).await?
        }
        // A skill that runs in a helper: only its result comes back to the conversation.
        Some(t)
            if t.name() == USE_SKILL
                && emitter.records()
                && !track.in_helper
                && find_skill(&ctx, arg_str(&args, "name")).is_some_and(|s| s.helper) =>
        {
            run_skill_in_helper(
                emitter, state, workspace, &ctx, &args, *mode, cancel, &step_id,
            )
            .await?
        }
        Some(t) => match checked(
            state,
            emitter.session_id,
            workspace,
            pc.as_deref(),
            t.as_ref(),
            &ctx,
            &args,
        )
        .await
        {
            Err(why) => ToolOutput::error(why),
            Ok(mut effect) => {
                // Saving facts without asking is off: ask, showing the fact.
                if t.name() == MEMORY_SAVE && !state.config.memory.save_facts {
                    effect = Effect::AlwaysAsk {
                        summary: format!(
                            "Remember “{}”: {}",
                            arg_str(&args, "topic"),
                            arg_str(&args, "content")
                        ),
                        risk: Risk::Low,
                        diff: None,
                    };
                }
                // Your pre_tool hooks run before anything else decides.
                let hook = hooks::fire(
                    state,
                    HookEvent::PreTool,
                    Some(t.name()),
                    json!({ "tool": t.name(), "arguments": args }),
                    workspace,
                )
                .await;
                let allowed = match hook.blocked {
                    // Split mode's Thinker only looks; the Doer makes changes.
                    _ if track.look_only && !matches!(effect, Effect::Read) => {
                        audit(
                            state,
                            gate::TOOL,
                            &call.name,
                            decision::DENY,
                            json!({ "split": "thinker" }),
                        )
                        .await?;
                        Ok(Err((
                            THINKER_LOOK_ONLY.to_owned(),
                            StepStatus::Denied,
                            THINKER_BLOCKED.to_owned(),
                        )))
                    }
                    Some(why) => {
                        audit(
                            state,
                            gate::TOOL,
                            &call.name,
                            decision::DENY,
                            json!({ "hook": why }),
                        )
                        .await?;
                        Ok(Err((
                            format!("one of the user's hooks blocked this: {why}"),
                            StepStatus::Denied,
                            BLOCKED_BY_HOOK.to_owned(),
                        )))
                    }
                    None => {
                        gate_call(emitter, state, &step_id, &call.name, &effect, *mode, cancel)
                            .await
                    }
                }?;
                match allowed {
                    Ok(()) => {
                        // Keep the file as it was, so the person can rewind this turn.
                        let edited = match &effect {
                            // A PC's files aren't kept here (rewinding leaves them as they are).
                            Effect::Edit { path, .. } if emitter.records() && pc.is_none() => {
                                Some(thrall_tools::paths::resolve(workspace, path))
                            }
                            _ => None,
                        };
                        if let Some(path) = &edited {
                            crate::checkpoints::before_edit(
                                state,
                                emitter.session_id,
                                emitter.turn_id,
                                path,
                            )
                            .await;
                        }
                        // The humaniser on full checks the writing in documents Thrall saves.
                        let written = (state.config.writing.humanise == Humanise::Full
                            && pc.is_none())
                        .then(|| crate::writing::written_text(t.name(), &args, workspace))
                        .flatten();
                        let call_id = new_id(id_prefix::REQUEST);
                        let run = match pc.as_deref() {
                            None => t.run(&ctx, args),
                            Some(device) => Box::pin(run_on_pc(
                                state,
                                device,
                                &call_id,
                                emitter.session_id,
                                workspace,
                                t.name(),
                                args,
                                &ctx,
                            )),
                        };
                        tokio::pin!(run);
                        // A backstop: no tool, however stuck, holds the turn past TOOL_MAX_SECS,
                        // and Stop ends it (after a moment to stop by itself).
                        let deadline =
                            tokio::time::sleep(std::time::Duration::from_secs(TOOL_MAX_SECS));
                        tokio::pin!(deadline);
                        let mut stop = cancel.clone();
                        let mut result = loop {
                            tokio::select! {
                                result = &mut run => break result,
                                Some(text) = output_rx.recv() => {
                                    emitter.emit(Event::StepOutput { step: step_id.clone(), text }).await?;
                                }
                                () = &mut deadline => break ToolOutput::error(format!(
                                    "{} didn't finish within {} minutes, so it was stopped. Tell the person if it matters.",
                                    call.name,
                                    TOOL_MAX_SECS / 60
                                )),
                                () = stop.cancelled() => {
                                    if let Some(device) = pc.as_deref() {
                                        stop_on_pc(state, device, &call_id, emitter.session_id);
                                    }
                                    let grace = std::time::Duration::from_secs(TOOL_STOP_GRACE_SECS);
                                    break tokio::time::timeout(grace, &mut run).await.unwrap_or_else(
                                        |_| ToolOutput::error("stopped by the person before it finished"),
                                    );
                                }
                            }
                        };
                        while let Ok(text) = output_rx.try_recv() {
                            emitter
                                .emit(Event::StepOutput {
                                    step: step_id.clone(),
                                    text,
                                })
                                .await?;
                        }
                        if result.ok
                            && let Some(path) = &edited
                        {
                            crate::checkpoints::after_edit(
                                state,
                                emitter.session_id,
                                emitter.turn_id,
                                path,
                            )
                            .await;
                        }
                        if result.ok
                            && let Some(note) = written.as_deref().and_then(crate::writing::review)
                        {
                            result.content.push_str("\n\n");
                            result.content.push_str(&note);
                        }
                        if result.ok {
                            match &effect {
                                Effect::Edit { path, .. } => track.edited(path),
                                // Keeping notes (the task list, memory, sums) isn't looking.
                                _ if NOT_LOOKING.contains(&call.name.as_str()) => {}
                                _ => track.checked(),
                            }
                        }
                        if result.restart {
                            track.restart = true;
                        }
                        result
                    }
                    Err((why, s, shown)) => {
                        status = Some(s);
                        step_summary = Some(shown);
                        ToolOutput::error(why)
                    }
                }
            }
        },
    };

    // Looking at a picture (a preview of what was made) is checking it.
    if output.ok && call.name == VIEW_IMAGE {
        track.checked();
    }
    let status = status.unwrap_or(if output.ok {
        StepStatus::Ok
    } else if output.warn {
        StepStatus::Warn
    } else {
        StepStatus::Error
    });
    if tool.is_some() && status != StepStatus::Denied {
        hooks::fire(
            state,
            HookEvent::PostTool,
            Some(&call.name),
            json!({ "tool": call.name, "ok": output.ok, "summary": output.summary }),
            workspace,
        )
        .await;
    }
    if let Some(items) = output.tasks.clone() {
        emitter.emit(Event::TasksUpdated { items }).await?;
    }
    let summary = step_summary.unwrap_or_else(|| output.summary.clone());
    emitter
        .emit(Event::StepCompleted {
            step: step_id.clone(),
            status,
            summary: summary.clone(),
            diff: output.diff.clone(),
            files: output.files.clone(),
        })
        .await?;
    // The person is told about every fact saved, and can forget it.
    if let Some(fact) = &output.remembered {
        emitter
            .emit(Event::MemorySaved {
                fact: fact.id.clone(),
                topic: fact.topic.clone(),
                content: fact.content.clone(),
            })
            .await?;
    }
    if emitter.records() {
        state
            .store
            .finish_step(
                &state.user_id,
                &step_id,
                match status {
                    StepStatus::Ok => step_status::OK,
                    StepStatus::Warn => step_status::WARN,
                    StepStatus::Denied | StepStatus::Cancelled => step_status::DENIED,
                    StepStatus::Error => step_status::ERROR,
                },
                &summary,
                now_ms(),
            )
            .await?;
    }

    // The context gate: keys and tokens in tool results never reach the model.
    let (content, found) = thrall_gates::redact(&output.content);
    if found > 0 {
        audit(
            state,
            gate::CONTEXT,
            &call.name,
            decision::CHANGED,
            json!({ "secrets_removed": found }),
        )
        .await?;
    }
    Ok(content)
}

/// Why a call can't work (`Err`, for the model), or what it would do. A tool that runs on the
/// session's companion PC (`pc`) is checked there.
async fn checked(
    state: &AppState,
    session: &str,
    workspace: &Path,
    pc: Option<&str>,
    tool: &dyn thrall_tools::Tool,
    ctx: &ToolContext,
    args: &Value,
) -> Result<Effect, String> {
    let Some(device) = pc else {
        if let Some(why) = tool.precheck(ctx, args) {
            return Err(why);
        }
        tool.prepare(ctx, args).await;
        return Ok(tool.effect(ctx, args));
    };
    let call = Call::Check {
        tool: tool.name().to_owned(),
        args: args.clone(),
    };
    let answer = state
        .companions
        .call(
            device,
            &state.user_id,
            session,
            &workspace.to_string_lossy(),
            call,
            None,
        )
        .await?;
    let checked: Checked = serde_json::from_value(answer).map_err(|e| {
        format!("the PC's answer didn't make sense ({e}); is its Thrall up to date?")
    })?;
    match checked.precheck {
        Some(why) => Err(why),
        None => Ok(checked.effect),
    }
}

/// Run a call on the session's companion PC (`device`), with its live output shown as usual.
#[allow(clippy::too_many_arguments)]
async fn run_on_pc(
    state: &AppState,
    device: &str,
    id: &str,
    session: &str,
    workspace: &Path,
    tool: &str,
    args: Value,
    ctx: &ToolContext,
) -> ToolOutput {
    let call = Call::Run {
        tool: tool.to_owned(),
        args,
    };
    let answer = state
        .companions
        .call_as(
            id,
            device,
            &state.user_id,
            session,
            &workspace.to_string_lossy(),
            call,
            Some(ctx.output.clone()),
        )
        .await;
    match answer.map(serde_json::from_value::<Ran>) {
        Ok(Ok(ran)) => ran.into_output(ctx),
        Ok(Err(e)) => ToolOutput::error(format!(
            "the PC's answer didn't make sense ({e}); is its Thrall up to date?"
        )),
        Err(why) => ToolOutput::error(why),
    }
}

/// Tell the session's companion PC to stop call `id` (the person pressed Stop).
fn stop_on_pc(state: &AppState, device: &str, id: &str, session: &str) {
    let companions = state.companions.clone();
    let (device, id, session, person) = (
        device.to_owned(),
        id.to_owned(),
        session.to_owned(),
        state.user_id.clone(),
    );
    tokio::spawn(async move {
        let call = Call::Stop { call: id };
        let _ = companions
            .call(&device, &person, &session, "", call, None)
            .await;
    });
}

/// Run a `run: helper` skill: a helper gets its instructions and the task; its answer is the
/// result. `step` is the skill call's own step.
#[allow(clippy::too_many_arguments)]
fn run_skill_in_helper<'a>(
    emitter: &'a Emitter<'a>,
    state: &'a AppState,
    workspace: &'a Path,
    ctx: &'a ToolContext,
    args: &'a Value,
    mode: Mode,
    cancel: &'a Cancel,
    step: &'a str,
) -> futures::future::BoxFuture<'a, Result<ToolOutput, AgentError>> {
    // Boxed: a helper's steps come back through `run_step`.
    Box::pin(async move {
        let name = arg_str(args, "name");
        let Some(skill) = find_skill(ctx, name) else {
            return Ok(ToolOutput::error(format!("there's no skill called {name}")));
        };
        let task = arg_str(args, "task").trim();
        let task = if task.is_empty() {
            "Do what the skill is for, as fits the working folder."
        } else {
            task
        };
        let full = format!(
            "{SKILL_HELPER_TASK}\n\n{}\n\n# The task\n\n{task}",
            skill_text(&skill)
        );
        let helping = crate::workers::Helping {
            kind: None,
            step: Some(step),
        };
        let answer = crate::workers::run_helper(
            emitter, state, workspace, &full, mode, cancel, None, helping,
        )
        .await?;
        Ok(ToolOutput::ok(
            answer,
            format!("{SKILL_HELPER_DONE}: {}", skill.name),
        ))
    })
}

/// The tool gate for one call, asking the user when the mode says so. `Ok(Ok(()))` = run it;
/// `Ok(Err((reason for the model, step status, what the user sees)))` = don't.
#[allow(clippy::type_complexity)]
pub(crate) async fn gate_call(
    emitter: &Emitter<'_>,
    state: &AppState,
    step_id: &str,
    tool: &str,
    effect: &Effect,
    mode: Mode,
    cancel: &Cancel,
) -> Result<Result<(), (String, StepStatus, String)>, AgentError> {
    let scope = match effect {
        Effect::Edit { .. } => Some(EDIT_GRANT.to_owned()),
        Effect::Run { grant, .. } | Effect::Probe { grant, .. } => grant.clone(),
        Effect::Read | Effect::AlwaysAsk { .. } => None,
    };
    let granted = match &scope {
        Some(s) => {
            state
                .store
                .has_grant(&state.user_id, emitter.session_id, s)
                .await?
        }
        None => false,
    };
    let detail = json!({ "mode": mode.as_str(), "effect": effect_name(effect), "scope": scope });
    // A check-in only looks.
    if state.is_check_in(emitter.session_id) && !matches!(effect, Effect::Read) {
        audit(state, gate::TOOL, tool, decision::DENY, detail).await?;
        return Ok(Err((
            CHECK_IN_READ_ONLY.to_owned(),
            StepStatus::Denied,
            CHECK_IN_BLOCKED.to_owned(),
        )));
    }
    match tool_gate(mode, effect, granted) {
        Decision::Allow => {
            if !matches!(effect, Effect::Read) {
                audit(state, gate::TOOL, tool, decision::ALLOW, detail).await?;
            }
            Ok(Ok(()))
        }
        // Background helpers run in plan mode so nothing asks; say what's really going on.
        Decision::Deny(_) if !emitter.records() => {
            audit(state, gate::TOOL, tool, decision::DENY, detail).await?;
            Ok(Err((
                BACKGROUND_CANT_CHANGE.to_owned(),
                StepStatus::Denied,
                BACKGROUND_CANT_CHANGE.to_owned(),
            )))
        }
        Decision::Deny(why) => {
            audit(state, gate::TOOL, tool, decision::DENY, detail).await?;
            Ok(Err((why, StepStatus::Denied, BLOCKED_BY_PLAN.to_owned())))
        }
        // A background helper plans with nobody to ask: its probes stay refused.
        Decision::Ask { .. }
            if !emitter.records()
                && mode == Mode::Plan
                && matches!(effect, Effect::Probe { .. }) =>
        {
            audit(state, gate::TOOL, tool, decision::DENY, detail).await?;
            Ok(Err((
                BACKGROUND_CANT_CHANGE.to_owned(),
                StepStatus::Denied,
                BACKGROUND_CANT_CHANGE.to_owned(),
            )))
        }
        Decision::Ask { .. } if state.is_unattended(emitter.session_id) => {
            audit(state, gate::TOOL, tool, decision::DENY, detail).await?;
            Ok(Err((
                "this is a scheduled run and nobody is here to approve it. Do only what's "
                    .to_owned()
                    + "allowed without asking, and say in your reply what needs the user.",
                StepStatus::Denied,
                BLOCKED_UNATTENDED.to_owned(),
            )))
        }
        Decision::Ask { always } => {
            let (summary, risk, diff) = match effect {
                Effect::Edit { path, diff } => (format!("Edit {path}"), Risk::Low, diff.clone()),
                Effect::Run { summary, risk, .. } | Effect::Probe { summary, risk, .. } => {
                    (summary.clone(), *risk, None)
                }
                Effect::AlwaysAsk {
                    summary,
                    risk,
                    diff,
                } => (summary.clone(), *risk, diff.clone()),
                Effect::Read => (String::new(), Risk::Low, None),
            };
            let asked = ask(
                emitter, state, step_id, tool, summary, risk, diff, always, cancel,
            )
            .await?;
            match asked {
                Asked::Yes => {
                    audit(state, gate::TOOL, tool, decision::APPROVED, detail).await?;
                    Ok(Ok(()))
                }
                Asked::Always => {
                    if let Some(s) = &scope {
                        state
                            .store
                            .add_grant(
                                &state.user_id,
                                emitter.session_id,
                                &new_id(id_prefix::GRANT),
                                tool,
                                s,
                                now_ms(),
                            )
                            .await?;
                    }
                    audit(state, gate::TOOL, tool, decision::ALWAYS, detail).await?;
                    Ok(Ok(()))
                }
                Asked::No => {
                    audit(state, gate::TOOL, tool, decision::DECLINED, detail).await?;
                    Ok(Err((
                        "the user said no to this. Don't try the same thing again; ask what \
                         they'd like instead, or carry on without it."
                            .to_owned(),
                        StepStatus::Denied,
                        "You said no".to_owned(),
                    )))
                }
                Asked::Interrupted => Ok(Err((
                    "the user interrupted before answering.".to_owned(),
                    StepStatus::Cancelled,
                    "Stopped".to_owned(),
                ))),
            }
        }
    }
}

fn effect_name(effect: &Effect) -> &'static str {
    match effect {
        Effect::Read => "read",
        Effect::Edit { .. } => "edit",
        Effect::Run { .. } => "run",
        Effect::Probe { .. } => "probe",
        Effect::AlwaysAsk { .. } => "always_ask",
    }
}

/// Ask the user and wait (or stop waiting if they interrupt).
#[allow(clippy::too_many_arguments)]
async fn ask(
    emitter: &Emitter<'_>,
    state: &AppState,
    step_id: &str,
    tool: &str,
    summary: String,
    risk: Risk,
    diff: Option<thrall_protocol::Diff>,
    always: bool,
    cancel: &Cancel,
) -> Result<Asked, AgentError> {
    // One card at a time: a helper working beside another waits until that one is answered.
    let mut waiting = cancel.clone();
    let _asking = tokio::select! {
        held = emitter.asking.lock() => held,
        () = waiting.cancelled() => return Ok(Asked::Interrupted),
    };
    let request = new_id(id_prefix::REQUEST);
    let answer = state.prompts.register(&request, &state.user_id);
    let mut choices = vec![PermissionChoice::Yes];
    if always {
        choices.push(PermissionChoice::Always);
    }
    choices.push(PermissionChoice::No);
    emitter
        .emit(Event::PermissionRequest {
            request: request.clone(),
            turn: emitter.turn_id.to_owned(),
            step: step_id.to_owned(),
            tool: tool.to_owned(),
            summary,
            risk,
            diff,
            choices,
        })
        .await?;
    let mut cancel = cancel.clone();
    let (asked, answered_by) = tokio::select! {
        answer = answer => match answer {
            Ok(a) => {
                let asked = match a.reply {
                    Reply::Permission(PermissionChoice::Yes) => Asked::Yes,
                    // "always" only counts when it was offered.
                    Reply::Permission(PermissionChoice::Always) if always => Asked::Always,
                    Reply::Permission(PermissionChoice::Always) => Asked::Yes,
                    _ => Asked::No,
                };
                (asked, a.answered_by)
            }
            Err(_) => (Asked::No, String::new()),
        },
        () = cancel.cancelled() => {
            state.prompts.cancel(&request);
            (Asked::Interrupted, String::new())
        }
    };
    emitter
        .emit(Event::PermissionResolved {
            request,
            choice: match asked {
                Asked::Yes => PermissionChoice::Yes,
                Asked::Always => PermissionChoice::Always,
                Asked::No | Asked::Interrupted => PermissionChoice::No,
            },
            answered_by,
        })
        .await?;
    Ok(asked)
}

/// Plan mode: show the plan and wait for the user's decision.
async fn propose_plan(
    emitter: &Emitter<'_>,
    state: &AppState,
    args: &Value,
    mode: &mut Mode,
    cancel: &Cancel,
) -> Result<ToolOutput, AgentError> {
    let markdown = args
        .get("plan")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if markdown.trim().is_empty() {
        return Ok(ToolOutput::error("the plan is empty; write it in markdown"));
    }
    if !decide_plan(emitter, state, markdown, mode, cancel).await? {
        return Ok(ToolOutput::ok(
            "The user hasn't approved the plan. Stop here: don't start the work. Ask what \
             they'd like changed, and wait for their reply.",
            "Plan not approved",
        ));
    }
    Ok(ToolOutput::ok(
        format!(
            "The user approved the plan. Permission mode is now: {}. Carry the plan out now, \
             keeping the task list up to date.",
            mode.label()
        ),
        "Plan approved",
    ))
}

/// Show a plan, wait for the user, and on approval leave plan mode. True if approved.
pub(crate) async fn decide_plan(
    emitter: &Emitter<'_>,
    state: &AppState,
    markdown: String,
    mode: &mut Mode,
    cancel: &Cancel,
) -> Result<bool, AgentError> {
    let plan = new_id(id_prefix::REQUEST);
    let answer = state.prompts.register(&plan, &state.user_id);
    emitter
        .emit(Event::PlanProposed {
            plan: plan.clone(),
            markdown,
        })
        .await?;
    let mut cancel = cancel.clone();
    let (approve, auto_edits, answered_by) = tokio::select! {
        answer = answer => match answer {
            Ok(a) => match a.reply {
                Reply::Plan { approve, auto_edits } => (approve, auto_edits, a.answered_by),
                Reply::Permission(c) => (c != PermissionChoice::No, false, a.answered_by),
                // Not an answer to a plan.
                Reply::Question(_) => (false, false, a.answered_by),
            },
            Err(_) => (false, false, String::new()),
        },
        () = cancel.cancelled() => {
            state.prompts.cancel(&plan);
            (false, false, String::new())
        }
    };
    emitter
        .emit(Event::PlanResolved {
            plan,
            approved: approve,
            answered_by,
        })
        .await?;
    if !approve {
        emitter.note_plan_rejected();
        return Ok(false);
    }
    let next = if auto_edits {
        Mode::AutoEdits
    } else {
        Mode::Ask
    };
    state
        .store
        .set_session_mode(&state.user_id, emitter.session_id, next.as_str())
        .await?;
    *mode = next;
    emitter
        .emit(Event::SessionUpdated {
            title: None,
            mode: Some(next.as_str().to_owned()),
            mode_limit: None,
            workspace: None,
        })
        .await?;
    Ok(true)
}
