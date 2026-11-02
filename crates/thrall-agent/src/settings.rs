//! What the settings screen shows: the config file as JSON and which keys are stored. Values that
//! could be secrets are hidden; key values never leave the engine.

use serde_json::Value;
use thrall_core::config::ProviderKind;
use thrall_core::constants::CONFIG_FILE;
use thrall_protocol::{KeyStatus, Reply, SkillInfo};

use crate::constants::{
    CHATGPT_KEY_NAME, FAL_KEY_NAME, HIDDEN_VALUE, SETTINGS_KEYS, SUPPLIER_LOCAL,
    SUPPLIER_OPENROUTER,
};
use crate::state::AppState;

/// The `settings.shown` answer for `state`; `pending` when saved settings aren't in use yet.
pub async fn shown(state: &AppState, pending: bool) -> Reply {
    let mut config = serde_json::to_value(&state.config).unwrap_or(Value::Null);
    hide_secrets(&mut config);
    // Someone else in the household sees only the services shared with them.
    if state.user_id != state.owner_id
        && let Some(servers) = config
            .get_mut("mcp")
            .and_then(|m| m.get_mut("servers"))
            .and_then(Value::as_object_mut)
    {
        servers.retain(|name, _| state.mcp.server(name).is_some());
    }
    let keys = SETTINGS_KEYS
        .iter()
        .map(|(name, purpose)| KeyStatus {
            name: (*name).to_owned(),
            purpose: (*purpose).to_owned(),
            set: key_env_var(name).is_some_and(|v| std::env::var_os(v).is_some())
                || state.keys.get(name).is_some(),
        })
        .collect();
    Reply::SettingsShown {
        config,
        config_file: state
            .data_dir
            .as_ref()
            .map(|d| d.join(CONFIG_FILE).display().to_string()),
        model: state.model(),
        keys,
        plan_models: state.llm.plan_models().await.unwrap_or_default(),
        skills: skills(state),
        pending,
        mcp_signed_in: state
            .mcp
            .names()
            .into_iter()
            .filter(|n| {
                state.mcp.server(n).is_some_and(|s| s.is_remote())
                    && thrall_mcp::remote::signed_in(n)
            })
            .collect(),
    }
}

/// The environment variable that can hold key `name` instead of the keychain.
fn key_env_var(name: &str) -> Option<&'static str> {
    let main = ProviderKind::OpenRouter;
    if name == main.as_str() {
        return Some(main.key_env_var());
    }
    thrall_llm::direct::supplier(name).map(|s| s.key_env_var)
}

/// The key Thrall has for `name`: its environment variable, else the keychain.
fn stored_key(state: &AppState, name: &str) -> Option<String> {
    key_env_var(name)
        .and_then(|v| std::env::var(v).ok())
        .map(|k| k.trim().to_owned())
        .filter(|k| !k.is_empty())
        .or_else(|| state.keys.get(name))
}

/// The `provider.tested` answer: whether `supplier` works with the key Thrall has for it, and the
/// models it offers.
pub async fn test_supplier(state: &AppState, supplier: &str) -> Reply {
    let tested = |ok: bool, message: String, models: Vec<String>| Reply::ProviderTested {
        supplier: supplier.to_owned(),
        ok,
        message,
        models,
    };
    match supplier {
        SUPPLIER_OPENROUTER => {
            if stored_key(state, supplier).is_none() {
                return tested(
                    false,
                    "No OpenRouter key yet. Run `thrall key set openrouter`.".to_owned(),
                    Vec::new(),
                );
            }
            match state.llm.credit_left().await {
                Some(left) => tested(true, format!("Works. ${left:.2} credit left."), Vec::new()),
                None => tested(
                    false,
                    "OpenRouter didn't answer with that key. Check it, then run `thrall key set \
                     openrouter`."
                        .to_owned(),
                    Vec::new(),
                ),
            }
        }
        CHATGPT_KEY_NAME => match state.llm.plan_models().await {
            Ok(models) if !models.is_empty() => {
                tested(true, "Signed in and working.".to_owned(), models)
            }
            Ok(_) => tested(
                false,
                "Not signed in to ChatGPT. Use Sign in under Models.".to_owned(),
                Vec::new(),
            ),
            Err(e) => tested(false, e.to_string(), Vec::new()),
        },
        SUPPLIER_LOCAL => {
            let url = state.config.provider.local_base_url.trim();
            if url.is_empty() {
                return tested(
                    false,
                    "No local model server is set. Fill in its address first.".to_owned(),
                    Vec::new(),
                );
            }
            match thrall_llm::direct::check_local(url).await {
                Ok(models) => tested(
                    true,
                    format!("Found it: {} model(s) ready.", models.len()),
                    models,
                ),
                Err(e) => tested(
                    false,
                    format!("{e} Is Ollama or LM Studio running at {url}?"),
                    Vec::new(),
                ),
            }
        }
        FAL_KEY_NAME => {
            if state.keys.get(FAL_KEY_NAME).is_some() {
                tested(
                    true,
                    "The key is saved. fal.ai checks it when the first video is made (there's no \
                     free way to try it sooner)."
                        .to_owned(),
                    Vec::new(),
                )
            } else {
                tested(
                    false,
                    "No fal.ai key yet. Run `thrall key set fal`.".to_owned(),
                    Vec::new(),
                )
            }
        }
        other => {
            let Some(s) = thrall_llm::direct::supplier(other) else {
                return tested(false, format!("Thrall doesn't know {other}."), Vec::new());
            };
            let Some(key) = stored_key(state, s.id) else {
                return tested(
                    false,
                    format!("No {} key yet. Run `thrall key set {}`.", s.name, s.id),
                    Vec::new(),
                );
            };
            match thrall_llm::direct::check(s, &key, s.base_url).await {
                Ok(models) => tested(
                    true,
                    format!(
                        "Works: {} model(s) to pick from. A key saved while Thrall was running is \
                         used once Thrall restarts.",
                        models.len()
                    ),
                    models,
                ),
                Err(e) => tested(false, e.to_string(), Vec::new()),
            }
        }
    }
}

/// The `local.found` answer: programs and model servers on this computer, job by job.
pub async fn local_found(state: &AppState) -> Reply {
    let models_dir = state
        .data_dir
        .as_deref()
        .map(thrall_tools::programs::models_dir);
    let provider = &state.config.provider;
    let items = match &models_dir {
        Some(dir) => {
            thrall_tools::programs::find(dir, &provider.local_base_url, &provider.sd_webui_url)
                .await
        }
        None => Vec::new(),
    };
    Reply::LocalFound {
        items,
        models_dir: models_dir.map(|d| d.display().to_string()),
    }
}
/// The person's own skills (project skills belong to their folder and show there).
fn skills(state: &AppState) -> Vec<SkillInfo> {
    let Some(data) = state.my_dir() else {
        return Vec::new();
    };
    thrall_tools::skills::personal(&data)
        .into_iter()
        .map(|s| SkillInfo {
            enabled: !thrall_tools::skills::is_disabled(&s.name, &state.config.skills.disabled),
            name: s.name,
            description: s.description,
            personal: s.personal,
            helper: s.helper,
            rating: s.rating.map(|r| r.as_str().to_owned()),
        })
        .collect()
}

/// Remove one of the person's own skills: its folder goes. Refused for anything that isn't a
/// personal skill's folder.
pub fn remove_skill(state: &AppState, name: &str) -> Result<(), String> {
    let data = state
        .my_dir()
        .ok_or("Thrall has no data folder, so there are no skills of yours to remove.")?;
    let skill = thrall_tools::skills::personal(&data)
        .into_iter()
        .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
        .ok_or_else(|| format!("There's no skill of yours called {name}."))?;
    // Only ever a folder directly inside the personal skills folder.
    let root = data.join(thrall_tools::constants::SKILLS_DIR);
    if skill.dir.parent() != Some(root.as_path()) {
        return Err(format!("{name} isn't in your skills folder."));
    }
    std::fs::remove_dir_all(&skill.dir).map_err(|e| format!("Couldn't remove {name}: {e}"))
}

/// MCP servers' environment and header values are often keys: show only `keychain:` references.
/// A web address's query can hold a key too, so it's hidden.
fn hide_secrets(config: &mut Value) {
    let Some(servers) = config
        .pointer_mut("/mcp/servers")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    for server in servers.values_mut() {
        if let Some(url) = server.get_mut("url")
            && let Some(text) = url.as_str()
        {
            let bare = crate::commands::address_without_query(text);
            if bare.len() != text.len() {
                *url = Value::String(format!("{bare}?{HIDDEN_VALUE}"));
            }
        }
        for field in ["env", "headers"] {
            let Some(values) = server.get_mut(field).and_then(Value::as_object_mut) else {
                continue;
            };
            for value in values.values_mut() {
                let reference = value
                    .as_str()
                    .is_some_and(|v| thrall_core::secrets::keychain_ref(v).is_some());
                if !reference {
                    *value = Value::String(HIDDEN_VALUE.to_owned());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mcp_environment_values_are_hidden_except_keychain_references() {
        let mut config = json!({"mcp": {"servers": {"gh": {"command": "npx",
            "env": {"TOKEN": "ghp_secret", "OTHER": "keychain:github"}}}}});
        hide_secrets(&mut config);
        let env = &config["mcp"]["servers"]["gh"]["env"];
        assert_eq!(env["TOKEN"], HIDDEN_VALUE);
        assert_eq!(env["OTHER"], "keychain:github");
        assert_eq!(config["mcp"]["servers"]["gh"]["command"], "npx");
    }

    #[test]
    fn web_mcp_headers_and_address_keys_are_hidden() {
        let mut config = json!({"mcp": {"servers": {"docs": {
            "url": "https://e.com/mcp?api_key=secret",
            "headers": {"X-Api-Key": "abc", "Authorization": "keychain:docs"}}}}});
        hide_secrets(&mut config);
        let docs = &config["mcp"]["servers"]["docs"];
        assert_eq!(docs["url"], format!("https://e.com/mcp?{HIDDEN_VALUE}"));
        assert_eq!(docs["headers"]["X-Api-Key"], HIDDEN_VALUE);
        assert_eq!(docs["headers"]["Authorization"], "keychain:docs");
    }
}
