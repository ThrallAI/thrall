//! `helper_propose`: Thrall offers the person a new helper type (`agents/<name>.md` in their data
//! folder), saved only when they say yes.

use futures::future::BoxFuture;
use serde_json::{Value, json};
use thrall_gates::Effect;
use thrall_protocol::Risk;

use crate::constants::{
    AGENT_FILE_EXT, AGENTS_DIR, HELPER_DESCRIPTION_MAX_CHARS, HELPER_DESCRIPTION_START,
    HELPER_NAME_MAX_WORDS, HELPER_NOT_FOR,
};
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str};

/// The tool's name.
pub const PROPOSE_HELPER: &str = "helper_propose";

/// Offer the person a new helper type.
pub struct ProposeHelper;

#[derive(serde::Deserialize)]
struct Args {
    name: String,
    description: String,
    not_for: String,
    instructions: String,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    read_only: bool,
    #[serde(default)]
    light: bool,
    #[serde(default)]
    replaces: bool,
}

/// What saving it means: the file, its text, and the text it replaces (if any).
#[derive(Debug)]
struct Prepared {
    name: String,
    path: std::path::PathBuf,
    text: String,
    old: Option<String>,
}

/// One line, no line breaks (front matter is one value per line).
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl ProposeHelper {
    fn prepare(data: Option<&std::path::Path>, args: &Value) -> Result<Prepared, String> {
        let a: Args = serde_json::from_value(args.clone())
            .map_err(|e| format!("the arguments for {PROPOSE_HELPER} weren't right: {e}"))?;
        let data = data.ok_or("this Thrall has no data folder to keep helper types in")?;
        let name = crate::skills::slug(&a.name)
            .ok_or("give the helper a short name made of letters, digits and dashes")?;
        if name.split('-').count() > HELPER_NAME_MAX_WORDS {
            return Err(format!(
                "name the helper by its role in 1 to {HELPER_NAME_MAX_WORDS} words (\"tester\", \
                 \"data-cleaner\"), not \"{name}\""
            ));
        }
        let description = one_line(&a.description);
        if !description
            .to_lowercase()
            .starts_with(&HELPER_DESCRIPTION_START.to_lowercase())
        {
            return Err(format!(
                "start the description with \"{HELPER_DESCRIPTION_START}\": the kind of work it's for"
            ));
        }
        let not_for = one_line(&a.not_for);
        if not_for.is_empty() {
            return Err("say in not_for what it's NOT for (the look-alike work)".to_owned());
        }
        let full = format!(
            "{} {HELPER_NOT_FOR} {}",
            description.trim_end_matches('.').to_owned() + ".",
            not_for
                .trim_start_matches(HELPER_NOT_FOR)
                .trim()
                .trim_end_matches('.')
                .to_owned()
                + "."
        );
        if full.chars().count() > HELPER_DESCRIPTION_MAX_CHARS {
            return Err(format!(
                "keep description and not_for short (at most {HELPER_DESCRIPTION_MAX_CHARS} \
                 characters together)"
            ));
        }
        if a.instructions.trim().is_empty() {
            return Err("give the helper its instructions".to_owned());
        }
        // Only tools a helper can have.
        let known: Vec<String> = crate::tools::core()
            .iter()
            .map(|t| t.name().to_owned())
            .chain([
                crate::tools::MCP_SEARCH.to_owned(),
                crate::tools::MCP_CALL.to_owned(),
            ])
            .filter(|t| !crate::tools::NOT_FOR_HELPERS.contains(&t.as_str()))
            .collect();
        let tools: Vec<String> = a
            .tools
            .iter()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect();
        if let Some(bad) = tools.iter().find(|t| !known.contains(t)) {
            return Err(format!(
                "a helper can't have the tool {bad}; pick from: {}",
                known.join(", ")
            ));
        }
        let taken = crate::agents::personal(Some(data))
            .into_iter()
            .any(|t| t.name == name);
        let path = data
            .join(AGENTS_DIR)
            .join(format!("{name}.{AGENT_FILE_EXT}"));
        if taken && !a.replaces {
            return Err(format!(
                "there's already a helper type called {name}; pick another name, or set \
                 replaces: true to change it"
            ));
        }
        let old = std::fs::read_to_string(&path).ok();
        let yes_no = |b: bool| if b { "yes" } else { "no" };
        let mut text = format!("---\nname: {name}\ndescription: {full}\n");
        if !tools.is_empty() {
            text.push_str(&format!("tools: {}\n", tools.join(", ")));
        }
        text.push_str(&format!(
            "read_only: {}\nlight: {}\n---\n{}\n",
            yes_no(a.read_only),
            yes_no(a.light),
            a.instructions.trim()
        ));
        Ok(Prepared {
            name,
            path,
            text,
            old,
        })
    }
}

impl Tool for ProposeHelper {
    fn name(&self) -> &'static str {
        PROPOSE_HELPER
    }
    fn description(&self) -> &'static str {
        "Offer the user a new helper type: a kind of helper you'd start often for work none of \
         your helper types fits (its role, instructions, the only tools it gets, whether it only \
         looks, and whether reading-and-summing-up work lets it run on the cheaper helper model). \
         Always put to the user first. Only when the same kind of work keeps coming up; never for \
         one task, and never a near-copy of a type you have."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Its role in 1 to 3 words: \"tester\", \"data-cleaner\", \"translator\""
                },
                "description": {
                    "type": "string",
                    "description": "One sentence starting \"Use when\": the kind of work it's for"
                },
                "not_for": {
                    "type": "string",
                    "description": "The look-alike work it's NOT for, in a few words"
                },
                "instructions": {
                    "type": "string",
                    "description": "What it's told before its task: how to work and what to hand back"
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The only tools it gets (leave out for all helper tools). Give it only what the work needs"
                },
                "read_only": {
                    "type": "boolean",
                    "description": "It only looks and never changes anything"
                },
                "light": {
                    "type": "boolean",
                    "description": "Reading, searching or summing up only: runs on the helper model when one is set"
                },
                "replaces": {
                    "type": "boolean",
                    "description": "Change the helper type of this name the user already has"
                }
            },
            "required": ["name", "description", "not_for", "instructions"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Offering a {} helper", arg_str(args, "name"))
    }
    fn precheck(&self, ctx: &ToolContext, args: &Value) -> Option<String> {
        Self::prepare(ctx.data_dir.as_deref(), args).err()
    }
    fn effect(&self, ctx: &ToolContext, args: &Value) -> Effect {
        match Self::prepare(ctx.data_dir.as_deref(), args) {
            Ok(p) => {
                let shown = format!("{AGENTS_DIR}/{}.{AGENT_FILE_EXT}", p.name);
                let what = if p.old.is_some() {
                    "Change the helper type"
                } else {
                    "Add a helper type"
                };
                Effect::AlwaysAsk {
                    summary: format!(
                        "{what} “{}”? Thrall can then start it for this kind of work. You can \
                         change or remove it later in Settings → Helpers.",
                        p.name
                    ),
                    risk: Risk::Medium,
                    diff: Some(crate::diff::make(&shown, p.old.as_deref(), &p.text)),
                }
            }
            Err(_) => Effect::Read,
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let p = match Self::prepare(ctx.data_dir.as_deref(), &args) {
                Ok(p) => p,
                Err(why) => return ToolOutput::error(why),
            };
            if let Some(dir) = p.path.parent()
                && let Err(e) = tokio::fs::create_dir_all(dir).await
            {
                return ToolOutput::error(format!("couldn't make the helpers folder: {e}"));
            }
            if let Err(e) = tokio::fs::write(&p.path, &p.text).await {
                return ToolOutput::error(format!("couldn't save the helper type: {e}"));
            }
            let name = p.name;
            ToolOutput::ok(
                format!(
                    "Saved the {name} helper type. It's in your helper types from the next \
                     message on (agent type: {name}). Tell the user in one line."
                ),
                format!("Added helper type {name}"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(name: &str, tools: &[&str]) -> Value {
        json!({
            "name": name,
            "description": "Use when a CSV needs cleaning up",
            "not_for": "Not for making charts",
            "instructions": "Clean it.",
            "tools": tools,
            "light": true,
        })
    }

    #[test]
    fn a_helper_type_is_named_by_role_says_when_not_and_gets_only_real_tools() {
        let dir =
            std::env::temp_dir().join(format!("thrall-helper-propose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let c = Some(dir.as_path());
        let p =
            ProposeHelper::prepare(c, &args("data-cleaner", &["read_file", "write_file"])).unwrap();
        assert!(
            p.text
                .contains("description: Use when a CSV needs cleaning up. Not for making charts.")
        );
        assert!(p.text.contains("tools: read_file, write_file"));
        assert!(p.text.contains("light: yes"));
        let long = ProposeHelper::prepare(c, &args("cleans-up-the-csv-files", &[]));
        assert!(long.unwrap_err().contains("role"));
        let bad = ProposeHelper::prepare(c, &args("cleaner", &["agent"]));
        assert!(bad.unwrap_err().contains("can't have the tool agent"));
        let mut no_use = args("cleaner", &[]);
        no_use["description"] = json!("Cleans CSVs");
        assert!(ProposeHelper::prepare(c, &no_use).is_err());
        // A built-in's name needs replaces: true.
        let taken = ProposeHelper::prepare(c, &args("tester", &[]));
        assert!(taken.unwrap_err().contains("already a helper type"));
        let mut replace = args("tester", &[]);
        replace["replaces"] = json!(true);
        assert!(ProposeHelper::prepare(c, &replace).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
