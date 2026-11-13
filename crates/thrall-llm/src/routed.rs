//! Picks the provider per model: ids starting with `chatgpt/` go to ChatGPT (the person's plan),
//! `local/` to a model server on this computer or network (Ollama, LM Studio; the prefix is
//! dropped and nothing is charged), `direct/<supplier>/` straight to that company with the
//! person's own key for it, `program/` to programs on this computer (whisper.cpp, Piper, a
//! Stable Diffusion web UI; free), everything else to the main provider (OpenRouter).

use std::collections::HashMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use futures::{StreamExt, stream};

use crate::constants::{CHATGPT_MODEL_PREFIX, LOCAL_MODEL_PREFIX, PROGRAM_MODEL_PREFIX};
use crate::direct::{self, DirectSupplier};
use crate::error::LlmError;
use crate::provider::{LlmProvider, ReplyStream};
use crate::types::{
    ChatRequest, ImageReply, ImageRequest, ModelPrices, SpeechReply, SpeechRequest, StreamEvent,
    TokenUsage,
};

/// The main provider, plus ChatGPT when the person has signed in, a local server, and the
/// companies the person has a key for.
pub struct Routed {
    main: Arc<dyn LlmProvider>,
    chatgpt: Option<Arc<dyn LlmProvider>>,
    local: Option<Arc<dyn LlmProvider>>,
    direct: HashMap<&'static str, Arc<dyn LlmProvider>>,
    programs: Option<Arc<dyn LlmProvider>>,
}

/// Where one model's calls go.
enum Route<'a> {
    Main,
    ChatGpt,
    Local(&'a str),
    Program(&'a str),
    Direct(&'static DirectSupplier, &'a str),
}

impl Routed {
    /// Route between `main` and `chatgpt` (absent: not signed in).
    pub fn new(main: Arc<dyn LlmProvider>, chatgpt: Option<Arc<dyn LlmProvider>>) -> Self {
        Self {
            main,
            chatgpt,
            local: None,
            direct: HashMap::new(),
            programs: None,
        }
    }

    /// Also send `program/` models to `programs` (programs on this computer).
    pub fn with_programs(mut self, programs: Arc<dyn LlmProvider>) -> Self {
        self.programs = Some(programs);
        self
    }

    /// Also send `local/` models to `local` (an OpenAI-compatible server).
    pub fn with_local(mut self, local: Arc<dyn LlmProvider>) -> Self {
        self.local = Some(local);
        self
    }

    /// Also send `direct/<supplier>/` models to `provider` (built with the person's key for it).
    pub fn with_direct(mut self, supplier: &'static str, provider: Arc<dyn LlmProvider>) -> Self {
        self.direct.insert(supplier, provider);
        self
    }

    /// Whether `model`'s calls go to the main provider (OpenRouter), so its credit is what they
    /// spend.
    pub fn goes_to_main(model: &str) -> bool {
        matches!(Self::route(model), Route::Main)
    }

    fn route(model: &str) -> Route<'_> {
        if let Some(m) = model.strip_prefix(LOCAL_MODEL_PREFIX) {
            return Route::Local(m);
        }
        if let Some(m) = model.strip_prefix(PROGRAM_MODEL_PREFIX) {
            return Route::Program(m);
        }
        if model.starts_with(CHATGPT_MODEL_PREFIX) {
            return Route::ChatGpt;
        }
        match direct::split(model) {
            Some((s, m)) => Route::Direct(s, m),
            None => Route::Main,
        }
    }

    /// The programs on this computer, or why they can't be used.
    fn program(&self) -> Result<&Arc<dyn LlmProvider>, LlmError> {
        self.programs.as_ref().ok_or_else(|| LlmError::Rejected {
            provider: "This computer".to_owned(),
            status: 0,
            message: "This Thrall can't run programs on its computer; pick another model."
                .to_owned(),
        })
    }

    /// The provider for a direct supplier, or why there isn't one.
    fn supplier(&self, s: &DirectSupplier) -> Result<&Arc<dyn LlmProvider>, LlmError> {
        self.direct.get(s.id).ok_or_else(|| missing_key(s))
    }

    /// A direct model's prices, from the main provider's list under OpenRouter's name for it.
    async fn direct_prices(&self, s: &DirectSupplier, model: &str) -> Option<ModelPrices> {
        for id in direct::price_ids(s, model) {
            if let Some(p) = self.main.prices(&id).await {
                return Some(p);
            }
        }
        None
    }
}

/// Thrall has no key for `s`.
fn missing_key(s: &DirectSupplier) -> LlmError {
    LlmError::Rejected {
        provider: s.name.to_owned(),
        status: 0,
        message: format!(
            "Thrall has no {name} key. Add one with `thrall key set {id}` (Settings → Providers \
             shows how), or pick another model.",
            name = s.name,
            id = s.id
        ),
    }
}

/// `usage` with its cost worked out from `prices` when the supplier didn't say.
fn priced(mut usage: TokenUsage, prices: Option<ModelPrices>) -> TokenUsage {
    if usage.cost_usd.is_none() {
        usage.cost_usd = prices.map(|p| direct::cost(&p, &usage));
    }
    usage
}

impl LlmProvider for Routed {
    fn name(&self) -> &str {
        self.main.name()
    }

    fn stream(&self, mut request: ChatRequest) -> ReplyStream {
        match Self::route(&request.model) {
            Route::Local(model) => {
                request.model = model.to_owned();
                match &self.local {
                    Some(local) => local
                        .stream(request)
                        .map(|item| item.map_err(|e| e.relabel("The local model server", "local")))
                        .boxed(),
                    None => stream::once(async {
                        Err(LlmError::Rejected {
                            provider: "local".to_owned(),
                            status: 0,
                            message: "No local model server is set up. Set [provider] \
                                      local_base_url (Ollama: http://localhost:11434/v1), or pick \
                                      another model."
                                .to_owned(),
                        })
                    })
                    .boxed(),
                }
            }
            Route::ChatGpt => match &self.chatgpt {
                Some(chatgpt) => chatgpt.stream(request),
                None => stream::once(async {
                    Err(LlmError::Rejected {
                        provider: "ChatGPT".to_owned(),
                        status: 0,
                        message: "Thrall isn't signed in to ChatGPT. Sign in with `/login \
                                  chatgpt` (or Settings → Models), or pick another model."
                            .to_owned(),
                    })
                })
                .boxed(),
            },
            Route::Direct(s, model) => {
                let provider = match self.supplier(s) {
                    Ok(p) => p.clone(),
                    Err(e) => return stream::once(async { Err(e) }).boxed(),
                };
                let model = model.to_owned();
                request.model = model.clone();
                let main = self.main.clone();
                // These suppliers report tokens but not cost: work it out from OpenRouter's
                // prices for the same model, looked up only when the count arrives.
                provider
                    .stream(request)
                    .then(move |item| {
                        let main = main.clone();
                        let model = model.clone();
                        async move {
                            match item {
                                Ok(StreamEvent::Usage(u)) if u.cost_usd.is_none() => {
                                    let mut prices = None;
                                    for id in direct::price_ids(s, &model) {
                                        prices = main.prices(&id).await;
                                        if prices.is_some() {
                                            break;
                                        }
                                    }
                                    Ok(StreamEvent::Usage(priced(u, prices)))
                                }
                                Ok(event) => Ok(event),
                                Err(e) => Err(e.relabel(s.name, s.id)),
                            }
                        }
                    })
                    .boxed()
            }
            Route::Program(model) => match self.program() {
                Ok(p) => {
                    request.model = model.to_owned();
                    p.stream(request)
                }
                Err(e) => stream::once(async { Err(e) }).boxed(),
            },
            Route::Main => self.main.stream(request),
        }
    }

    fn accepts_images<'a>(&'a self, model: &'a str) -> BoxFuture<'a, Option<bool>> {
        match Self::route(model) {
            Route::Local(m) => match &self.local {
                Some(l) => l.accepts_images(m),
                None => Box::pin(async { None }),
            },
            Route::ChatGpt => match &self.chatgpt {
                Some(c) => c.accepts_images(model),
                None => Box::pin(async { None }),
            },
            // OpenRouter's list says what the same model accepts.
            Route::Direct(s, m) => Box::pin(async move {
                for id in direct::price_ids(s, m) {
                    if let Some(yes) = self.main.accepts_images(&id).await {
                        return Some(yes);
                    }
                }
                None
            }),
            Route::Program(_) => Box::pin(async { Some(false) }),
            Route::Main => self.main.accepts_images(model),
        }
    }

    fn prices<'a>(&'a self, model: &'a str) -> BoxFuture<'a, Option<ModelPrices>> {
        match Self::route(model) {
            // Included in the ChatGPT plan, or run locally.
            Route::ChatGpt | Route::Local(_) | Route::Program(_) => Box::pin(async {
                Some(ModelPrices {
                    prompt: 0.0,
                    completion: 0.0,
                    image_output: None,
                })
            }),
            Route::Direct(s, m) => Box::pin(self.direct_prices(s, m)),
            Route::Main => self.main.prices(model),
        }
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

    fn speak(&self, mut request: SpeechRequest) -> BoxFuture<'_, Result<SpeechReply, LlmError>> {
        if let Route::Program(model) = Self::route(&request.model) {
            let model = model.to_owned();
            return Box::pin(async move {
                request.model = model;
                self.program()?.speak(request).await
            });
        }
        let Route::Direct(s, model) = Self::route(&request.model) else {
            return self.main.speak(request);
        };
        let model = model.to_owned();
        Box::pin(async move {
            let provider = self.supplier(s)?;
            request.model = model.clone();
            let mut reply = provider
                .speak(request)
                .await
                .map_err(|e| e.relabel(s.name, s.id))?;
            if reply.usage != TokenUsage::default() {
                reply.usage = priced(reply.usage, self.direct_prices(s, &model).await);
            }
            Ok(reply)
        })
    }

    fn make_image(&self, mut request: ImageRequest) -> BoxFuture<'_, Result<ImageReply, LlmError>> {
        if let Route::Program(model) = Self::route(&request.model) {
            let model = model.to_owned();
            return Box::pin(async move {
                request.model = model;
                self.program()?.make_image(request).await
            });
        }
        let Route::Direct(s, model) = Self::route(&request.model) else {
            return self.main.make_image(request);
        };
        let model = model.to_owned();
        Box::pin(async move {
            let provider = self.supplier(s)?;
            request.model = model.clone();
            let mut reply = provider
                .make_image(request)
                .await
                .map_err(|e| e.relabel(s.name, s.id))?;
            reply.usage = priced(reply.usage, self.direct_prices(s, &model).await);
            Ok(reply)
        })
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
            thinking: None,
        };
        r.stream(request).next().await.unwrap()
    }

    #[test]
    fn only_main_models_spend_the_main_providers_credit() {
        assert!(Routed::goes_to_main("deepseek/deepseek-v4.1-flash"));
        assert!(Routed::goes_to_main("openai/gpt-5"));
        for other in [
            "chatgpt/gpt-5",
            "local/llama3.1",
            "program/codex",
            "direct/openai/gpt-5",
        ] {
            assert!(!Routed::goes_to_main(other), "{other}");
        }
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

    #[tokio::test]
    async fn program_models_go_to_this_computers_programs_and_are_free() {
        let r = Routed::new(Arc::new(Named("main")), None).with_programs(Arc::new(Echo));
        assert_eq!(
            first(&r, "program/whisper/ggml-base.en").await.unwrap(),
            StreamEvent::Text("whisper/ggml-base.en".into())
        );
        let free = r.prices("program/sd-webui").await.unwrap();
        assert_eq!((free.prompt, free.completion), (0.0, 0.0));
        let none = Routed::new(Arc::new(Named("main")), None);
        let e = first(&none, "program/whisper").await.unwrap_err();
        assert!(e.to_string().contains("can't run programs"));
    }
    /// Knows OpenRouter's prices for `anthropic/sonnet-4.5` only.
    struct Priced;
    impl LlmProvider for Priced {
        fn name(&self) -> &str {
            "main"
        }
        fn stream(&self, _request: ChatRequest) -> ReplyStream {
            stream::once(async { Ok(StreamEvent::Text("main".into())) }).boxed()
        }
        fn prices<'a>(&'a self, model: &'a str) -> BoxFuture<'a, Option<ModelPrices>> {
            Box::pin(async move {
                (model == "anthropic/sonnet-4.5").then_some(ModelPrices {
                    prompt: 0.000_003,
                    completion: 0.000_015,
                    image_output: None,
                })
            })
        }
    }

    /// Answers with the model it was asked for and a token count without a cost; or, asked for
    /// `bad`, refuses the key.
    struct Counting;
    impl LlmProvider for Counting {
        fn name(&self) -> &str {
            "OpenRouter"
        }
        fn stream(&self, request: ChatRequest) -> ReplyStream {
            if request.model == "bad" {
                return stream::once(async {
                    Err(LlmError::Unauthorized {
                        provider: "OpenRouter".into(),
                        provider_id: "openrouter".into(),
                    })
                })
                .boxed();
            }
            stream::iter(vec![
                Ok(StreamEvent::Text(request.model)),
                Ok(StreamEvent::Usage(TokenUsage {
                    input_tokens: 1000,
                    output_tokens: 100,
                    cached_tokens: 0,
                    cost_usd: None,
                })),
            ])
            .boxed()
        }
    }

    #[tokio::test]
    async fn direct_models_go_to_their_supplier_priced_from_openrouter() {
        let r = Routed::new(Arc::new(Priced), None).with_direct("anthropic", Arc::new(Counting));
        let request = |model: &str| ChatRequest {
            model: model.into(),
            messages: vec![ChatMessage::new(ChatRole::User, "hi")],
            max_tokens: 10,
            tools: Vec::new(),
            thinking: None,
        };
        let events: Vec<_> = r
            .stream(request("direct/anthropic/sonnet-4-5"))
            .collect()
            .await;
        assert_eq!(
            events[0].as_ref().unwrap(),
            &StreamEvent::Text("sonnet-4-5".into())
        );
        let Ok(StreamEvent::Usage(u)) = &events[1] else {
            panic!("no usage: {events:?}");
        };
        assert!((u.cost_usd.unwrap() - 0.0045).abs() < 1e-12);
        // Its errors name the supplier and its key.
        let e = first(&r, "direct/anthropic/bad").await.unwrap_err();
        assert!(e.to_string().contains("Anthropic") && e.to_string().contains("key set anthropic"));
        // No key for a supplier: say how to add one.
        let e = first(&r, "direct/openai/gpt-5").await.unwrap_err();
        assert!(e.to_string().contains("thrall key set openai"));
        // OpenRouter's own ids are untouched.
        assert_eq!(
            first(&r, "anthropic/sonnet-4.5").await.unwrap(),
            StreamEvent::Text("main".into())
        );
    }
}
