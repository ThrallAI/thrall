//! Compaction: when a session's context grows past the limit, the oldest part is summarised by the
//! model and the summary takes its place. History (the event log) is untouched; only what the
//! model is sent changes. Where to cut is decided by `thrall_memory::compaction::plan`.
//!
//! After a summary, lasting facts the person stated in that stretch are saved to long-term memory
//! (when `memory.save_facts` is on). Only the person's own messages are read for them, never
//! Thrall's replies or tool results, so text from a web page or file can't plant a "fact".

use futures::StreamExt;
use thrall_core::constants::{id_prefix, usage_role};
use thrall_core::ids::new_id;
use thrall_llm::{ChatMessage, ChatRequest, ChatRole, StreamEvent};
use thrall_memory::compaction::{Sized, estimate_tokens, plan, scaled};
use thrall_protocol::Usage;
use thrall_store::NewMessage;

use crate::constants::{
    COMPACT_PROMPT, COMPACT_SOURCE_MAX_CHARS, COMPACT_SUMMARY_MAX_TOKENS, FACTS_MAX,
    FACTS_MAX_TOKENS, FACTS_PROMPT, SUMMARY_PREFIX, SUMMARY_PREFIX_OLD, role,
};
use crate::error::AgentError;
use crate::state::AppState;
use crate::turn::{CallBy, StoredContent, record_usage};

/// What compaction did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compacted {
    /// Messages summarised.
    pub messages: usize,
}

/// What the check before a model call found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// What compaction did, if it ran.
    pub compacted: Option<Compacted>,
    /// The context's estimated size as it stood (before any correction, `extra_tokens`
    /// included), to measure against what the provider counts.
    pub estimated_tokens: usize,
}

/// Summarise the oldest part of `session_id`'s context if it's over the limit (or if `force`).
/// `extra_tokens` is the estimated size of what goes with it (system prompt, tools). The estimate
/// is corrected by the session's measured token scale (`AppState::token_scale`).
pub async fn compact(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    extra_tokens: usize,
    force: bool,
) -> Result<Checked, AgentError> {
    let stored = state
        .store
        .context_messages(&state.user_id, session_id)
        .await?;
    let scale = state.token_scale(session_id);
    let mut estimated_tokens = extra_tokens;
    let sizes: Vec<Sized> = stored
        .iter()
        .map(|m| {
            let tokens = estimate_tokens(&m.content);
            estimated_tokens += tokens;
            Sized {
                is_user: m.role == role::USER,
                tokens: scaled(tokens, scale),
                in_task: m.turn_id.as_deref() == Some(turn_id),
            }
        })
        .collect();
    let limit = if force {
        0
    } else {
        state.config.memory.context_limit_tokens
    };
    let compacted = match plan(&sizes, scaled(extra_tokens, scale), limit) {
        Some(cut) => {
            let compacted = summarise(state, session_id, turn_id, &stored, cut).await?;
            if compacted.is_some() {
                tracing::info!(
                    session = session_id,
                    messages = cut,
                    estimated_tokens,
                    scale_percent = scale,
                    "compacted"
                );
            }
            compacted
        }
        None => None,
    };
    Ok(Checked {
        compacted,
        estimated_tokens,
    })
}

/// Summarise `stored[..cut]` and put the summary in its place.
async fn summarise(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    stored: &[thrall_store::StoredMessage],
    cut: usize,
) -> Result<Option<Compacted>, AgentError> {
    // The older part as plain text for the summary; the person's own words alone for facts.
    let mut transcript = String::new();
    let mut said = String::new();
    for m in &stored[..cut] {
        let text = serde_json::from_str::<StoredContent>(&m.content)
            .map(|c| c.text)
            .unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        let who = match m.role.as_str() {
            role::USER if is_summary(&text) => "Thrall's earlier notes",
            role::USER => "User",
            role::ASSISTANT => "Thrall",
            _ => "Tool result",
        };
        transcript.push_str(&format!("{who}: {text}\n\n"));
        if m.role == role::USER && !is_summary(&text) {
            said.push_str(&format!("- {text}\n"));
        }
    }
    let transcript: String = transcript.chars().take(COMPACT_SOURCE_MAX_CHARS).collect();
    let request = ChatRequest {
        model: state.model(),
        messages: vec![
            ChatMessage::new(ChatRole::System, COMPACT_PROMPT),
            ChatMessage::new(ChatRole::User, transcript),
        ],
        max_tokens: COMPACT_SUMMARY_MAX_TOKENS,
        tools: Vec::new(),
        thinking: None,
    };
    let mut summary = String::new();
    let mut usage = Usage::default();
    let mut stream = state.llm.stream(request);
    while let Some(item) = stream.next().await {
        match item {
            Ok(StreamEvent::Text(t)) => summary.push_str(&t),
            Ok(StreamEvent::Usage(u)) => {
                usage = Usage {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cost_usd: u.cost_usd,
                }
            }
            Ok(_) => {}
            Err(e) => {
                // Nothing changes if the summary can't be made; the next turn tries again.
                tracing::warn!(%e, "compaction summary failed");
                return Ok(None);
            }
        }
    }
    let by = CallBy {
        model: state.model(),
        role: usage_role::CHAT,
        cached_tokens: 0,
    };
    record_usage(state, session_id, turn_id, usage, by).await?;
    if summary.trim().is_empty() {
        return Ok(None);
    }

    // The summary sits just before the first message kept, so the order stays right.
    let at = stored[cut].created_at - 1;
    state
        .store
        .add_message(NewMessage {
            id: new_id(id_prefix::MESSAGE),
            user_id: state.user_id.clone(),
            session_id: session_id.to_owned(),
            turn_id: None,
            role: role::USER.to_owned(),
            content: serde_json::to_string(&StoredContent {
                text: format!("{SUMMARY_PREFIX}\n{}", summary.trim()),
                tool_calls: Vec::new(),
                tool_call_id: None,
                files: Vec::new(),
            })?,
            tokens: 0,
            at,
        })
        .await?;
    let ids: Vec<String> = stored[..cut].iter().map(|m| m.id.clone()).collect();
    state
        .store
        .compact_messages(&state.user_id, session_id, &ids)
        .await?;
    if state.config.memory.save_facts {
        remember_facts(state, session_id, turn_id, &said).await?;
    }
    Ok(Some(Compacted { messages: cut }))
}

/// Whether a stored user-role message is a compaction summary rather than the person's words.
pub fn is_summary(text: &str) -> bool {
    text.starts_with(SUMMARY_PREFIX) || text.starts_with(SUMMARY_PREFIX_OLD)
}

/// A fact the model drew from the person's messages.
#[derive(serde::Deserialize)]
struct Drawn {
    topic: String,
    content: String,
}

/// The facts in the model's answer (a JSON list, possibly wrapped in a code fence), at most
/// [`FACTS_MAX`], blank ones left out.
fn drawn_facts(answer: &str) -> Vec<Drawn> {
    let start = answer.find('[');
    let end = answer.rfind(']');
    let Some(json) = start
        .zip(end)
        .filter(|(s, e)| s < e)
        .map(|(s, e)| &answer[s..=e])
    else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<Drawn>>(json)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| !f.topic.trim().is_empty() && !f.content.trim().is_empty())
        .take(FACTS_MAX)
        .collect()
}

/// Save the lasting facts in what the person said (one short model call). Failures are only
/// logged: compaction has already done its job.
async fn remember_facts(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    said: &str,
) -> Result<(), AgentError> {
    if said.trim().is_empty() {
        return Ok(());
    }
    let source: String = said.chars().take(COMPACT_SOURCE_MAX_CHARS).collect();
    let request = ChatRequest {
        model: state.model(),
        messages: vec![
            ChatMessage::new(ChatRole::System, FACTS_PROMPT),
            ChatMessage::new(ChatRole::User, source),
        ],
        max_tokens: FACTS_MAX_TOKENS,
        tools: Vec::new(),
        thinking: None,
    };
    let mut answer = String::new();
    let mut usage = Usage::default();
    let mut stream = state.llm.stream(request);
    while let Some(item) = stream.next().await {
        match item {
            Ok(StreamEvent::Text(t)) => answer.push_str(&t),
            Ok(StreamEvent::Usage(u)) => {
                usage = Usage {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cost_usd: u.cost_usd,
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(%e, "couldn't draw facts from a summary");
                return Ok(());
            }
        }
    }
    let by = CallBy {
        model: state.model(),
        role: usage_role::CHAT,
        cached_tokens: 0,
    };
    record_usage(state, session_id, turn_id, usage, by).await?;
    for fact in drawn_facts(&answer) {
        if let Err(e) = state
            .memory
            .save_fact(&state.user_id, fact.topic.trim(), fact.content.trim())
            .await
        {
            tracing::warn!(%e, "couldn't save a fact drawn from a summary");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facts_are_read_from_a_json_list_even_in_a_fence() {
        let got = drawn_facts(
            "```json\n[{\"topic\": \"editor\", \"content\": \"Uses VS Code\"}, \
             {\"topic\": \" \", \"content\": \"x\"}]\n```",
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].content, "Uses VS Code");
        assert!(drawn_facts("No lasting facts.").is_empty());
        assert!(drawn_facts("[]").is_empty());
    }
}
