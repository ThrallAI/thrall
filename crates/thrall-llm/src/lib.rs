//! AI providers (OpenRouter, and ChatGPT on the person's own plan): streaming, retries and cost
//! tracking.

pub mod chatgpt;
pub mod constants;
pub mod direct;
pub mod elevenlabs;
pub mod error;
pub mod fal;
pub mod openrouter;
pub mod provider;
pub mod routed;
pub mod types;

pub use chatgpt::ChatGpt;
pub use direct::{DirectEmbedder, DirectSupplier};
pub use elevenlabs::ElevenLabs;
pub use error::LlmError;
pub use openrouter::OpenRouter;
pub use provider::{Embedder, Embeddings, LlmProvider, ReplyStream};
pub use routed::Routed;
pub use types::{
    AudioPart, ChatMessage, ChatRequest, ChatRole, DocumentPart, ImagePart, ImageReply,
    ImageRequest, ModelPrices, SpeechReply, SpeechRequest, StreamEvent, TokenUsage, ToolCall,
    ToolCallAssembler, ToolCallDelta, ToolSpec,
};
