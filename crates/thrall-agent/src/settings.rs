//! What the settings screen shows: the config file as JSON and which keys are stored. Values that
//! could be secrets are hidden; key values never leave the engine.

use serde_json::Value;
use thrall_core::config::ProviderKind;
use thrall_core::constants::CONFIG_FILE;
use thrall_protocol::{HelperTypeInfo, KeyStatus, Reply, SkillInfo};

use crate::constants::{HIDDEN_VALUE, SETTINGS_KEYS};
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
    let provider = ProviderKind::OpenRouter;
    let keys = SETTINGS_KEYS
        .iter()
        .map(|(name, purpose)| {
            let from_env =
                *name == provider.as_str() && std::env::var_os(provider.key_env_var()).is_some();
            KeyStatus {
                name: (*name).to_owned(),
                purpose: (*purpose).to_owned(),
                set: from_env || state.keys.get(name).is_some(),
            }
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
        helper_types: thrall_tools::agents::personal(state.my_dir().as_deref())
            .into_iter()
            .map(|a| HelperTypeInfo {
                name: a.name,
                description: a.description,
                read_only: a.read_only,
                model: a.model,
                source: a.source.as_str().to_owned(),
            })
            .collect(),
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
