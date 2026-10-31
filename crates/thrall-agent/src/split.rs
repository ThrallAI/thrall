//! Split mode (experimental, `[split]` in config.toml). The Thinker runs the turn but can only
//! look; it writes a brief and hands the work to the Doer (a helper on another model, with every
//! tool), checks the report, and may send it back once. The Doer can ask the Thinker when stuck.
//! Every model call's usage row says which of them made it, so the two can be compared.

use std::path::Path;

use futures::StreamExt;
use serde_json::{Value, json};
use thrall_core::constants::{id_prefix, usage_role};
use thrall_core::ids::{new_id, now_ms};
use thrall_gates::Mode;
use thrall_llm::{ChatMessage, ChatRequest, ChatRole, StreamEvent, ToolCall, ToolSpec};
use thrall_protocol::{Event, StepStatus, Usage};
use thrall_store::NewStep;
use thrall_tools::Cancel;
use thrall_tools::tool::arg_str;

use crate::checks::Track;
use crate::constants::{
    ASK_THINKER, ASK_THINKER_DESCRIPTION, ASK_THINKER_TITLE, HAND_OFF, HAND_OFF_DESCRIPTION,
    HAND_OFF_TITLE, SPLIT_ASK_MAX_TOKENS, SPLIT_ASK_PROMPT, SPLIT_MAX_ASKS, SPLIT_MAX_HANDOFFS,
    SPLIT_NO_MORE_ASKS, SPLIT_NO_MORE_HANDOFFS, SPLIT_THINKER_NOTE, THINKER_CANT, step_status,
};
use crate::error::AgentError;
use crate::state::AppState;
use crate::turn::{CallBy, Emitter};

/// Split mode for one turn.
#[derive(Debug, Clone)]
pub(crate) struct Split {
    /// The model that plans, briefs and checks.
    pub(crate) thinker: String,
    /// The model that does the work.
    pub(crate) doer: String,
    /// Hand-offs so far this turn.
    handoffs: usize,
}

/// Split mode's Doer while it works on one brief.
#[derive(Debug)]
pub(crate) struct Doer {
    /// The Doer's model.
    pub(crate) model: String,
    /// The Thinker's model (it answers questions).
    thinker: String,
    /// The brief, which the Thinker sees again when asked.
    brief: String,
    /// Questions asked so far.
    asks: usize,
    /// Files the Doer changed (as shown to the user).
    pub(crate) edited: Vec<String>,
}

/// Split mode for a turn in `session_id`, when it's switched on. Check-ins never split.
pub(crate) fn for_turn(state: &AppState, session_id: &str) -> Option<Split> {
    let config = &state.config.split;
    if !config.enabled || state.is_check_in(session_id) {
        return None;
    }
    let or_chat = |m: &str| {
        let m = m.trim();
        if m.is_empty() {
            state.model_for(session_id)
        } else {
            m.to_owned()
        }
    };
    Some(Split {
        thinker: or_chat(&config.thinker_model),
        doer: or_chat(&config.doer_model),
        handoffs: 0,
    })
}

/// The Thinker's tools: everything that only looks, plus `hand_off`.
pub(crate) fn thinker_tools(all: Vec<ToolSpec>) -> Vec<ToolSpec> {
    let mut tools: Vec<ToolSpec> = all
        .into_iter()
        .filter(|t| !THINKER_CANT.contains(&t.name.as_str()))
        .collect();
    tools.push(ToolSpec {
        name: HAND_OFF.to_owned(),
        description: HAND_OFF_DESCRIPTION.to_owned(),
        parameters: json!({
            "type": "object",
            "properties": {
                "brief": { "type": "string", "description": "Everything the Doer needs to do the work." }
            },
            "required": ["brief"]
        }),
    });
    tools
}

/// The Doer's extra tool.
pub(crate) fn ask_thinker_spec() -> ToolSpec {
    ToolSpec {
        name: ASK_THINKER.to_owned(),
        description: ASK_THINKER_DESCRIPTION.to_owned(),
        parameters: json!({
            "type": "object",
            "properties": {
                "question": { "type": "string", "description": "What you need to know, what you tried, and what happened." }
            },
            "required": ["question"]
        }),
    }
}

/// Tell the Thinker its part (the system prompt is the first message).
pub(crate) fn brief_thinker(messages: &mut [ChatMessage]) {
    if let Some(system) = messages.first_mut()
        && system.role == ChatRole::System
    {
        system.content.push_str(SPLIT_THINKER_NOTE);
    }
}

/// The Thinker's `hand_off`: the Doer works on the brief and its report comes back. Files it
/// changed count as the turn's changes, so Thrall's own checks ask the Thinker to look at them.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn hand_off(
    emitter: &Emitter<'_>,
    state: &AppState,
    workspace: &Path,
    split: &mut Split,
    call: &ToolCall,
    mode: Mode,
    track: &mut Track,
    cancel: &Cancel,
) -> Result<String, AgentError> {
    if split.handoffs >= SPLIT_MAX_HANDOFFS {
        return Ok(format!("Error: {SPLIT_NO_MORE_HANDOFFS}"));
    }
    let brief = arg_str(&parse(&call.arguments), "brief").trim().to_owned();
    if brief.is_empty() {
        return Ok("Error: write the brief for the Doer.".to_owned());
    }
    split.handoffs += 1;
    let step = start_step(emitter, state, HAND_OFF, HAND_OFF_TITLE, &call.arguments).await?;
    let mut doer = Doer {
        model: split.doer.clone(),
        thinker: split.thinker.clone(),
        brief: brief.clone(),
        asks: 0,
        edited: Vec::new(),
    };
    let report = crate::workers::run_helper(
        emitter,
        state,
        workspace,
        &brief,
        mode,
        cancel,
        Some(&mut doer),
    )
    .await?;
    for path in &doer.edited {
        track.edited(path);
    }
    finish_step(
        emitter,
        state,
        &step,
        StepStatus::Ok,
        &format!("The Doer finished ({})", doer.model),
    )
    .await?;
    Ok(report)
}

/// The Doer's `ask_thinker`: one question to the Thinker, without tools.
pub(crate) async fn ask(
    emitter: &Emitter<'_>,
    state: &AppState,
    doer: &mut Doer,
    arguments: &str,
) -> Result<String, AgentError> {
    if doer.asks >= SPLIT_MAX_ASKS {
        return Ok(format!("Error: {SPLIT_NO_MORE_ASKS}"));
    }
    let question = arg_str(&parse(arguments), "question").trim().to_owned();
    if question.is_empty() {
        return Ok("Error: say what you want to ask.".to_owned());
    }
    doer.asks += 1;
    let step = start_step(emitter, state, ASK_THINKER, ASK_THINKER_TITLE, arguments).await?;
    let request = ChatRequest {
        model: doer.thinker.clone(),
        messages: vec![
            ChatMessage::new(ChatRole::System, SPLIT_ASK_PROMPT),
            ChatMessage::new(
                ChatRole::User,
                format!(
                    "# Your brief\n\n{}\n\n# The Doer asks\n\n{question}",
                    doer.brief
                ),
            ),
        ],
        max_tokens: SPLIT_ASK_MAX_TOKENS,
        tools: Vec::new(),
    };
    let mut answer = String::new();
    let mut usage = Usage::default();
    let mut cached_tokens = 0;
    let mut failed = None;
    let mut stream = state.llm.stream(request);
    while let Some(item) = stream.next().await {
        match item {
            Ok(StreamEvent::Text(t)) => answer.push_str(&t),
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
        model: doer.thinker.clone(),
        role: usage_role::ASK,
        cached_tokens,
    };
    emitter.record_side_usage_by(state, usage, by).await?;
    let (status, result) = match failed {
        Some(e) => (
            StepStatus::Error,
            format!("Error: the Thinker couldn't answer ({e}). Decide yourself."),
        ),
        None if answer.trim().is_empty() => (
            StepStatus::Error,
            "Error: the Thinker gave no answer. Decide yourself.".to_owned(),
        ),
        None => (StepStatus::Ok, thrall_gates::redact(&answer).0),
    };
    finish_step(emitter, state, &step, status, ASK_THINKER_TITLE).await?;
    Ok(result)
}

fn parse(arguments: &str) -> Value {
    serde_json::from_str(arguments).unwrap_or(Value::Null)
}

/// Show (and record) a split-mode step, as tool steps are.
async fn start_step(
    emitter: &Emitter<'_>,
    state: &AppState,
    tool: &str,
    title: &str,
    arguments: &str,
) -> Result<String, AgentError> {
    let step = new_id(id_prefix::STEP);
    emitter
        .emit(Event::StepStarted {
            turn: emitter.turn_id.to_owned(),
            step: step.clone(),
            tool: tool.to_owned(),
            title: title.to_owned(),
        })
        .await?;
    if emitter.records() {
        state
            .store
            .start_step(NewStep {
                id: step.clone(),
                user_id: state.user_id.clone(),
                turn_id: emitter.turn_id.to_owned(),
                tool: tool.to_owned(),
                title: title.to_owned(),
                args: thrall_gates::redact(arguments).0,
                at: now_ms(),
            })
            .await?;
    }
    Ok(step)
}

async fn finish_step(
    emitter: &Emitter<'_>,
    state: &AppState,
    step: &str,
    status: StepStatus,
    summary: &str,
) -> Result<(), AgentError> {
    emitter
        .emit(Event::StepCompleted {
            step: step.to_owned(),
            status,
            summary: summary.to_owned(),
            diff: None,
            files: Vec::new(),
        })
        .await?;
    if emitter.records() {
        state
            .store
            .finish_step(
                &state.user_id,
                step,
                if status == StepStatus::Ok {
                    step_status::OK
                } else {
                    step_status::ERROR
                },
                summary,
                now_ms(),
            )
            .await?;
    }
    Ok(())
}
