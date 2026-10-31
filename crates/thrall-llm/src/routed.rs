//! Picks the provider per model: ids starting with `chatgpt/` go to ChatGPT (the person's plan),
//! everything else (and pictures, speech) to the main provider.

use std::sync::Arc;

use futures::future::BoxFuture;
use futures::{StreamExt, stream};

use crate::constants::CHATGPT_MODEL_PREFIX;
use crate::error::LlmError;
use crate::provider::{LlmProvider, ReplyStream};
use crate::types::{
    ChatRequest, ImageReply, ImageRequest, ModelPrices, SpeechReply, SpeechRequest,
};

/// The main provider, plus ChatGPT when the person has signed in.
pub struct Routed {
    main: Arc<dyn LlmProvider>,
    chatgpt: Option<Arc<dyn LlmProvider>>,
}

impl Routed {
    /// Route between `main` and `chatgpt` (absent: not signed in).
    pub fn new(main: Arc<dyn LlmProvider>, chatgpt: Option<Arc<dyn LlmProvider>>) -> Self {
        Self { main, chatgpt }
    }

    fn is_chatgpt(model: &str) -> bool {
        model.starts_with(CHATGPT_MODEL_PREFIX)
    }
}

impl LlmProvider for Routed {
    fn name(&self) -> &str {
        self.main.name()
    }

    fn stream(&self, request: ChatRequest) -> ReplyStream {
        if !Self::is_chatgpt(&request.model) {
            return self.main.stream(request);
        }
        match &self.chatgpt {
            Some(chatgpt) => chatgpt.stream(request),
            None => stream::once(async {
                Err(LlmError::Rejected {
                    provider: "ChatGPT".to_owned(),
                    status: 0,
                    message: "Thrall isn't signed in to ChatGPT. Sign in with `/login chatgpt` \
                              (or Settings → Models), or pick another model."
                        .to_owned(),
                })
            })
            .boxed(),
        }
    }

    fn accepts_images<'a>(&'a self, model: &'a str) -> BoxFuture<'a, Option<bool>> {
        match (&self.chatgpt, Self::is_chatgpt(model)) {
            (Some(c), true) => c.accepts_images(model),
            (None, true) => Box::pin(async { None }),
            _ => self.main.accepts_images(model),
        }
    }

    fn prices<'a>(&'a self, model: &'a str) -> BoxFuture<'a, Option<ModelPrices>> {
        if Self::is_chatgpt(model) {
            // Included in the ChatGPT plan.
            return Box::pin(async {
                Some(ModelPrices {
                    prompt: 0.0,
                    completion: 0.0,
                    image_output: None,
                })
            });
        }
        self.main.prices(model)
    }

    fn credit_left(&self) -> BoxFuture<'_, Option<f64>> {
        self.main.credit_left()
    }

    fn plan_models(&self) -> BoxFuture<'_, Result<Vec<String>, LlmError>> {
        match &self.chatgpt {
            Some(c) => c.plan_models(),
            None => Box::pin(async { Ok(Vec::new()) }),
        }
    }

    fn speak(&self, request: SpeechRequest) -> BoxFuture<'_, Result<SpeechReply, LlmError>> {
        self.main.speak(request)
    }

    fn make_image(&self, request: ImageRequest) -> BoxFuture<'_, Result<ImageReply, LlmError>> {
        self.main.make_image(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatMessage, ChatRole, StreamEvent};

    struct Named(&'static str);
    impl LlmProvider for Named {
        fn name(&self) -> &str {
            self.0
        }
        fn stream(&self, _request: ChatRequest) -> ReplyStream {
            let name = self.0.to_owned();
            stream::once(async move { Ok(StreamEvent::Text(name)) }).boxed()
        }
    }

    async fn first(r: &Routed, model: &str) -> Result<StreamEvent, LlmError> {
        let request = ChatRequest {
            model: model.into(),
            messages: vec![ChatMessage::new(ChatRole::User, "hi")],
            max_tokens: 10,
            tools: Vec::new(),
        };
        r.stream(request).next().await.unwrap()
    }

    #[tokio::test]
    async fn chatgpt_models_go_to_chatgpt_and_the_rest_to_main() {
        let r = Routed::new(Arc::new(Named("main")), Some(Arc::new(Named("gpt"))));
        assert_eq!(
            first(&r, "chatgpt/gpt-5").await.unwrap(),
            StreamEvent::Text("gpt".into())
        );
        assert_eq!(
            first(&r, "openai/gpt-5").await.unwrap(),
            StreamEvent::Text("main".into())
        );
        let signed_out = Routed::new(Arc::new(Named("main")), None);
        let e = first(&signed_out, "chatgpt/gpt-5").await.unwrap_err();
        assert!(e.to_string().contains("/login chatgpt"));
    }
}
