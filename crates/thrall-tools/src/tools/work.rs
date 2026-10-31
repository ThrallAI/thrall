//! The task list (todo_write) and memory (memory_save, memory_search).

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use thrall_core::constants::id_prefix;
use thrall_core::ids::{new_id, now_ms};
use thrall_protocol::{TaskItem, TaskStatus};
use thrall_store::TaskRow;

use crate::constants::MEMORY_SEARCH_LIMIT;
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str, parse_args};

// ---------- todo_write ----------

/// Replace the session's visible task list.
pub struct TodoWrite;

#[derive(Deserialize)]
struct TodoArgs {
    // Other names models use for the list.
    #[serde(alias = "todos", alias = "tasks")]
    items: Vec<TodoItem>,
}

#[derive(Deserialize)]
struct TodoItem {
    #[serde(alias = "content", alias = "task", alias = "title")]
    text: String,
    status: TaskStatus,
}

fn status_str(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Pending => "pending",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Done => "done",
    }
}

impl Tool for TodoWrite {
    fn name(&self) -> &'static str {
        "todo_write"
    }
    fn description(&self) -> &'static str {
        "Set the task list the user sees for work of three or more steps (not for quick one- or \
         two-step jobs). Send the whole list every time. \
         Keep exactly one task in_progress while working; mark tasks done as soon as they are."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "done"] }
                        },
                        "required": ["text", "status"]
                    }
                }
            },
            "required": ["items"]
        })
    }
    fn title(&self, _args: &Value) -> String {
        "Updating the task list".to_owned()
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: TodoArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let items: Vec<TaskItem> = a
                .items
                .into_iter()
                .map(|i| TaskItem {
                    id: new_id(id_prefix::TASK),
                    text: i.text,
                    status: i.status,
                })
                .collect();
            let rows: Vec<TaskRow> = items
                .iter()
                .map(|i| TaskRow {
                    id: i.id.clone(),
                    text: i.text.clone(),
                    status: status_str(i.status).to_owned(),
                })
                .collect();
            if let Err(e) = ctx
                .store
                .set_tasks(&ctx.user_id, &ctx.session_id, &rows, now_ms())
                .await
            {
                return ToolOutput::error(format!("couldn't save the task list: {e}"));
            }
            let done = items
                .iter()
                .filter(|i| i.status == TaskStatus::Done)
                .count();
            let mut out = ToolOutput::ok(
                "Task list updated.",
                format!("{done} of {} done", items.len()),
            );
            out.tasks = Some(items);
            out
        })
    }
}

// ---------- memory_save ----------

/// Name of the tool that saves a fact; the agent asks first when saving without asking is off.
pub const MEMORY_SAVE: &str = "memory_save";

/// Remember a fact for later sessions.
pub struct MemorySave;

#[derive(Deserialize)]
struct SaveArgs {
    topic: String,
    content: String,
    #[serde(default)]
    replaces: Vec<String>,
}

impl Tool for MemorySave {
    fn name(&self) -> &'static str {
        MEMORY_SAVE
    }
    fn description(&self) -> &'static str {
        "Remember a lasting fact about the user or their work (preferences, decisions, setup) so \
         later sessions know it. One fact per call, with a short topic. When a fact changes, save \
         the new version under the SAME topic (or pass the old fact's id in `replaces`): the old \
         one is then replaced rather than kept beside it."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "topic": { "type": "string", "description": "A few words, e.g. \"editor\"" },
                "content": { "type": "string", "description": "The fact" },
                "replaces": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Ids of facts this one replaces (ids come from memory_search)"
                }
            },
            "required": ["topic", "content"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Remembering {}", arg_str(args, "topic"))
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: SaveArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            // The memory gate: secrets never reach long-term memory.
            if let Err(why) = thrall_gates::memory_gate(&format!("{}\n{}", a.topic, a.content)) {
                return ToolOutput::error(why);
            }
            if let Some(memory) = &ctx.memory {
                // Facts on the same topic are superseded: the newest is what Thrall recalls.
                let saved = match memory
                    .save_fact_replacing(&ctx.user_id, &a.topic, &a.content, &a.replaces)
                    .await
                {
                    Ok(saved) if saved.replaced.is_empty() => {
                        let mut out = ToolOutput::ok(
                            format!("Saved ({}).", saved.id),
                            format!("Saved “{}”", a.topic),
                        );
                        out.remembered = Some(fact(&saved.id, &a));
                        out
                    }
                    Ok(saved) => {
                        let mut out = ToolOutput::ok(
                            format!(
                                "Saved ({}); it replaces {} earlier fact(s) on this topic, which \
                                 are no longer recalled.",
                                saved.id,
                                saved.replaced.len()
                            ),
                            format!("Updated “{}”", a.topic),
                        );
                        out.remembered = Some(fact(&saved.id, &a));
                        out
                    }
                    Err(e) => ToolOutput::error(format!("couldn't save: {e}")),
                };
                return saved;
            }
            let id = new_id(id_prefix::KNOWLEDGE);
            match ctx
                .store
                .save_knowledge(&ctx.user_id, &id, &a.topic, &a.content, now_ms())
                .await
            {
                Ok(()) => {
                    let mut out =
                        ToolOutput::ok(format!("Saved ({id})."), format!("Saved “{}”", a.topic));
                    out.remembered = Some(fact(&id, &a));
                    out
                }
                Err(e) => ToolOutput::error(format!("couldn't save: {e}")),
            }
        })
    }
}

/// What the person is told about a saved fact.
fn fact(id: &str, a: &SaveArgs) -> crate::tool::SavedFact {
    crate::tool::SavedFact {
        id: id.to_owned(),
        topic: a.topic.clone(),
        content: a.content.clone(),
    }
}

// ---------- memory_search ----------

/// Look up remembered facts.
pub struct MemorySearch;

#[derive(Deserialize)]
struct SearchArgs {
    query: String,
}

impl Tool for MemorySearch {
    fn name(&self) -> &'static str {
        "memory_search"
    }
    fn description(&self) -> &'static str {
        "Search long-term memory: facts you saved and earlier conversations, by words and by \
         meaning. Use it when the user refers to something from before (\"like last time\", \"my \
         usual setup\") or before asking them something you may already know."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Recalling {}", arg_str(args, "query"))
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: SearchArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            if let Some(memory) = &ctx.memory {
                return match memory
                    .search(&ctx.user_id, &a.query, MEMORY_SEARCH_LIMIT as usize)
                    .await
                {
                    Ok(hits) if hits.is_empty() => {
                        ToolOutput::ok("Nothing remembered about that.", "Nothing found")
                    }
                    Ok(hits) => {
                        let facts = hits
                            .iter()
                            .filter(|h| h.kind == thrall_store::MemoryKind::Knowledge)
                            .count();
                        let out = hits
                            .iter()
                            .map(|h| match h.kind {
                                thrall_store::MemoryKind::Knowledge => {
                                    format!("- Fact [{}] ({}): {}", h.title, h.id, h.text)
                                }
                                thrall_store::MemoryKind::Episode => format!(
                                    "- Earlier conversation ({}), “{}”:\n  {}",
                                    crate::tools::schedule::local_time(h.at),
                                    h.title,
                                    h.text.replace('\n', "\n  ")
                                ),
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        ToolOutput::ok(
                            out,
                            format!("{facts} facts, {} conversations", hits.len() - facts),
                        )
                    }
                    Err(e) => ToolOutput::error(format!("couldn't search memory: {e}")),
                };
            }
            match ctx
                .store
                .search_knowledge(&ctx.user_id, &a.query, MEMORY_SEARCH_LIMIT)
                .await
            {
                Ok(hits) if hits.is_empty() => {
                    ToolOutput::ok("Nothing remembered about that.", "Nothing found")
                }
                Ok(hits) => {
                    let out = hits
                        .iter()
                        .map(|k| format!("- [{}] {}", k.topic, k.content))
                        .collect::<Vec<_>>()
                        .join("\n");
                    ToolOutput::ok(out, format!("{} facts", hits.len()))
                }
                Err(e) => ToolOutput::error(format!("couldn't search memory: {e}")),
            }
        })
    }
}
