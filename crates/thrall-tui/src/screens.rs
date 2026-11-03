//! The terminal's versions of the desktop's settings screens: `/settings`, `/skills` and
//! `/skill`, `/lessons` and `/group`. Each asks the engine and shows the answer in the
//! conversation; changes go through the same requests the desktop app sends.

use serde_json::Value;
use thrall_protocol::{
    GroupInfo, LessonInfo, Reply, Request, SettingValue, SkillDetail, SkillInfo,
};

use crate::app::App;
use crate::client::Client;
use crate::constants::{
    CONFIRM_WORD, GROUP_NONE, SETTINGS_ARRAY_SHOWN, SKILL_REMOVE, SKILL_SAFE, SKILL_SAVE_SAFE,
};
use crate::transcript::Kind;

/// Run one of these commands if `name` is one. `Ok(false)`: it isn't, the engine handles it.
pub async fn run(client: &Client, app: &mut App, name: &str, args: &str) -> anyhow::Result<bool> {
    let args = args.trim();
    match name {
        "settings" if args.is_empty() => settings(client, app).await?,
        "settings" => set(client, app, args).await?,
        "skills" => skills(client, app).await?,
        "skill" => skill(client, app, args).await?,
        "lessons" => lessons(client, app, args).await?,
        "group" => group(client, app, args).await?,
        _ => return Ok(false),
    }
    Ok(true)
}

fn show(app: &mut App, markdown: String) {
    app.transcript.push(Kind::Command(markdown));
    app.scroll_up = 0;
}

// ---------- settings ----------

async fn settings(client: &Client, app: &mut App) -> anyhow::Result<()> {
    if let Reply::SettingsShown {
        config,
        config_file,
        model,
        keys,
        pending,
        ..
    } = client.request(Request::SettingsGet {}).await?
    {
        let mut md = format!("**Settings**\n\nModel in use: `{model}`\n");
        if let Some(file) = config_file {
            md.push_str(&format!("File: `{file}`\n"));
        }
        if pending {
            md.push_str("\nSome changes apply when Thrall finishes what it's doing.\n");
        }
        md.push_str("\n**Keys** (set with `thrall key set <name>` in a terminal)\n");
        for k in keys {
            let state = if k.set { "set" } else { "not set" };
            md.push_str(&format!("- `{}`: {state} · {}\n", k.name, k.purpose));
        }
        md.push('\n');
        md.push_str(&settings_lines(&config));
        md.push_str(
            "\nChange one: `/settings <name> <value>`, e.g. `/settings agent.verify true`.",
        );
        show(app, md);
    }
    Ok(())
}

/// Every setting as `name = value`, grouped by section, in the dotted names `/settings` takes.
pub fn settings_lines(config: &Value) -> String {
    let mut out = String::new();
    let Some(sections) = config.as_object() else {
        return out;
    };
    for (section, body) in sections {
        let mut lines = Vec::new();
        flatten(section, body, &mut lines);
        if lines.is_empty() {
            continue;
        }
        out.push_str(&format!("**{section}**\n"));
        for (key, value) in lines {
            out.push_str(&format!("- `{key}` = {value}\n"));
        }
        out.push('\n');
    }
    out
}

fn flatten(path: &str, value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                flatten(&format!("{path}.{k}"), v, out);
            }
        }
        Value::Array(items) => {
            let shown: Vec<String> = items
                .iter()
                .take(SETTINGS_ARRAY_SHOWN)
                .map(scalar)
                .collect();
            let more = items.len().saturating_sub(SETTINGS_ARRAY_SHOWN);
            let tail = if more > 0 {
                format!(" … and {more} more")
            } else {
                String::new()
            };
            out.push((path.to_owned(), format!("[{}]{tail}", shown.join(", "))));
        }
        other => out.push((path.to_owned(), scalar(other))),
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) if s.is_empty() => "(empty)".to_owned(),
        Value::String(s) => format!("`{s}`"),
        Value::Object(_) => "{…}".to_owned(),
        other => other.to_string(),
    }
}

/// What a typed value means: yes/no, a whole number, or text.
pub fn parse_value(text: &str) -> SettingValue {
    match text.to_lowercase().as_str() {
        "true" | "yes" | "on" => SettingValue::Bool(true),
        "false" | "no" | "off" => SettingValue::Bool(false),
        _ => match text.parse::<i64>() {
            Ok(n) => SettingValue::Int(n),
            Err(_) => SettingValue::Text(text.trim_matches('"').to_owned()),
        },
    }
}

async fn set(client: &Client, app: &mut App, args: &str) -> anyhow::Result<()> {
    let Some((key, value)) = args.split_once(char::is_whitespace) else {
        app.info("Give the setting and its new value: /settings <name> <value>.");
        return Ok(());
    };
    let value = value.trim();
    let reply = client
        .request(Request::SettingsSet {
            key: key.to_owned(),
            value: parse_value(value),
        })
        .await?;
    let later = matches!(reply, Reply::SettingsShown { pending: true, .. });
    app.info(if later {
        format!("Saved {key} = {value}. It applies when Thrall finishes what it's doing.")
    } else {
        format!("Saved {key} = {value}.")
    });
    Ok(())
}

// ---------- skills ----------

async fn skills(client: &Client, app: &mut App) -> anyhow::Result<()> {
    if let Reply::SettingsShown { skills, .. } = client.request(Request::SettingsGet {}).await? {
        show(app, skills_list(&skills));
    }
    Ok(())
}

/// The skills Thrall can use, with whether each is on and how it's rated.
pub fn skills_list(skills: &[SkillInfo]) -> String {
    if skills.is_empty() {
        return "**Skills**\n\nNone yet. Thrall learns them as it works, or you can add your own."
            .to_owned();
    }
    let mut md = String::from("**Skills**\n\n");
    for s in skills {
        let mut tags = Vec::new();
        if !s.enabled {
            tags.push("off".to_owned());
        }
        if !s.personal {
            tags.push("this project's".to_owned());
        }
        if s.helper {
            tags.push("runs in a helper".to_owned());
        }
        if let Some(r) = &s.rating {
            tags.push(format!("rated {r}"));
        }
        let tags = if tags.is_empty() {
            String::new()
        } else {
            format!(" ({})", tags.join(", "))
        };
        md.push_str(&format!("- `/{}`{tags}: {}\n", s.name, s.description));
    }
    md.push_str(
        "\n`/skill <name>` shows one in full; `/skill <name> safe` suggests a safer version; \
         `/skill <name> remove` deletes one of yours.",
    );
    md
}

async fn skill(client: &Client, app: &mut App, args: &str) -> anyhow::Result<()> {
    let mut words = args.split_whitespace();
    let (Some(name), what) = (words.next(), words.next()) else {
        app.info("Say which skill: /skill <name> (/skills lists them).");
        return Ok(());
    };
    let name = name.trim_start_matches('/').to_owned();
    match what {
        None => {
            if let Reply::SkillShown { skill } = client
                .request(Request::SkillGet { name })
                .await?
            {
                show(app, skill_full(&skill));
            }
        }
        Some(SKILL_SAFE) => {
            app.info("Asking for a safer version…");
            if let Reply::SkillSafer {
                name,
                description,
                instructions,
                rating,
                why,
            } = client.request(Request::SkillMakeSafe { name }).await?
            {
                let mut md = format!("**Safer `{name}`** (would be rated {rating})\n\n{description}\n\n{instructions}");
                if !why.is_empty() {
                    md.push_str(&format!("\n\nStill rated {rating} because: {}", why.join("; ")));
                }
                md.push_str(&format!(
                    "\n\nNothing is saved yet. `/skill {name} {SKILL_SAVE_SAFE}` keeps this version."
                ));
                show(app, md);
                app.safer = Some(SkillDetail {
                    name,
                    description,
                    instructions,
                    helper: false,
                    enabled: true,
                    rating: Some(rating),
                });
            }
        }
        Some(SKILL_SAVE_SAFE) => {
            let Some(safer) = app.safer.take_if(|s| s.name == name) else {
                app.info(format!(
                    "There's no safer version of {name} to keep: ask for one with /skill {name} {SKILL_SAFE}."
                ));
                return Ok(());
            };
            client
                .request(Request::SkillEdit {
                    name: safer.name,
                    description: safer.description,
                    instructions: safer.instructions,
                })
                .await?;
            app.info(format!("Kept the safer version of {name}."));
        }
        Some(SKILL_REMOVE) if words.next() == Some(CONFIRM_WORD) => {
            client.request(Request::SkillRemove { name: name.clone() }).await?;
            app.info(format!("Removed the skill {name}."));
        }
        Some(SKILL_REMOVE) => app.info(format!(
            "This deletes the skill {name} for good. To go ahead: /skill {name} {SKILL_REMOVE} {CONFIRM_WORD}"
        )),
        Some(other) => app.info(format!(
            "/skill {name} {other} isn't something I can do. Try /skill {name}, /skill {name} \
             {SKILL_SAFE} or /skill {name} {SKILL_REMOVE}. Edit a skill's text in the desktop app's \
             Settings → Learning."
        )),
    }
    Ok(())
}

fn skill_full(s: &SkillDetail) -> String {
    let rating = s
        .rating
        .as_deref()
        .map(|r| format!(" · rated {r}"))
        .unwrap_or_default();
    let state = if s.enabled { "on" } else { "off" };
    let helper = if s.helper { " · runs in a helper" } else { "" };
    format!(
        "**`/{}`** ({state}{helper}{rating})\n\n{}\n\n{}",
        s.name, s.description, s.instructions
    )
}

// ---------- lessons ----------

async fn lessons(client: &Client, app: &mut App, args: &str) -> anyhow::Result<()> {
    let request = match args.split_once(char::is_whitespace) {
        Some(("forget", id)) => Request::ExperienceForget {
            lesson: id.trim().to_owned(),
        },
        _ if args == "forget" => {
            app.info(
                "Say which lesson: /lessons forget <id> (/lessons lists them with their ids).",
            );
            return Ok(());
        }
        _ => Request::ExperienceList {},
    };
    let forgetting = matches!(request, Request::ExperienceForget { .. });
    if let Reply::ExperienceListed { items } = client.request(request).await? {
        if forgetting {
            app.info("Forgotten: Thrall won't go by that lesson any more.");
        }
        show(app, lessons_list(&items));
    }
    Ok(())
}

/// What Thrall has learned from corrections and fixes, newest first.
pub fn lessons_list(items: &[LessonInfo]) -> String {
    if items.is_empty() {
        return "**Lessons**\n\nNone yet. When you correct Thrall, or it finds what works after a \
                failed step, it notes the lesson here."
            .to_owned();
    }
    let mut md = String::from("**Lessons** (newest first)\n\n");
    for l in items {
        md.push_str(&format!("- {} · `{}`\n", l.text, l.lesson));
    }
    md.push_str("\n`/lessons forget <id>` deletes one.");
    md
}

// ---------- groups ----------

async fn group(client: &Client, app: &mut App, args: &str) -> anyhow::Result<()> {
    let Reply::GroupListed { items } = client.request(Request::GroupList {}).await? else {
        return Ok(());
    };
    if args.is_empty() {
        let current = app.session.as_ref().and_then(|s| s.group.clone());
        show(app, groups_list(&items, current.as_deref()));
        return Ok(());
    }
    let Some(session) = app.session.as_ref().map(|s| s.session.clone()) else {
        app.info("Send a message first; that starts the session to put in a group.");
        return Ok(());
    };
    let group = if args.eq_ignore_ascii_case(GROUP_NONE) {
        None
    } else if let Some(g) = items.iter().find(|g| g.name.eq_ignore_ascii_case(args)) {
        Some(g.group.clone())
    } else {
        let Reply::GroupListed { items } = client
            .request(Request::GroupCreate {
                name: args.to_owned(),
            })
            .await?
        else {
            return Ok(());
        };
        items
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case(args))
            .map(|g| g.group.clone())
    };
    client
        .request(Request::SessionMove {
            session,
            group: group.clone(),
        })
        .await?;
    if let Some(s) = &mut app.session {
        s.group = group.clone();
    }
    app.info(match group {
        Some(_) => format!("Moved this session into {args}."),
        None => "Took this session out of its group.".to_owned(),
    });
    Ok(())
}

/// The person's groups, marking the one this session is in.
pub fn groups_list(items: &[GroupInfo], current: Option<&str>) -> String {
    let mut md = String::from("**Groups**\n\n");
    if items.is_empty() {
        md.push_str("None yet.\n");
    }
    for g in items {
        let here = if current == Some(g.group.as_str()) {
            " ← this session"
        } else {
            ""
        };
        md.push_str(&format!("- {}{here}\n", g.name));
    }
    md.push_str(&format!(
        "\n`/group <name>` puts this session in a group (making it if it's new); `/group {GROUP_NONE}` \
         takes it out."
    ));
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_read_in_the_names_settings_takes() {
        let text = settings_lines(&json!({
            "agent": { "verify": true, "max_steps": 40 },
            "provider": { "model": "x/y", "base_url": "" },
            "skills": { "disabled": ["a", "b", "c", "d", "e", "f", "g"] }
        }));
        assert!(text.contains("**agent**"));
        assert!(text.contains("- `agent.verify` = true"));
        assert!(text.contains("- `agent.max_steps` = 40"));
        assert!(text.contains("- `provider.model` = `x/y`"));
        assert!(text.contains("- `provider.base_url` = (empty)"));
        assert!(text.contains("… and 1 more"), "{text}");
    }

    #[test]
    fn typed_values_become_the_right_kind() {
        assert_eq!(parse_value("on"), SettingValue::Bool(true));
        assert_eq!(parse_value("False"), SettingValue::Bool(false));
        assert_eq!(parse_value("40"), SettingValue::Int(40));
        assert_eq!(
            parse_value("\"openai/gpt-5\""),
            SettingValue::Text("openai/gpt-5".into())
        );
    }

    #[test]
    fn skills_say_what_matters_about_each() {
        let md = skills_list(&[SkillInfo {
            name: "python-tool-setup".into(),
            description: "Start a Python tool".into(),
            personal: true,
            helper: true,
            enabled: false,
            rating: Some("medium".into()),
        }]);
        assert!(
            md.contains("`/python-tool-setup` (off, runs in a helper, rated medium)"),
            "{md}"
        );
        assert!(skills_list(&[]).contains("None yet"));
    }

    #[test]
    fn groups_mark_this_sessions() {
        let groups = [
            GroupInfo {
                group: "g1".into(),
                name: "Work".into(),
            },
            GroupInfo {
                group: "g2".into(),
                name: "Home".into(),
            },
        ];
        let md = groups_list(&groups, Some("g2"));
        assert!(
            md.contains("- Home ← this session") && md.contains("- Work\n"),
            "{md}"
        );
    }

    #[test]
    fn lessons_show_their_ids_for_forgetting() {
        let md = lessons_list(&[LessonInfo {
            lesson: "l_1".into(),
            text: "use py, not python".into(),
            at: 0,
        }]);
        assert!(md.contains("use py, not python · `l_1`"));
    }
}
