//! Building the engine (key, storage, provider, tools) for any command that talks to Thrall.

use std::sync::Arc;

use thrall_agent::AppState;
use thrall_agent::keys::SystemKeys;
use thrall_core::constants::{LOCAL_USER_ID, LOCAL_USER_NAME, person_role};
use thrall_core::ids::now_ms;
use thrall_core::{Config, Paths};
use thrall_llm::{ChatGpt, LlmProvider, OpenRouter, Routed};
use thrall_store::{SqliteStore, Store};

use crate::key;

/// The engine, or `None` (after telling the user what to do) when no key is set yet.
pub async fn build(paths: &Paths, config: Config) -> anyhow::Result<Option<AppState>> {
    let provider = config.provider.kind;
    let Some(api_key) = key::resolve(provider)? else {
        eprintln!(
            "Thrall needs your {name} key first. Run `thrall key set {name}` and paste it in.",
            name = provider.as_str()
        );
        return Ok(None);
    };
    let store = SqliteStore::open(&paths.db_file).await?;
    store
        .ensure_user(LOCAL_USER_ID, LOCAL_USER_NAME, now_ms())
        .await?;
    // Whoever runs this engine on their computer is its owner.
    store.set_role(LOCAL_USER_ID, person_role::OWNER).await?;
    let base_url = config.provider.base_url.clone();
    let openrouter: Arc<dyn LlmProvider> =
        Arc::new(OpenRouter::new(api_key.clone(), &config.provider.base_url)?);
    let embedder = OpenRouter::new(api_key, &config.provider.base_url)?;
    // Signed in to ChatGPT: `chatgpt/<model>` chats on the person's plan.
    let chatgpt = match thrall_agent::signin::stored_chatgpt(&SystemKeys) {
        Some(stored) => {
            let save: thrall_llm::chatgpt::SaveRefresh = Arc::new(|refresh, account| {
                if let Err(e) = thrall_agent::signin::save_chatgpt(refresh, account) {
                    tracing::warn!(%e, "couldn't keep the renewed ChatGPT sign-in");
                }
            });
            let provider = ChatGpt::new(stored.refresh_token, stored.account_id, save)?;
            Some(Arc::new(provider) as Arc<dyn LlmProvider>)
        }
        None => None,
    };
    let llm = Routed::new(openrouter, chatgpt);
    let mut state = AppState::new(
        config,
        Arc::new(store),
        Arc::new(llm),
        LOCAL_USER_ID.to_owned(),
    )?
    .with_embedder(Arc::new(embedder))
    .with_own_keys(Arc::new(thrall_agent::people::KeychainOwnKeys::new(
        provider, &base_url,
    )));
    state.load_terms().await?;
    state.data_dir = Some(paths.home.clone());
    Ok(Some(state))
}
