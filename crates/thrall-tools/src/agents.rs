//! Helper types: what a helper is for, how it works and what it may use. Thrall ships seven
//! (researcher, coder, reviewer, planner, tester, explorer, summariser); a person adds their own as `agents/<name>.md` in their
//! data folder, and a project as `.thrall/agents/<name>.md`. Same name: the project's wins, then
//! the person's, then the built-in.
//!
//! A file is front matter, then the helper's instructions:
//!
//! ```text
//! ---
//! name: tester
//! description: Runs the tests and says what failed
//! tools: read_file, grep, shell
//! read_only: no
//! model: openai/gpt-5-mini
//! ---
//! You run the project's tests...
//! ```
//!
//! `tools` limits what the helper is offered (left out: every helper tool). `read_only: yes` runs
//! it in plan mode, so it can look but not change anything. `model` picks its model (left out:
//! Thrall's, or the helper model with `light: yes`).

use std::path::Path;

use crate::constants::{
    AGENT_FILE_EXT, AGENT_TYPES, AGENTS_DIR, HELPER_NO_TYPE, HELPER_USAGE_FILE, PROJECT_DIR,
};
use crate::skills::front_matter;

/// One helper type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentType {
    /// Its name (`agent {type}`).
    pub name: String,
    /// What it's for, shown to Thrall when choosing.
    pub description: String,
    /// The only tools it's offered; none: every helper tool.
    pub tools: Option<Vec<String>>,
    /// It works in plan mode: it can look but not change anything.
    pub read_only: bool,
    /// Its model; none: the helper model when `light`, else Thrall's.
    pub model: Option<String>,
    /// Reading and summing-up work: runs on the helper model (`agent.helper_model`) if set.
    pub light: bool,
    /// Its instructions, put before the task.
    pub prompt: String,
    /// Where it comes from.
    pub source: AgentSource,
}

/// Where a helper type is defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSource {
    /// Shipped with Thrall.
    BuiltIn,
    /// The person's data folder.
    Personal,
    /// The project's `.thrall/agents`.
    Project,
}

impl AgentSource {
    /// As shown on the settings screen and on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BuiltIn => "built_in",
            Self::Personal => "personal",
            Self::Project => "project",
        }
    }
}

fn yes(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "yes" | "true" | "on" | "1"
    )
}

/// A helper type from its file's text; `fallback` names it when the front matter doesn't.
pub fn parse(text: &str, fallback: &str, source: AgentSource) -> Option<AgentType> {
    let (pairs, body) = front_matter(text);
    let mut out = AgentType {
        name: fallback.to_owned(),
        description: String::new(),
        tools: None,
        read_only: false,
        model: None,
        light: false,
        prompt: body,
        source,
    };
    for (key, value) in pairs {
        match key.as_str() {
            "name" if !value.is_empty() => out.name = value,
            "description" => out.description = value,
            "tools" => {
                let tools: Vec<String> = value
                    .split(',')
                    .map(|t| t.trim().to_owned())
                    .filter(|t| !t.is_empty())
                    .collect();
                out.tools = (!tools.is_empty()).then_some(tools);
            }
            "read_only" => out.read_only = yes(&value),
            "light" => out.light = yes(&value),
            "model" if !value.is_empty() => out.model = Some(value),
            _ => {}
        }
    }
    out.name = crate::skills::slug(&out.name)?;
    (!out.prompt.trim().is_empty()).then_some(out)
}

/// Thrall's own helper types.
pub fn built_in() -> Vec<AgentType> {
    AGENT_TYPES
        .iter()
        .map(|h| AgentType {
            name: h.name.to_owned(),
            description: h.description.to_owned(),
            tools: h
                .tools
                .map(|only| only.iter().map(|t| (*t).to_owned()).collect()),
            read_only: h.read_only,
            model: None,
            light: h.light,
            prompt: h.prompt.to_owned(),
            source: AgentSource::BuiltIn,
        })
        .collect()
}

fn read_dir(root: &Path, source: AgentSource) -> Vec<AgentType> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(AGENT_FILE_EXT) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(agent) = parse(&text, &stem, source) {
            out.push(agent);
        }
    }
    out
}

/// `more` replaces anything in `all` with the same name.
fn add(all: &mut Vec<AgentType>, more: Vec<AgentType>) {
    for agent in more {
        all.retain(|a| a.name != agent.name);
        all.push(agent);
    }
}

/// Thrall's helper types and the person's own (no project's), by name.
pub fn personal(data_dir: Option<&Path>) -> Vec<AgentType> {
    let mut all = built_in();
    if let Some(data) = data_dir {
        add(
            &mut all,
            read_dir(&data.join(AGENTS_DIR), AgentSource::Personal),
        );
    }
    all.sort_by(|a, b| a.name.cmp(&b.name));
    all
}

/// Every helper type for `workspace`, by name: built-ins, then the person's, then the project's,
/// a later one replacing an earlier one with the same name.
pub fn discover(data_dir: Option<&Path>, workspace: &Path) -> Vec<AgentType> {
    let mut all = personal(data_dir);
    add(
        &mut all,
        read_dir(
            &workspace.join(PROJECT_DIR).join(AGENTS_DIR),
            AgentSource::Project,
        ),
    );
    all.sort_by(|a, b| a.name.cmp(&b.name));
    all
}

/// How much the helpers have been used, kept in the person's data folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HelperUsage {
    /// Uses by helper type (`HELPER_NO_TYPE` for a helper started without one).
    #[serde(default)]
    pub types: std::collections::BTreeMap<String, crate::skills::Usage>,
    /// Replies of Thrall's that started several helpers at once.
    #[serde(default)]
    pub side_by_side: u32,
    /// Replies that started just one.
    #[serde(default)]
    pub alone: u32,
}

/// The helpers' use so far (data folder `data`).
pub fn usage(data: &Path) -> HelperUsage {
    std::fs::read_to_string(data.join(HELPER_USAGE_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Count one reply of Thrall's that started helpers of these `types` (one per helper), `now`.
pub fn record_use(data: &Path, types: &[String], now: i64) {
    if types.is_empty() {
        return;
    }
    let mut all = usage(data);
    for t in types {
        let name = if t.trim().is_empty() {
            HELPER_NO_TYPE
        } else {
            t.trim()
        };
        let entry = all.types.entry(name.to_ascii_lowercase()).or_default();
        entry.uses = entry.uses.saturating_add(1);
        entry.last = now;
    }
    if types.len() > 1 {
        all.side_by_side = all.side_by_side.saturating_add(1);
    } else {
        all.alone = all.alone.saturating_add(1);
    }
    if let Ok(text) = serde_json::to_string_pretty(&all) {
        let _ = std::fs::write(data.join(HELPER_USAGE_FILE), text);
    }
}

/// The helper type called `name` for `workspace`, if there is one.
pub fn find(data_dir: Option<&Path>, workspace: &Path, name: &str) -> Option<AgentType> {
    discover(data_dir, workspace)
        .into_iter()
        .find(|a| a.name.eq_ignore_ascii_case(name.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_read_with_its_limits() {
        let a = parse(
            "---\nname: Tester\ndescription: Runs tests\ntools: read_file, shell ,\nread_only: no\nmodel: x/y\n---\nRun them.",
            "file",
            AgentSource::Personal,
        )
        .unwrap();
        assert_eq!(a.name, "tester");
        assert_eq!(a.description, "Runs tests");
        assert_eq!(
            a.tools,
            Some(vec!["read_file".to_owned(), "shell".to_owned()])
        );
        assert!(!a.read_only);
        assert_eq!(a.model.as_deref(), Some("x/y"));
        assert_eq!(a.prompt, "Run them.");
        let b = parse(
            "---\nread_only: yes\n---\nLook.",
            "looker",
            AgentSource::Project,
        )
        .unwrap();
        assert_eq!(b.name, "looker");
        assert!(b.read_only && b.tools.is_none() && b.model.is_none());
        assert!(parse("---\nname: x\n---\n", "x", AgentSource::Personal).is_none());
    }

    #[test]
    fn the_built_ins_are_there_and_only_the_coder_and_tester_can_change_things() {
        let all = built_in();
        let names: Vec<&str> = all.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "researcher",
                "coder",
                "reviewer",
                "planner",
                "tester",
                "explorer",
                "summariser"
            ]
        );
        for a in &all {
            assert_eq!(
                a.read_only,
                a.name != "coder" && a.name != "tester",
                "{}",
                a.name
            );
            assert!(
                a.description.starts_with("Use when") && a.description.contains("Not for"),
                "{} says when to use it and when not",
                a.name
            );
        }
        // Only the coder gets every tool; the tester can run things but not edit files.
        let tester = all.iter().find(|a| a.name == "tester").unwrap();
        let tools = tester.tools.as_ref().unwrap();
        assert!(tools.iter().any(|t| t == "shell"));
        assert!(!tools.iter().any(|t| t == "write_file" || t == "edit_file"));
        for a in &all {
            assert_eq!(a.tools.is_none(), a.name == "coder", "{}", a.name);
            assert_eq!(
                a.light,
                matches!(a.name.as_str(), "researcher" | "explorer" | "summariser"),
                "{}",
                a.name
            );
        }
    }

    #[test]
    fn helper_use_is_counted_with_how_often_they_work_side_by_side() {
        let dir = std::env::temp_dir().join(format!("thrall-helper-use-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(usage(&dir), HelperUsage::default());
        record_use(&dir, &["researcher".into(), "Explorer".into()], 10);
        record_use(&dir, &[String::new()], 20);
        record_use(&dir, &["researcher".into()], 30);
        record_use(&dir, &[], 40);
        let u = usage(&dir);
        assert_eq!(u.side_by_side, 1);
        assert_eq!(u.alone, 2);
        assert_eq!(u.types["researcher"].uses, 2);
        assert_eq!(u.types["researcher"].last, 30);
        assert_eq!(u.types["explorer"].uses, 1);
        assert_eq!(u.types[HELPER_NO_TYPE].uses, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_types_replace_personal_ones_which_replace_built_ins() {
        let base = std::env::temp_dir().join(format!("thrall-agents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let data = base.join("data");
        let ws = base.join("ws");
        let write = |dir: std::path::PathBuf, name: &str, body: &str| {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{name}.{AGENT_FILE_EXT}")), body).unwrap();
        };
        write(
            data.join(AGENTS_DIR),
            "coder",
            "---\ndescription: mine\n---\nMine.",
        );
        write(
            data.join(AGENTS_DIR),
            "tester",
            "---\ndescription: tests\n---\nTest.",
        );
        write(
            ws.join(PROJECT_DIR).join(AGENTS_DIR),
            "tester",
            "---\ndescription: project tests\n---\nProject.",
        );
        let all = discover(Some(&data), &ws);
        // The built-ins plus nothing new: the person's coder and tester replace Thrall's.
        assert_eq!(all.len(), built_in().len());
        let coder = all.iter().find(|a| a.name == "coder").unwrap();
        assert_eq!(coder.source, AgentSource::Personal);
        let tester = find(Some(&data), &ws, "Tester").unwrap();
        assert_eq!(tester.source, AgentSource::Project);
        assert_eq!(tester.prompt, "Project.");
        let _ = std::fs::remove_dir_all(&base);
    }
}
