//! Helper types: what a helper is for, how it works and what it may use. Thrall ships four
//! (researcher, coder, reviewer, planner); a person adds their own as `agents/<name>.md` in their
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
//! Thrall's).

use std::path::Path;

use crate::constants::{AGENT_FILE_EXT, AGENT_TYPES, AGENTS_DIR, PROJECT_DIR};
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
    /// Its model; none: Thrall's.
    pub model: Option<String>,
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
        .map(|(name, description, read_only, prompt)| AgentType {
            name: (*name).to_owned(),
            description: (*description).to_owned(),
            tools: None,
            read_only: *read_only,
            model: None,
            prompt: (*prompt).to_owned(),
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
    fn the_four_built_ins_are_there_and_only_the_coder_changes_things() {
        let all = built_in();
        let names: Vec<&str> = all.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["researcher", "coder", "reviewer", "planner"]);
        for a in &all {
            assert_eq!(a.read_only, a.name != "coder", "{}", a.name);
        }
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
        assert_eq!(all.len(), 5);
        let coder = all.iter().find(|a| a.name == "coder").unwrap();
        assert_eq!(coder.source, AgentSource::Personal);
        let tester = find(Some(&data), &ws, "Tester").unwrap();
        assert_eq!(tester.source, AgentSource::Project);
        assert_eq!(tester.prompt, "Project.");
        let _ = std::fs::remove_dir_all(&base);
    }
}
