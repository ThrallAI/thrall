//! Each person's terms: whose AI key their turns use, their model, and their monthly spending
//! limit (set by the owner in Settings → People). The engine's AI provider is wrapped per person
//! ([`PersonLlm`]), so every model call made for someone (turns, helpers, titles, pictures,
//! speech) goes on their key and stops at their limit. A turn checks first ([`before_turn`]) so
//! it's refused with a clear message instead of failing part way.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use chrono::{Datelike, Local, TimeZone};
use futures::future::BoxFuture;
use futures::{StreamExt, stream};
use thrall_core::ProviderKind;
use thrall_llm::{
    ChatRequest, ImageReply, ImageRequest, LlmError, LlmProvider, ModelPrices, OpenRouter,
    ReplyStream, SpeechReply, SpeechRequest,
};
use thrall_store::{PersonTerms, Store};

use crate::constants::{LIMIT_WARN_PERCENT, OWN_KEY_SEPARATOR};
use crate::{AgentError, AppState};

/// Everyone's terms, by person id (people with none set aren't in it).
pub type Terms = Arc<RwLock<HashMap<String, PersonTerms>>>;

/// People's own AI keys: kept write-only (nothing reads one back out but the provider made
/// from it).
pub trait OwnKeys: Send + Sync {
    /// Whether `person` has set their own key.
    fn has(&self, person: &str) -> bool;
    /// A provider on `person`'s own key, or `None` when they haven't set one.
    fn provider(&self, person: &str) -> Option<Arc<dyn LlmProvider>>;
    /// Keep (or replace) `person`'s own key.
    fn set(&self, person: &str, key: &str) -> Result<(), String>;
    /// Forget `person`'s own key.
    fn clear(&self, person: &str) -> Result<(), String>;
}

/// No own keys (an engine built without a keychain, and tests that don't set any).
pub struct NoOwnKeys;

impl OwnKeys for NoOwnKeys {
    fn has(&self, _person: &str) -> bool {
        false
    }
    fn provider(&self, _person: &str) -> Option<Arc<dyn LlmProvider>> {
        None
    }
    fn set(&self, _person: &str, _key: &str) -> Result<(), String> {
        Err("This Thrall can't keep people's own keys.".to_owned())
    }
    fn clear(&self, _person: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Providers made from own keys, by person: the key each was made with, and the provider.
type Made = HashMap<String, (String, Arc<dyn LlmProvider>)>;

/// Own keys in the OS keychain, as `<provider>-<person id>` (person ids are lower-case already).
pub struct KeychainOwnKeys {
    kind: ProviderKind,
    base_url: String,
    /// Providers already made, by person: the key they were made with and the provider.
    made: Mutex<Made>,
}

impl KeychainOwnKeys {
    /// Own keys for `kind`, whose providers talk to `base_url`.
    pub fn new(kind: ProviderKind, base_url: &str) -> Self {
        Self {
            kind,
            base_url: base_url.to_owned(),
            made: Mutex::new(HashMap::new()),
        }
    }

    fn name(&self, person: &str) -> String {
        own_key_name(self.kind, person)
    }
}

/// The keychain name for `person`'s own key.
pub fn own_key_name(kind: ProviderKind, person: &str) -> String {
    format!("{}{OWN_KEY_SEPARATOR}{person}", kind.as_str())
}

impl OwnKeys for KeychainOwnKeys {
    fn has(&self, person: &str) -> bool {
        matches!(
            thrall_core::secrets::get_named(&self.name(person)),
            Ok(Some(_))
        )
    }

    fn provider(&self, person: &str) -> Option<Arc<dyn LlmProvider>> {
        let key = thrall_core::secrets::get_named(&self.name(person)).ok()??;
        let mut made = self
            .made
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((with, provider)) = made.get(person)
            && *with == key
        {
            return Some(provider.clone());
        }
        let provider: Arc<dyn LlmProvider> = match self.kind {
            ProviderKind::OpenRouter => {
                Arc::new(OpenRouter::new(key.clone(), &self.base_url).ok()?)
            }
        };
        made.insert(person.to_owned(), (key, provider.clone()));
        Some(provider)
    }

    fn set(&self, person: &str, key: &str) -> Result<(), String> {
        let name = self.name(person);
        if !thrall_core::secrets::valid_name(&name) {
            return Err("That person's key can't be kept.".to_owned());
        }
        thrall_core::secrets::set_named(&name, key).map_err(|e| e.to_string())
    }

    fn clear(&self, person: &str) -> Result<(), String> {
        self.made
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(person);
        thrall_core::secrets::delete_named(&self.name(person))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Midnight on the first of this month, local time, as Unix ms (spending limits run per
/// calendar month).
pub fn month_start_ms() -> i64 {
    let now = Local::now();
    Local
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .earliest()
        .map_or(0, |t| t.timestamp_millis())
}

/// Dollars from millionths of a dollar, for messages.
fn dollars(micros: i64) -> String {
    format!("${:.2}", micros as f64 / 1_000_000.0)
}

/// What someone has spent this month, in millionths of a dollar.
async fn spent_this_month(
    store: &dyn Store,
    person: &str,
) -> Result<i64, thrall_store::StoreError> {
    let usd = store.spent_since(person, month_start_ms()).await?;
    Ok((usd * 1_000_000.0).round() as i64)
}

fn over_limit(limit: i64) -> String {
    format!(
        "You've reached this month's spending limit of {}. It starts again on the 1st; whoever \
         runs this Thrall can raise it in Settings → People.",
        dollars(limit)
    )
}

const KEY_MISSING: &str = "Your turns use your own AI key, and it isn't set yet. Add it on your \
                           phone in ⋮ → You and your key, or whoever runs this Thrall can add it in \
                           Settings → People.";

/// Check a turn may start for `state`'s person: their own key is there if they use one, and
/// they're under their limit. `Some` is a warning to show when they're near it.
pub async fn before_turn(state: &AppState) -> Result<Option<String>, AgentError> {
    let terms = state.terms_of(&state.user_id);
    if terms.own_key {
        let keys = state.own_keys.clone();
        let person = state.user_id.clone();
        let has = tokio::task::spawn_blocking(move || keys.has(&person))
            .await
            .unwrap_or(false);
        if !has {
            return Err(AgentError::OwnKeyMissing(KEY_MISSING.to_owned()));
        }
    }
    let Some(limit) = terms.monthly_limit_micros else {
        return Ok(None);
    };
    let spent = spent_this_month(state.store.as_ref(), &state.user_id).await?;
    if spent >= limit {
        return Err(AgentError::OverLimit(over_limit(limit)));
    }
    Ok((spent * 100 >= limit * LIMIT_WARN_PERCENT).then(|| {
        format!(
            "You've used {} of this month's {} limit.",
            dollars(spent),
            dollars(limit)
        )
    }))
}

/// The engine's AI provider as one person uses it: their own key when they use one, and nothing
/// that costs money once they're at their limit.
#[derive(Clone)]
pub struct PersonLlm {
    person: String,
    base: Arc<dyn LlmProvider>,
    store: Arc<dyn Store>,
    terms: Terms,
    own_keys: Arc<dyn OwnKeys>,
}

impl PersonLlm {
    /// `base` (the owner's provider) as `person` uses it.
    pub fn new(
        person: &str,
        base: Arc<dyn LlmProvider>,
        store: Arc<dyn Store>,
        terms: Terms,
        own_keys: Arc<dyn OwnKeys>,
    ) -> Self {
        Self {
            person: person.to_owned(),
            base,
            store,
            terms,
            own_keys,
        }
    }

    fn terms(&self) -> PersonTerms {
        self.terms
            .read()
            .ok()
            .and_then(|t| t.get(&self.person).cloned())
            .unwrap_or_default()
    }

    /// Whose key: the owner's, or the person's own (an error when they use one and it's unset).
    async fn provider(&self) -> Result<Arc<dyn LlmProvider>, LlmError> {
        if !self.terms().own_key {
            return Ok(self.base.clone());
        }
        let keys = self.own_keys.clone();
        let person = self.person.clone();
        tokio::task::spawn_blocking(move || keys.provider(&person))
            .await
            .ok()
            .flatten()
            .ok_or_else(|| LlmError::KeyMissing {
                message: KEY_MISSING.to_owned(),
            })
    }

    /// `provider`, for something that costs money: refused at the limit.
    async fn paying(&self) -> Result<Arc<dyn LlmProvider>, LlmError> {
        if let Some(limit) = self.terms().monthly_limit_micros {
            // If the spending can't be read, the call goes ahead: the limit is a guard, not a
            // lock that a storage hiccup should trip.
            if let Ok(spent) = spent_this_month(self.store.as_ref(), &self.person).await
                && spent >= limit
            {
                return Err(LlmError::OverLimit {
                    message: over_limit(limit),
                });
            }
        }
        self.provider().await
    }
}

impl LlmProvider for PersonLlm {
    fn name(&self) -> &str {
        self.base.name()
    }

    fn stream(&self, request: ChatRequest) -> ReplyStream {
        let me = self.clone();
        stream::once(async move {
            match me.paying().await {
                Ok(provider) => provider.stream(request),
                Err(e) => stream::once(async move { Err(e) }).boxed(),
            }
        })
        .flatten()
        .boxed()
    }

    fn accepts_images<'a>(&'a self, model: &'a str) -> BoxFuture<'a, Option<bool>> {
        Box::pin(async move {
            match self.provider().await {
                Ok(p) => p.accepts_images(model).await,
                Err(_) => self.base.accepts_images(model).await,
            }
        })
    }

    fn prices<'a>(&'a self, model: &'a str) -> BoxFuture<'a, Option<ModelPrices>> {
        Box::pin(async move {
            match self.provider().await {
                Ok(p) => p.prices(model).await,
                Err(_) => self.base.prices(model).await,
            }
        })
    }

    fn credit_left(&self) -> BoxFuture<'_, Option<f64>> {
        Box::pin(async move { self.provider().await.ok()?.credit_left().await })
    }

    fn plan_models(&self) -> BoxFuture<'_, Result<Vec<String>, LlmError>> {
        Box::pin(async move { self.provider().await?.plan_models().await })
    }

    fn speak(&self, request: SpeechRequest) -> BoxFuture<'_, Result<SpeechReply, LlmError>> {
        Box::pin(async move { self.paying().await?.speak(request).await })
    }

    fn make_image(&self, request: ImageRequest) -> BoxFuture<'_, Result<ImageReply, LlmError>> {
        Box::pin(async move { self.paying().await?.make_image(request).await })
    }
}

#[cfg(test)]
mod tests {
    use chrono::Timelike;

    use super::*;

    #[test]
    fn own_keys_have_keychain_names() {
        let name = own_key_name(ProviderKind::OpenRouter, "pe_0192abc");
        assert_eq!(name, "openrouter-pe_0192abc");
        assert!(thrall_core::secrets::valid_name(&name));
    }

    #[test]
    fn the_month_starts_on_the_first() {
        let start = Local.timestamp_millis_opt(month_start_ms()).unwrap();
        assert_eq!((start.day(), start.hour(), start.minute()), (1, 0, 0));
    }

    #[test]
    fn money_reads_in_dollars() {
        assert_eq!(dollars(5_000_000), "$5.00");
        assert_eq!(dollars(4_105_000), "$4.11");
    }
}
