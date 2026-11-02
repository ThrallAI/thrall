//! Picks the provider per model: ids starting with `chatgpt/` go to ChatGPT (the person's plan),
//! `local/` to a model server on this computer or network (Ollama, LM Studio; the prefix is
//! dropped and nothing is charged), everything else (and pictures, speech) to the main provider.

use std::sync::Arc;

use futures::future::BoxFuture;
use futures::{StreamExt, stream};

use crate::constants::{CHATGPT_MODEL_PREFIX, LOCAL_MODEL_PREFIX};
use crate::error::LlmError;
use crate::provider::{LlmProvider, ReplyStream};
use crate::types::{
    ChatRequest, ImageReply, ImageRequest, ModelPrices, SpeechReply, SpeechRequest,
};

/// The main provider, plus ChatGPT when the person has signed in.
pub struct Routed {
    main: Arc<dyn LlmProvider>,
    chatgpt: Option<Arc<dyn LlmProvider>>,
    local: Option<Arc<dyn LlmProvider>>,
}

impl Routed {
    /// Route between `main` and `chatgpt` (absent: not signed in).
    pub fn new(main: Arc<dyn LlmProvider>, chatgpt: Option<Arc<dyn LlmProvider>>) -> Self {
        Self {
            main,
            chatgpt,
            local: None,
        }
    }

    /// Also send `local/` models to `local` (an OpenAI-compatible server).
    pub fn with_local(mut self, local: Arc<dyn LlmProvider>) -> Self {
        self.local = Some(local);
        self
    }

    fn local_model(model: &str) -> Option<&str> {
        model.strip_prefix(LOCAL_MODEL_PREFIX)
    }

    fn is_chatgpt(model: &str) -> bool {
        model.starts_with(CHATGPT_MODEL_PREFIX)
    }
}

impl LlmProvider for Routed {
    fn name(&self) -> &str {
        self.main.name()
    }

    fn stream(&self, mut request: ChatRequest) -> ReplyStream {
        if let Some(model) = Self::local_model(&request.model) {
            request.model = model.to_owned();
            return match &self.local {
                Some(local) => local.stream(request),
                None => stream::once(async {
                    Err(LlmError::Rejected {
                        provider: "local".to_owned(),
                        status: 0,
                        message: "No local model server is set up. Set [provider] local_base_url \
                                  (Ollama: http://localhost:11434/v1), or pick another model."
                            .to_owned(),
                    })
                })
                .boxed(),
            };
        }
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
        if let Some(m) = Self::local_model(model) {
            return match &self.local {
                Some(l) => l.accepts_images(m),
                None => Box::pin(async { None }),
            };
        }
        match (&self.chatgpt, Self::is_chatgpt(model)) {
            (Some(c), true) => c.accepts_images(model),
            (None, true) => Box::pin(async { None }),
            _ => self.main.accepts_images(model),
        }
    }

    fn prices<'a>(&'a self, model: &'a str) -> BoxFuture<'a, Option<ModelPrices>> {
        if Self::is_chatgpt(model) || Self::local_model(model).is_some() {
            // Included in the ChatGPT plan, or run locally.
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

    /// Answers with the model it was asked for.
    struct Echo;
    impl LlmProvider for Echo {
        fn name(&self) -> &str {
            "local"
        }
        fn stream(&self, request: ChatRequest) -> ReplyStream {
            stream::once(async move { Ok(StreamEvent::Text(request.model)) }).boxed()
        }
    }

    #[tokio::test]
    async fn local_models_go_to_the_local_server_without_their_prefix_and_free() {
        let r = Routed::new(Arc::new(Named("main")), None).with_local(Arc::new(Echo));
        assert_eq!(
            first(&r, "local/llama3.1").await.unwrap(),
            StreamEvent::Text("llama3.1".into())
        );
        assert_eq!(
            first(&r, "openai/gpt-5").await.unwrap(),
            StreamEvent::Text("main".into())
        );
        let free = r.prices("local/llama3.1").await.unwrap();
        assert_eq!((free.prompt, free.completion), (0.0, 0.0));
        let none = Routed::new(Arc::new(Named("main")), None);
        let e = first(&none, "local/llama3.1").await.unwrap_err();
        assert!(e.to_string().contains("local_base_url"));
    }
}
