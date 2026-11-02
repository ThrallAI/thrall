//! What the settings screen shows: the config file as JSON and which keys are stored. Values that
//! could be secrets are hidden; key values never leave the engine.

use futures::StreamExt;
use serde_json::Value;
use thrall_core::config::ProviderKind;
use thrall_core::constants::CONFIG_FILE;
use thrall_llm::{ChatMessage, ChatRequest, ChatRole, StreamEvent};
use thrall_protocol::{ErrorCode, KeyStatus, Reply, SkillDetail, SkillInfo, Usage};
use thrall_tools::constants::{
    SKILL_DESCRIPTION_MAX_CHARS, SKILL_FILE, SKILL_MAX_CHARS, SKILL_SOURCE_MY_WORK,
    SKILL_SOURCE_USER, SKILLS_DIR,
};
use thrall_tools::skills::{self, Skill};

use crate::constants::{HIDDEN_VALUE, SETTINGS_KEYS, SKILL_SAFER_MAX_TOKENS, SKILL_SAFER_PROMPT};
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

/// One of the person's own skills, by name: only ever a folder directly inside their skills folder.
fn my_skill(state: &AppState, name: &str) -> Result<Skill, String> {
    let data = state
        .my_dir()
        .ok_or("Thrall has no data folder, so there are no skills of yours.")?;
    let skill = skills::personal(&data)
        .into_iter()
        .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
        .ok_or_else(|| format!("There's no skill of yours called {name}."))?;
    let root = data.join(SKILLS_DIR);
    if skill.dir.parent() != Some(root.as_path()) {
        return Err(format!("{name} isn't in your skills folder."));
    }
    Ok(skill)
}

/// Remove one of the person's own skills: its folder goes. Refused for anything that isn't a
/// personal skill's folder.
pub fn remove_skill(state: &AppState, name: &str) -> Result<(), String> {
    let skill = my_skill(state, name)?;
    std::fs::remove_dir_all(&skill.dir).map_err(|e| format!("Couldn't remove {name}: {e}"))
}

fn detail(state: &AppState, skill: Skill) -> SkillDetail {
    SkillDetail {
        enabled: !skills::is_disabled(&skill.name, &state.config.skills.disabled),
        name: skill.name,
        description: skill.description,
        instructions: skill.body,
        helper: skill.helper,
        rating: skill.rating.map(|r| r.as_str().to_owned()),
    }
}

/// One of the person's own skills in full.
pub fn skill_detail(state: &AppState, name: &str) -> Result<SkillDetail, String> {
    Ok(detail(state, my_skill(state, name)?))
}

/// Why a description or instructions can't be saved.
fn check_skill_text(description: &str, instructions: &str) -> Result<(), String> {
    if description.trim().is_empty() || description.chars().count() > SKILL_DESCRIPTION_MAX_CHARS {
        return Err(format!(
            "The description must say in one sentence when to use the skill (at most \
             {SKILL_DESCRIPTION_MAX_CHARS} characters)."
        ));
    }
    if instructions.trim().is_empty() {
        return Err("The skill needs instructions.".to_owned());
    }
    if instructions.chars().count() > SKILL_MAX_CHARS {
        return Err(format!(
            "The instructions are too long (at most {SKILL_MAX_CHARS} characters)."
        ));
    }
    Ok(())
}

/// Change one of the person's own skills. A learned one is rated again from what it now says:
/// the person wrote this version, so only what it does can raise the rating.
pub fn edit_skill(
    state: &AppState,
    name: &str,
    description: &str,
    instructions: &str,
) -> Result<SkillDetail, String> {
    let skill = my_skill(state, name)?;
    check_skill_text(description, instructions)?;
    let rating = skill
        .rating
        .map(|_| skills::rate(SKILL_SOURCE_USER, instructions).0);
    let text = skills::skill_file(&skill.name, description, skill.helper, rating, instructions);
    std::fs::write(skill.dir.join(SKILL_FILE), text)
        .map_err(|e| format!("Couldn't save {}: {e}", skill.name))?;
    skill_detail(state, &skill.name)
}

/// The rewrite the model gave.
#[derive(serde::Deserialize)]
struct Safer {
    description: String,
    instructions: String,
}

/// The rewrite in the model's answer (a JSON object, possibly in a code fence).
fn safer_from(answer: &str) -> Option<Safer> {
    let start = answer.find('{')?;
    let end = answer.rfind('}')?;
    if start >= end {
        return None;
    }
    let safer: Safer = serde_json::from_str(&answer[start..=end]).ok()?;
    check_skill_text(&safer.description, &safer.instructions).ok()?;
    Some(safer)
}

/// A safer version of one of the person's own skills (one model call), rated the way a skill
/// Thrall worked out itself would be. Nothing is saved.
pub async fn make_skill_safe(state: &AppState, name: &str) -> Result<Reply, (ErrorCode, String)> {
    let skill = my_skill(state, name).map_err(|e| (ErrorCode::Invalid, e))?;
    let (_, why) = skills::rate(SKILL_SOURCE_MY_WORK, &skill.body);
    let rated = skill.rating.map_or("not rated", |r| r.as_str());
    let mut asked = format!(
        "Name: {}\nDescription: {}\nRated: {rated}",
        skill.name, skill.description
    );
    if !why.is_empty() {
        asked.push_str(&format!(" ({})", why.join(", ")));
    }
    asked.push_str(&format!("\n\nInstructions:\n{}", skill.body));
    let model = state.model();
    let request = ChatRequest {
        model: model.clone(),
        messages: vec![
            ChatMessage::new(ChatRole::System, SKILL_SAFER_PROMPT),
            ChatMessage::new(ChatRole::User, asked),
        ],
        max_tokens: SKILL_SAFER_MAX_TOKENS,
        tools: Vec::new(),
    };
    let mut answer = String::new();
    let mut usage = Usage::default();
    let mut stream = state.llm.stream(request);
    while let Some(item) = stream.next().await {
        match item {
            Ok(StreamEvent::Text(t)) => answer.push_str(&t),
            Ok(StreamEvent::Usage(u)) => {
                usage = Usage {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cost_usd: u.cost_usd,
                }
            }
            Ok(_) => {}
            Err(e) => {
                return Err((
                    ErrorCode::ProviderUnavailable,
                    format!("Couldn't ask {model} for a safer version: {e}"),
                ));
            }
        }
    }
    if let Err(e) = crate::audio::record_loose_usage(state, None, &model, usage).await {
        tracing::warn!(%e, "couldn't record what making a skill safe cost");
    }
    let safer = safer_from(&answer).ok_or((
        ErrorCode::ProviderUnavailable,
        "The model didn't give a usable safer version. Try again, or edit the skill yourself."
            .to_owned(),
    ))?;
    let (rating, why) = skills::rate(SKILL_SOURCE_MY_WORK, &safer.instructions);
    Ok(Reply::SkillSafer {
        name: skill.name,
        description: safer.description.trim().to_owned(),
        instructions: safer.instructions.trim().to_owned(),
        rating: rating.as_str().to_owned(),
        why,
    })
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
    fn a_safer_version_is_read_from_the_answer_even_in_a_fence() {
        let got = safer_from(
            "```json\n{\"description\": \"Tidy the build folder\", \"instructions\": \"1. Ask \
             before removing anything.\"}\n```",
        )
        .unwrap();
        assert_eq!(got.description, "Tidy the build folder");
        assert!(got.instructions.starts_with("1. Ask"));
        assert!(safer_from("I can't do that.").is_none());
        assert!(
            safer_from("{\"description\": \"\", \"instructions\": \"x\"}").is_none(),
            "an empty description isn't usable"
        );
    }

    #[test]
    fn skill_text_is_checked_before_saving() {
        assert!(check_skill_text("When to tidy", "1. Look.").is_ok());
        assert!(check_skill_text("  ", "1. Look.").is_err());
        assert!(check_skill_text("When to tidy", " ").is_err());
        let long = "x".repeat(SKILL_DESCRIPTION_MAX_CHARS + 1);
        assert!(check_skill_text(&long, "1. Look.").is_err());
    }

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
