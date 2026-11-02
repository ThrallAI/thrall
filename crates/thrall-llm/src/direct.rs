//! Suppliers reached directly with the person's own key (OpenAI, Anthropic, Google, ElevenLabs).
//! Their models are named `direct/<supplier>/<model>`; OpenRouter's model list (which needs no
//! key) supplies what they accept and cost, since these APIs don't report it.

use std::time::Duration;

use futures::future::BoxFuture;
use serde::Deserialize;

use crate::constants::{
    ANTHROPIC_KEY_HEADER, ANTHROPIC_VERSION, ANTHROPIC_VERSION_HEADER, CONNECT_TIMEOUT_SECS,
    DIRECT_MODEL_PREFIX, DIRECT_SUPPLIERS, ELEVENLABS_ID, ELEVENLABS_KEY_HEADER,
    GOOGLE_MODEL_LIST_PREFIX, LOCAL_MODEL_PREFIX, MODELS_PATH, READ_TIMEOUT_SECS, TEST_MODELS_MAX,
};
use crate::error::LlmError;
use crate::openrouter::{error_from_response, network};
use crate::provider::{Embedder, Embeddings};
use crate::types::{ModelPrices, TokenUsage};

/// A company Thrall talks to directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectSupplier {
    /// Its name in model ids, the keychain and `thrall key set` (`openai`).
    pub id: &'static str,
    /// What it's called on screen.
    pub name: &'static str,
    /// Its API's address.
    pub base_url: &'static str,
    /// The environment variable that can hold its key instead of the keychain.
    pub key_env_var: &'static str,
    /// What its chat API calls the reply-length limit; `None`: it doesn't chat (voices only).
    pub max_tokens_field: Option<&'static str>,
}

/// The supplier called `id`.
pub fn supplier(id: &str) -> Option<&'static DirectSupplier> {
    DIRECT_SUPPLIERS.iter().find(|s| s.id == id)
}

/// `direct/openai/gpt-5` → OpenAI and `gpt-5`. `None` for any other model id (or an unknown
/// supplier, or no model after it).
pub fn split(model: &str) -> Option<(&'static DirectSupplier, &str)> {
    let rest = model.strip_prefix(DIRECT_MODEL_PREFIX)?;
    let (id, bare) = rest.split_once('/')?;
    let s = supplier(id)?;
    (!bare.is_empty()).then_some((s, bare))
}

/// The names OpenRouter may list a direct model under, best guess first: `openai/gpt-5`, and for
/// version numbers written with dashes (`sonnet-4-5`) the dotted form too
/// (`anthropic/sonnet-4.5`).
pub fn price_ids(supplier: &DirectSupplier, model: &str) -> Vec<String> {
    let exact = format!("{}/{model}", supplier.id);
    let chars: Vec<char> = model.chars().collect();
    let dotted: String = chars
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let between_digits = i > 0
                && i + 1 < chars.len()
                && chars[i - 1].is_ascii_digit()
                && chars[i + 1].is_ascii_digit();
            if c == '-' && between_digits { '.' } else { c }
        })
        .collect();
    let dotted = format!("{}/{dotted}", supplier.id);
    if dotted == exact {
        vec![exact]
    } else {
        vec![exact, dotted]
    }
}

/// What `usage` cost at `prices` (cached tokens counted at the full price: never under).
pub fn cost(prices: &ModelPrices, usage: &TokenUsage) -> f64 {
    usage.input_tokens as f64 * prices.prompt + usage.output_tokens as f64 * prices.completion
}

/// Whether `key` works with `supplier` at `base_url`, by listing its models: the model ids
/// (as Thrall names them, `direct/<supplier>/<model>`), or why not.
pub async fn check(
    supplier: &DirectSupplier,
    key: &str,
    base_url: &str,
) -> Result<Vec<String>, LlmError> {
    let mut request = models_request(base_url)?;
    request = if supplier.id == ELEVENLABS_ID {
        request.header(ELEVENLABS_KEY_HEADER, key)
    } else {
        // Anthropic's own model list wants its own headers; the others read the bearer key.
        request
            .bearer_auth(key)
            .header(ANTHROPIC_KEY_HEADER, key)
            .header(ANTHROPIC_VERSION_HEADER, ANTHROPIC_VERSION)
    };
    let prefix = format!("{DIRECT_MODEL_PREFIX}{}/", supplier.id);
    listed(request, supplier.name, supplier.id, &prefix).await
}

/// The models a local model server (Ollama, LM Studio) at `base_url` offers, as `local/<model>`.
pub async fn check_local(base_url: &str) -> Result<Vec<String>, LlmError> {
    listed(
        models_request(base_url)?,
        "The local model server",
        "local",
        LOCAL_MODEL_PREFIX,
    )
    .await
}

/// A request for the model list under `base_url`.
fn models_request(base_url: &str) -> Result<reqwest::RequestBuilder, LlmError> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(READ_TIMEOUT_SECS))
        .build()
        .map_err(|e| network(&e))?;
    Ok(client.get(format!("{}{MODELS_PATH}", base_url.trim_end_matches('/'))))
}

/// Send a model-list `request`; the ids it lists, each after `prefix`. Errors name `name` (and
/// `id` in the key hint).
async fn listed(
    request: reqwest::RequestBuilder,
    name: &str,
    id: &str,
    prefix: &str,
) -> Result<Vec<String>, LlmError> {
    let response = request
        .send()
        .await
        .map_err(|e| network(&e).relabel(name, id))?;
    if !response.status().is_success() {
        return Err(error_from_response(response).await.relabel(name, id));
    }
    let body = response
        .text()
        .await
        .map_err(|e| network(&e).relabel(name, id))?;
    Ok(model_ids(&body)
        .into_iter()
        .take(TEST_MODELS_MAX)
        .map(|m| format!("{prefix}{m}"))
        .collect())
}

/// The model ids in a model list: `{"data": [{"id": …}]}` (OpenAI, Anthropic, Google) or
/// `[{"model_id": …}]` (ElevenLabs).
fn model_ids(body: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct Item {
        id: Option<String>,
        model_id: Option<String>,
    }
    #[derive(Deserialize)]
    struct List {
        data: Vec<Item>,
    }
    let items = serde_json::from_str::<List>(body)
        .map(|l| l.data)
        .or_else(|_| serde_json::from_str::<Vec<Item>>(body))
        .unwrap_or_default();
    items
        .into_iter()
        .filter_map(|i| i.id.or(i.model_id))
        .map(|id| {
            id.strip_prefix(GOOGLE_MODEL_LIST_PREFIX)
                .map_or(id.clone(), str::to_owned)
        })
        .collect()
}

/// An embedder for a direct supplier's models: takes `direct/<supplier>/<model>` and asks the
/// supplier for `<model>`, naming it in errors.
pub struct DirectEmbedder<E> {
    inner: E,
    supplier: &'static DirectSupplier,
}

impl<E> DirectEmbedder<E> {
    /// `inner` (a client for `supplier`'s API), taking Thrall's model names.
    pub fn new(inner: E, supplier: &'static DirectSupplier) -> Self {
        Self { inner, supplier }
    }
}

impl<E: Embedder> Embedder for DirectEmbedder<E> {
    fn embed<'a>(
        &'a self,
        model: &'a str,
        inputs: Vec<String>,
    ) -> BoxFuture<'a, Result<Embeddings, LlmError>> {
        let bare = split(model).map_or(model, |(_, m)| m);
        Box::pin(async move {
            self.inner
                .embed(bare, inputs)
                .await
                .map_err(|e| e.relabel(self.supplier.name, self.supplier.id))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_model_ids_name_their_supplier() {
        let (s, m) = split("direct/openai/gpt-5").unwrap();
        assert_eq!((s.id, m), ("openai", "gpt-5"));
        let (s, m) = split("direct/google/gemini-2.5-pro").unwrap();
        assert_eq!((s.name, m), ("Google", "gemini-2.5-pro"));
        assert!(split("openai/gpt-5").is_none());
        assert!(split("direct/nobody/x").is_none());
        assert!(split("direct/openai/").is_none());
    }

    #[test]
    fn prices_are_looked_up_under_openrouter_names() {
        let anthropic = supplier("anthropic").unwrap();
        assert_eq!(
            price_ids(anthropic, "sonnet-4-5"),
            vec![
                "anthropic/sonnet-4-5".to_owned(),
                "anthropic/sonnet-4.5".to_owned()
            ]
        );
        let openai = supplier("openai").unwrap();
        assert_eq!(price_ids(openai, "gpt-5"), vec!["openai/gpt-5".to_owned()]);
        let p = ModelPrices {
            prompt: 0.000_001,
            completion: 0.000_01,
            image_output: None,
        };
        let u = TokenUsage {
            input_tokens: 1000,
            output_tokens: 100,
            cached_tokens: 0,
            cost_usd: None,
        };
        assert!((cost(&p, &u) - 0.002).abs() < 1e-12);
    }

    #[test]
    fn model_lists_of_every_shape_are_read() {
        assert_eq!(
            model_ids(r#"{"data":[{"id":"gpt-5"},{"id":"models/gemini-2.5-pro"}]}"#),
            vec!["gpt-5", "gemini-2.5-pro"]
        );
        assert_eq!(
            model_ids(r#"[{"model_id":"eleven_multilingual_v2"}]"#),
            vec!["eleven_multilingual_v2"]
        );
        assert!(model_ids("nonsense").is_empty());
    }
}
