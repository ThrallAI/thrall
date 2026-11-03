//! Provider-neutral request and stream types. Every provider turns its own wire format into these.

use serde::{Deserialize, Serialize};

/// Who a chat message is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    /// Instructions to the model.
    System,
    /// The person.
    User,
    /// The model.
    Assistant,
    /// The result of a tool call the model made.
    Tool,
}

/// One message in the conversation sent to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// Who it's from.
    pub role: ChatRole,
    /// Its text (may be empty when an assistant message only calls tools).
    pub content: String,
    /// Tools the assistant asked to run (assistant messages only).
    pub tool_calls: Vec<ToolCall>,
    /// Which call this answers (tool messages only).
    pub tool_call_id: Option<String>,
    /// Pictures sent with the text (user messages, for models that can see).
    pub images: Vec<ImagePart>,
    /// Documents sent with the text (PDFs, for models that read files).
    pub documents: Vec<DocumentPart>,
    /// Audio sent with the text (for models that listen).
    pub audio: Vec<AudioPart>,
}

/// A recording in a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioPart {
    /// Its format: `mp3`, `wav`, `ogg`, `flac`, `m4a`, `aac` or `webm`.
    pub format: String,
    /// The file's bytes, base64-encoded.
    pub base64: String,
}

/// A document in a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentPart {
    /// File name.
    pub name: String,
    /// Its type: `application/pdf`.
    pub mime: String,
    /// The file's bytes, base64-encoded.
    pub base64: String,
}

/// A picture in a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePart {
    /// Its type: `image/png`, `image/jpeg`, `image/webp` or `image/gif`.
    pub mime: String,
    /// The file's bytes, base64-encoded.
    pub base64: String,
}

impl ChatMessage {
    /// A message with the given role and text.
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            images: Vec::new(),
            documents: Vec::new(),
            audio: Vec::new(),
        }
    }

    /// The result of the tool call `call_id`.
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
            images: Vec::new(),
            documents: Vec::new(),
            audio: Vec::new(),
        }
    }
}

/// A tool the model asked to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    /// The provider's id for the call; the result must quote it.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Arguments as a JSON string, exactly as the model wrote them (may be invalid JSON).
    pub arguments: String,
}

/// A tool the model may call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// Name the model uses.
    pub name: String,
    /// What it does and when to use it.
    pub description: String,
    /// JSON Schema for the arguments.
    pub parameters: serde_json::Value,
}

/// A request for one streamed reply.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    /// Model id as the provider names it.
    pub model: String,
    /// The conversation so far, oldest first.
    pub messages: Vec<ChatMessage>,
    /// Most tokens the model may write.
    pub max_tokens: u32,
    /// Tools the model may call (empty = none).
    pub tools: Vec<ToolSpec>,
    /// How hard a model that thinks before replying should think ("low", "medium", "high"), or
    /// `None` to leave it to the model. Its thinking counts toward `max_tokens`. Only OpenRouter
    /// is sent it; it also turns thinking on for a model that can think but doesn't by default.
    pub thinking: Option<&'static str>,
}

/// A piece of a tool call as it streams in. Pieces with the same `index` belong to one call: the
/// first usually carries `id` and `name`, the rest add to `arguments`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallDelta {
    /// Which call (position in the reply).
    pub index: u32,
    /// Call id, when this piece has it.
    pub id: Option<String>,
    /// Tool name, when this piece has it.
    pub name: Option<String>,
    /// More argument text.
    pub arguments: String,
}

/// What a provider's stream yields.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// More reply text.
    Text(String),
    /// More thinking text (models that expose it).
    Reasoning(String),
    /// Part of a tool call.
    ToolCall(ToolCallDelta),
    /// Token counts and cost, usually once near the end.
    Usage(TokenUsage),
    /// The model stopped; the reason as the provider gave it (`stop`, `tool_calls`, `length`, …).
    Finished(Option<String>),
}

/// Tokens and cost for one model call.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TokenUsage {
    /// Tokens sent.
    pub input_tokens: u64,
    /// Tokens written.
    pub output_tokens: u64,
    /// How many of the tokens sent the provider had already seen (cached, charged less); 0 when
    /// it doesn't say.
    pub cached_tokens: u64,
    /// Cost in US dollars, if the provider reports it.
    pub cost_usd: Option<f64>,
}

/// Builds whole tool calls from streamed pieces.
#[derive(Debug, Default)]
pub struct ToolCallAssembler {
    calls: Vec<(u32, ToolCall)>,
}

impl ToolCallAssembler {
    /// Add one streamed piece.
    pub fn push(&mut self, delta: ToolCallDelta) {
        let slot = match self.calls.iter().position(|(i, _)| *i == delta.index) {
            Some(pos) => pos,
            None => {
                self.calls.push((
                    delta.index,
                    ToolCall {
                        id: String::new(),
                        name: String::new(),
                        arguments: String::new(),
                    },
                ));
                self.calls.len() - 1
            }
        };
        let call = &mut self.calls[slot].1;
        if let Some(id) = delta.id.filter(|s| !s.is_empty()) {
            call.id = id;
        }
        if let Some(name) = delta.name.filter(|s| !s.is_empty()) {
            call.name.push_str(&name);
        }
        call.arguments.push_str(&delta.arguments);
    }

    /// Whether any call has started.
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// The finished calls in order. Calls the provider sent without an id get one, so results can
    /// always be matched.
    pub fn finish(mut self) -> Vec<ToolCall> {
        self.calls.sort_by_key(|(i, _)| *i);
        self.calls
            .into_iter()
            .map(|(i, mut call)| {
                if call.id.is_empty() {
                    call.id = format!("call_{i}");
                }
                call
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_calls_from_pieces() {
        let mut a = ToolCallAssembler::default();
        a.push(ToolCallDelta {
            index: 0,
            id: Some("c1".into()),
            name: Some("read_file".into()),
            arguments: "{\"pa".into(),
        });
        a.push(ToolCallDelta {
            index: 1,
            id: None,
            name: Some("list_dir".into()),
            arguments: "{}".into(),
        });
        a.push(ToolCallDelta {
            index: 0,
            id: None,
            name: None,
            arguments: "th\":\"a\"}".into(),
        });
        let calls = a.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments, "{\"path\":\"a\"}");
        assert_eq!(calls[1].id, "call_1");
    }
}

/// A request to make (or edit) pictures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRequest {
    /// Model id; it must output pictures.
    pub model: String,
    /// What to make, or how to change the pictures given.
    pub prompt: String,
    /// Pictures to start from (empty: make a new one).
    pub images: Vec<ImagePart>,
    /// Shape, like `1:1` or `16:9`, when the user asked for one.
    pub aspect_ratio: Option<String>,
}

/// Pictures a model made, with any words it added and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageReply {
    /// The pictures.
    pub images: Vec<ImagePart>,
    /// Text the model wrote alongside.
    pub text: String,
    /// Tokens and cost.
    pub usage: TokenUsage,
}

/// A model's prices in US dollars per token, from the provider's model list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPrices {
    /// Per input token.
    pub prompt: f64,
    /// Per output text token.
    pub completion: f64,
    /// Per output picture token, for models that make pictures.
    pub image_output: Option<f64>,
}

/// A request to read text aloud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechRequest {
    /// Model id; it must output audio.
    pub model: String,
    /// What to say.
    pub text: String,
    /// The voice, like `alloy` or `nova`.
    pub voice: String,
}

/// Speech a model made: a WAV file, and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeechReply {
    /// The audio as a complete WAV file.
    pub wav: Vec<u8>,
    /// Tokens and cost.
    pub usage: TokenUsage,
}
