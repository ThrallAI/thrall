//! Tools about the session itself: leaving plan mode, and restarting the engine.
//!
//! `exit_plan_mode` is handled by the agent (it shows the plan and waits for a decision); its
//! `run` is only reached outside plan mode.

use futures::future::BoxFuture;
use serde_json::{Value, json};
use thrall_gates::Effect;
use thrall_protocol::Risk;

use crate::constants::{SKILL_DESCRIPTION_MAX_CHARS, SKILL_FILE};
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str};

/// Name of the plan tool; the agent looks for it.
pub const EXIT_PLAN_MODE: &str = "exit_plan_mode";

/// Present a plan for approval (plan mode).
pub struct ExitPlanMode;

impl Tool for ExitPlanMode {
    fn name(&self) -> &'static str {
        EXIT_PLAN_MODE
    }
    fn description(&self) -> &'static str {
        "In plan mode: present your plan for the user to approve, as markdown (what you'll change, \
         in which files, and how you'll check it). Call this once you've looked around enough; \
         nothing can be changed until the plan is approved."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "plan": { "type": "string", "description": "The plan, in markdown" } },
            "required": ["plan"]
        })
    }
    fn title(&self, _args: &Value) -> String {
        "Presenting a plan".to_owned()
    }
    fn run<'a>(&'a self, _ctx: &'a ToolContext, _args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async {
            ToolOutput::ok(
                "Plan mode isn't on, so there's nothing to approve: go ahead with the work.",
                "Not in plan mode",
            )
        })
    }
}

/// Name of the hand-off tool; the agent runs it (a helper with its own context).
pub const DELEGATE: &str = "agent";

/// Hand a task to a helper with a fresh context. The agent carries it out; `run` is only reached
/// where helpers aren't available.
pub struct Delegate;

impl Tool for Delegate {
    fn name(&self) -> &'static str {
        DELEGATE
    }
    fn description(&self) -> &'static str {
        "Hand a self-contained task to a helper with a fresh context, and get its final answer. Use \
         it for research or work that would fill your context (searching a codebase, reading many \
         files, comparing sources). The helper has your tools (not this one) and the same \
         permissions; write `task` so it stands on its own. With background: true the helper works \
         read-only while you carry on, and its result comes later (agent_result)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "A few words for the user" },
                "task": { "type": "string", "description": "Full instructions for the helper" },
                "background": { "type": "boolean", "description": "Run read-only in the background" }
            },
            "required": ["title", "task"]
        })
    }
    fn title(&self, args: &Value) -> String {
        let bg = args
            .get("background")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        format!(
            "{} {}",
            if bg { "Background helper:" } else { "Helper:" },
            arg_str(args, "title")
        )
    }
    fn run<'a>(&'a self, _ctx: &'a ToolContext, _args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async {
            ToolOutput::error("helpers can't start other helpers; do this part yourself")
        })
    }
}

/// Look up a background helper's result.
pub struct AgentResult;

impl Tool for AgentResult {
    fn name(&self) -> &'static str {
        "agent_result"
    }
    fn description(&self) -> &'static str {
        "Get a background helper's status and, once it's done, its result (id from the agent tool)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Checking helper {}", arg_str(args, "id"))
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let id = arg_str(&args, "id").to_owned();
            match ctx.store.get_agent(&ctx.user_id, &id).await {
                Ok(Some(a)) if a.status == "running" => {
                    ToolOutput::ok(format!("“{}” is still working.", a.title), "Still working")
                }
                Ok(Some(a)) => {
                    let result = serde_json::from_str::<Value>(&a.state)
                        .ok()
                        .and_then(|v| v.get("result").and_then(Value::as_str).map(str::to_owned))
                        .unwrap_or(a.state);
                    ToolOutput::ok(
                        format!("“{}” ({}):\n\n{result}", a.title, a.status),
                        format!("Helper {}", a.status),
                    )
                }
                Ok(None) => ToolOutput::error(format!("there's no helper {id}")),
                Err(e) => ToolOutput::error(format!("couldn't look it up: {e}")),
            }
        })
    }
}

/// Name of the skill tool; the agent runs `run: helper` skills itself.
pub const USE_SKILL: &str = "skill";

/// Name of the tool that proposes a new skill.
pub const PROPOSE_SKILL: &str = "skill_propose";

/// A skill by name, if it's there and not turned off.
pub fn find_skill(ctx: &ToolContext, name: &str) -> Option<crate::skills::Skill> {
    crate::skills::discover(
        ctx.data_dir.as_deref(),
        &ctx.workspace,
        &ctx.disabled_skills,
    )
    .into_iter()
    .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
}

/// A skill's instructions as the model (or a helper) reads them, with its folder's other files.
pub fn skill_text(skill: &crate::skills::Skill) -> String {
    let mut out = format!("# Skill: {}\n\n{}", skill.name, skill.body);
    let extras: Vec<String> = std::fs::read_dir(&skill.dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().is_some_and(|n| n != SKILL_FILE))
                .map(|p| p.display().to_string())
                .collect()
        })
        .unwrap_or_default();
    if !extras.is_empty() {
        out.push_str("\n\nFiles in this skill's folder (read them as needed):\n");
        for path in extras {
            out.push_str(&format!("- {path}\n"));
        }
    }
    out
}

/// Load a skill's full instructions.
pub struct UseSkill;

impl Tool for UseSkill {
    fn name(&self) -> &'static str {
        USE_SKILL
    }
    fn description(&self) -> &'static str {
        "Use a skill by name (the available skills are listed in your instructions). Use a skill \
         whenever the task matches its description, and follow it. Say in `task` what it's for: \
         some skills run in a helper that only knows the task you give it."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "task": {
                    "type": "string",
                    "description": "What to do with it, with everything needed (file names, \
                                    the person's words)"
                }
            },
            "required": ["name"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Using the {} skill", arg_str(args, "name"))
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let wanted = arg_str(&args, "name").to_owned();
            match find_skill(ctx, &wanted) {
                Some(skill) => ToolOutput::ok(skill_text(&skill), format!("Loaded {}", skill.name)),
                None => ToolOutput::error(format!("there's no skill called {wanted}")),
            }
        })
    }
}

/// Offer the person a new skill: a recipe for something Thrall worked out, saved only if they
/// approve it.
pub struct ProposeSkill;

#[derive(serde::Deserialize)]
struct ProposeArgs {
    name: String,
    description: String,
    instructions: String,
    #[serde(default)]
    learned_from: String,
}

/// What saving a learned skill means: where it goes, its file, its rating and why.
struct Prepared {
    dir: std::path::PathBuf,
    text: String,
    rating: crate::skills::Rating,
    why: Vec<String>,
}

impl ProposeSkill {
    /// The folder, file text and why it can't be saved (if it can't).
    fn prepare(ctx: &ToolContext, args: &Value) -> Result<Prepared, String> {
        let a: ProposeArgs = serde_json::from_value(args.clone())
            .map_err(|e| format!("the arguments for {PROPOSE_SKILL} weren't right: {e}"))?;
        let data = ctx
            .data_dir
            .as_deref()
            .ok_or("this Thrall has no data folder to keep skills in")?;
        let slug = crate::skills::slug(&a.name)
            .ok_or("give the skill a short name made of letters, digits and dashes")?;
        let description = a.description.trim();
        if description.is_empty() || description.chars().count() > SKILL_DESCRIPTION_MAX_CHARS {
            return Err(format!(
                "the description must say in one sentence when to use the skill (at most \
                 {SKILL_DESCRIPTION_MAX_CHARS} characters)"
            ));
        }
        if a.instructions.trim().is_empty() {
            return Err("the skill needs instructions".to_owned());
        }
        let taken = crate::skills::discover_all(Some(data), &ctx.workspace)
            .into_iter()
            .any(|s| s.name.eq_ignore_ascii_case(&slug));
        if taken {
            return Err(format!(
                "there's already a skill called {slug}; pick another name"
            ));
        }
        let (rating, why) = crate::skills::rate(&a.learned_from, &a.instructions);
        let text =
            crate::skills::skill_file(&slug, description, true, Some(rating), &a.instructions);
        Ok(Prepared {
            dir: crate::skills::personal_dir(data, &slug),
            text,
            rating,
            why,
        })
    }
}

impl Tool for ProposeSkill {
    fn name(&self) -> &'static str {
        PROPOSE_SKILL
    }
    fn description(&self) -> &'static str {
        "Learn a new skill: step-by-step instructions for something you had to work out that's \
         likely to come up again (a procedure, a workflow, a fix that took several tries). It's \
         saved straight away with a safety rating; a skill rated High (learned from the web, or \
         one that deletes, sends, or handles keys) is put to the user first. Say where it came \
         from in learned_from. Don't save one for one-off or trivial tasks, or after the user \
         turned it down."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Short, e.g. \"release-notes\"" },
                "description": {
                    "type": "string",
                    "description": "One sentence: when to use it"
                },
                "instructions": {
                    "type": "string",
                    "description": "Markdown steps someone new could follow, with the details \
                                    that mattered. Commands only as you actually ran them and \
                                    saw them work, never untried variations"
                },
                "learned_from": {
                    "type": "string",
                    "enum": ["user", "my_work", "files", "web"],
                    "description": "Where the knowledge came from: the user taught you, your own work this session, the user's files, or the web / a download / a tool's output"
                }
            },
            "required": ["name", "description", "instructions", "learned_from"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Proposing the {} skill", arg_str(args, "name"))
    }
    fn precheck(&self, ctx: &ToolContext, args: &Value) -> Option<String> {
        Self::prepare(ctx, args).err()
    }
    /// Low and medium skills are saved without asking; a high one is put to the person first,
    /// with why it's rated high.
    fn effect(&self, ctx: &ToolContext, args: &Value) -> Effect {
        match Self::prepare(ctx, args) {
            Ok(p) if p.rating == crate::skills::Rating::High => {
                let name = p
                    .dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let shown = format!("skills/{name}/{SKILL_FILE}");
                Effect::AlwaysAsk {
                    summary: format!(
                        "Keep and turn on a new skill, “{name}”? It's rated High: {}. You can \
                         change or remove it later in Settings → Learning.",
                        p.why.join(", ")
                    ),
                    risk: Risk::High,
                    diff: Some(crate::diff::make(&shown, None, &p.text)),
                }
            }
            _ => Effect::Read,
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let p = match Self::prepare(ctx, &args) {
                Ok(p) => p,
                Err(why) => return ToolOutput::error(why),
            };
            if let Err(e) = tokio::fs::create_dir_all(&p.dir).await {
                return ToolOutput::error(format!("couldn't make the skill's folder: {e}"));
            }
            if let Err(e) = tokio::fs::write(p.dir.join(SKILL_FILE), &p.text).await {
                return ToolOutput::error(format!("couldn't save the skill: {e}"));
            }
            let name = p
                .dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let rating = p.rating.as_str();
            ToolOutput::ok(
                format!(
                    "Learned the {name} skill (rated {rating}). The user can run it with /{name}, \
                     and it's listed for you from the next message on. Tell the user in one line \
                     that you learned it, and that they can see, change or remove what you learn \
                     in Settings → Learning."
                ),
                format!("Learned skill {name} ({rating})"),
            )
        })
    }
}
/// Restart Thrall's engine (reload the config, reconnect), keeping the session.
pub struct Restart;

impl Tool for Restart {
    fn name(&self) -> &'static str {
        "thrall_restart"
    }
    fn description(&self) -> &'static str {
        "Restart Thrall's engine: reloads config.toml and reconnects, keeping this session. Use \
         after changing Thrall's own settings. Always asks the user first."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "reason": { "type": "string", "description": "Why, in a few words" } },
            "required": ["reason"]
        })
    }
    fn title(&self, _args: &Value) -> String {
        "Restarting Thrall".to_owned()
    }
    fn effect(&self, _ctx: &ToolContext, args: &Value) -> Effect {
        Effect::AlwaysAsk {
            summary: format!("Restart Thrall ({})", arg_str(args, "reason")),
            risk: Risk::Medium,
            diff: None,
        }
    }
    fn run<'a>(&'a self, _ctx: &'a ToolContext, _args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async {
            let mut out = ToolOutput::ok(
                "Restart approved. Thrall restarts when this turn ends; finish your reply now.",
                "Restart after this turn",
            );
            out.restart = true;
            out
        })
    }
}
