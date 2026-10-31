//! The provider trait every AI provider implements.

use futures::future::BoxFuture;
use futures::stream::BoxStream;

use crate::error::LlmError;
use crate::types::{
    ChatRequest, ImageReply, ImageRequest, ModelPrices, SpeechReply, SpeechRequest, StreamEvent,
};

/// Vectors for some texts, and what they cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Embeddings {
    /// One vector per input, in order.
    pub vectors: Vec<Vec<f32>>,
    /// Tokens used.
    pub tokens: u64,
    /// Cost in US dollars, when reported.
    pub cost_usd: Option<f64>,
}

/// Turns text into vectors for searching by meaning.
pub trait Embedder: Send + Sync {
    /// Embed `inputs` with `model`.
    fn embed<'a>(
        &'a self,
        model: &'a str,
        inputs: Vec<String>,
    ) -> BoxFuture<'a, Result<Embeddings, LlmError>>;
}

/// The stream a provider returns for one reply.
pub type ReplyStream = BoxStream<'static, Result<StreamEvent, LlmError>>;

/// An AI provider. Implementations live one per file and are chosen explicitly at startup.
pub trait LlmProvider: Send + Sync {
    /// Display name for messages ("OpenRouter").
    fn name(&self) -> &str;

    /// Start one streamed reply. The request is sent when the stream is first polled. Temporary
    /// failures before any text arrives are retried inside the stream.
    fn stream(&self, request: ChatRequest) -> ReplyStream;

    /// Whether `model` accepts pictures. `None` when the provider can't tell (then pictures are
    /// sent and the provider says if it can't take them).
    fn accepts_images<'a>(&'a self, _model: &'a str) -> BoxFuture<'a, Option<bool>> {
        Box::pin(async { None })
    }

    /// `model`'s prices, when the provider publishes them.
    fn prices<'a>(&'a self, _model: &'a str) -> BoxFuture<'a, Option<ModelPrices>> {
        Box::pin(async { None })
    }

    /// Money left on the account in US dollars, when the provider can say (prepaid credit).
    fn credit_left(&self) -> BoxFuture<'_, Option<f64>> {
        Box::pin(async { None })
    }

    /// Chat models the person's account offers, as model ids Thrall accepts, when the provider can
    /// list them (ChatGPT plans differ by account). Empty when it can't.
    fn plan_models(&self) -> BoxFuture<'_, Result<Vec<String>, LlmError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Read text aloud.
    fn speak(&self, request: SpeechRequest) -> BoxFuture<'_, Result<SpeechReply, LlmError>> {
        let provider = self.name().to_owned();
        Box::pin(async move {
            Err(LlmError::Rejected {
                provider,
                status: 0,
                message: format!("{} can't speak", request.model),
            })
        })
    }

    /// Make or edit pictures (one request, not streamed).
    fn make_image(&self, request: ImageRequest) -> BoxFuture<'_, Result<ImageReply, LlmError>> {
        let provider = self.name().to_owned();
        Box::pin(async move {
            Err(LlmError::Rejected {
                provider,
                status: 0,
                message: format!("{} can't make pictures", request.model),
            })
        })
    }
}
