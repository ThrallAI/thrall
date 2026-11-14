//! Slash commands the engine runs (protocol §10). Clients send the name and arguments; the engine
//! decides what it means and answers in markdown. Commands about the client itself (quit, new
//! session, resume) belong to the client.

use thrall_core::ids::now_ms;
use thrall_gates::Mode;

use crate::constants::{COST_BALANCE_LINE, COST_BALANCE_UNAVAILABLE, MEMORY_COMMAND_LIMIT};
use crate::error::AgentError;
use crate::session;
use crate::state::AppState;

/// A command, for `/help` and autocomplete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInfo {
    /// Name without the slash.
    pub name: &'static str,
    /// What it does.
    pub summary: &'static str,
    /// Arguments, for the hint after the name.
    pub args_hint: &'static str,
}

/// The engine's commands.
pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "help",
        summary: "List commands",
        args_hint: "",
    },
    CommandInfo {
        name: "cost",
        summary: "Tokens and money spent: this session and all time",
        args_hint: "",
    },
    CommandInfo {
        name: "model",
        summary: "Show the model and profiles, or switch (an id or @profile) until restart",
        args_hint: "[model id | @profile]",
    },
    CommandInfo {
        name: "login",
        summary: "Sign in with your ChatGPT plan (chat models named chatgpt/<model>)",
        args_hint: "chatgpt",
    },
    CommandInfo {
        name: "logout",
        summary: "Forget the ChatGPT sign-in",
        args_hint: "chatgpt",
    },
    CommandInfo {
        name: "mcp",
        summary: "MCP servers; sign in to or out of one on the web",
        args_hint: "[login | logout <server>]",
    },
    CommandInfo {
        name: "title",
        summary: "Rename this session",
        args_hint: "<new title>",
    },
    CommandInfo {
        name: "folder",
        summary: "Show the folder this session works in, or move it to another",
        args_hint: "[path]",
    },
    CommandInfo {
        name: "tasks",
        summary: "Show this session's task list",
        args_hint: "",
    },
    CommandInfo {
        name: "memory",
        summary: "Search what Thrall remembers",
        args_hint: "<words>",
    },
    CommandInfo {
        name: "status",
        summary: "Model, session, folder and usage at a glance",
        args_hint: "",
    },
    CommandInfo {
        name: "compact",
        summary: "Summarise the earlier conversation now to free up room",
        args_hint: "",
    },
    CommandInfo {
        name: "jobs",
        summary: "Scheduled jobs; remove one with /jobs remove <id>",
        args_hint: "[remove <id>]",
    },
    CommandInfo {
        name: "mode",
        summary: "Show or set the permission mode: ask, auto, plan, full (full 30m, full on)",
        args_hint: "[mode] [30m | 2h | on]",
    },
];

/// Built-in commands that go to Thrall as a message (clients send `/name …` with `turn.send`):
/// Thrall itself carries them out, like the person's own skills.
pub const MESSAGE_COMMANDS: &[CommandInfo] = &[CommandInfo {
    name: crate::constants::HUMANISE_COMMAND,
    summary: "Rewrite text or a file so it reads like a person wrote it",
    args_hint: "<text or file>",
}];

/// Run `/name args` for `session_id`; the reply is markdown.
pub async fn run(
    state: &AppState,
    session_id: &str,
    name: &str,
    args: &str,
) -> Result<String, AgentError> {
    let args = args.trim();
    match name {
        "help" => Ok(help()),
        "cost" => cost(state, session_id).await,
        "model" => Ok(model(state, args).await),
        "title" => title(state, session_id, args).await,
        "folder" => folder(state, session_id, args).await,
        "tasks" => tasks(state, session_id).await,
        "memory" => memory(state, args).await,
        "status" => status(state, session_id).await,
        "mode" => mode(state, session_id, args).await,
        "jobs" => jobs(state, args).await,
        "compact" => compact(state, session_id).await,
        "mcp" => Ok(mcp(state)),
        other => Err(AgentError::UnknownCommand(other.to_owned())),
    }
}

/// A web address with any query left out (it can hold a key).
pub fn address_without_query(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

/// `/mcp`: the MCP servers and, for web ones, whether you're signed in. (Signing in and out is
/// the hub's: it waits for the browser.)
fn mcp(state: &AppState) -> String {
    let names = state.mcp.names();
    if names.is_empty() {
        return "No MCP servers yet. Add one under `[mcp.servers.<name>]` in config.toml: \
                `command = \"…\"` for a program on this computer, or `url = \"https://…\"` for a \
                server on the web."
            .to_owned();
    }
    let mut out = String::from("**MCP servers** (started when Thrall needs them)\n\n");
    for name in names {
        let Some(server) = state.mcp.server(&name) else {
            continue;
        };
        let line = if server.is_remote() {
            let signed = if thrall_mcp::remote::signed_in(&name) {
                " · signed in"
            } else {
                ""
            };
            format!(
                "- **{name}** · on the web, `{}`{signed}\n",
                address_without_query(&server.url)
            )
        } else {
            format!("- **{name}** · on this computer, `{}`\n", server.command)
        };
        out.push_str(&line);
    }
    out.push_str(
        "\nSign in to a web server with `/mcp login <name>` (only for servers that ask), and out \
         with `/mcp logout <name>`.",
    );
    out
}

async fn compact(state: &AppState, session_id: &str) -> Result<String, AgentError> {
    if state.is_running(session_id) {
        return Ok("Thrall is working in this session; try /compact when it's done.".to_owned());
    }
    match crate::compaction::compact(state, session_id, "", 0, true)
        .await?
        .compacted
    {
        Some(done) => Ok(format!(
            "Summarised the earlier {} messages. The full history is still there to scroll \
             back through; only what Thrall reads got shorter.",
            done.messages
        )),
        None => Ok("Nothing to summarise yet: the conversation is still short.".to_owned()),
    }
}

async fn jobs(state: &AppState, args: &str) -> Result<String, AgentError> {
    if let Some(id) = args
        .strip_prefix("remove")
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok(if state.store.delete_job(&state.user_id, id).await? {
            format!("Deleted job `{id}`.")
        } else {
            format!("There's no job `{id}`.")
        });
    }
    let jobs = state.store.list_jobs(&state.user_id).await?;
    if jobs.is_empty() {
        return Ok(
            "No scheduled jobs. Ask Thrall to schedule one, e.g. *every weekday at 8, ".to_owned()
                + "summarise the news on Rust*.",
        );
    }
    let mut out = String::from("**Scheduled jobs** (they run while Thrall is open)\n\n");
    for j in jobs {
        out.push_str(&format!(
            "- `{}` **{}** · `{}` · next {}{}\n",
            j.id,
            j.title,
            j.schedule,
            thrall_tools::tools::schedule::local_time(j.next_run_at),
            if j.enabled { "" } else { " (off)" }
        ));
    }
    Ok(out)
}

async fn mode(state: &AppState, session_id: &str, args: &str) -> Result<String, AgentError> {
    let current = session::mode_of(&session::get(state, session_id).await?);
    if args.is_empty() {
        return Ok(format!(
            "Mode: **{}**\n\n- `ask`: ask before edits and commands\n- `auto`: edits go ahead, \
             commands ask\n- `plan`: read-only until you approve a plan\n- `full`: no questions \
             (restarts still ask) until the task ends; `full 30m` or `full 2h` for a while, \
             `full on` until you switch it off\n\nShift+Tab switches between ask, auto and plan.",
            current.label()
        ));
    }
    let Some(next) = Mode::parse(args) else {
        return Ok(format!(
            "There's no `{args}` mode. Use ask, auto, plan or full (`full 30m`, `full 2h`, \
             `full on`)."
        ));
    };
    session::set_mode(state, session_id, next).await?;
    mode_changed(next)
}

/// The answer to `/mode <mode>` once the mode is changed: what the new mode covers.
pub fn mode_changed(mode: Mode) -> Result<String, AgentError> {
    Ok(format!(
        "Mode is now **{}**: {}",
        mode.label(),
        mode.covers()
    ))
}

fn help() -> String {
    let mut out = String::from("**Commands**\n\n");
    for c in COMMANDS.iter().chain(MESSAGE_COMMANDS) {
        let hint = if c.args_hint.is_empty() {
            String::new()
        } else {
            format!(" {}", c.args_hint)
        };
        out.push_str(&format!("- `/{}{hint}`: {}\n", c.name, c.summary));
    }
    out
}

fn money(usd: f64) -> String {
    if usd < 0.01 {
        format!("${usd:.4}")
    } else {
        format!("${usd:.2}")
    }
}

async fn cost(state: &AppState, session_id: &str) -> Result<String, AgentError> {
    let here = state
        .store
        .usage_totals(&state.user_id, Some(session_id))
        .await?;
    let all = state.store.usage_totals(&state.user_id, None).await?;
    let mut out = format!(
        "**This session:** {} · {} in · {} out\n\n**All time:** {} · {} in · {} out",
        money(here.cost_usd),
        here.input_tokens,
        here.output_tokens,
        money(all.cost_usd),
        all.input_tokens,
        all.output_tokens
    );
    // Only when the model spends OpenRouter credit; ChatGPT plans, local models and own keys don't.
    if thrall_llm::Routed::goes_to_main(&state.model()) {
        let line = match state.llm.credit_left().await {
            Some(left) => COST_BALANCE_LINE
                .replace("{provider}", state.llm.name())
                .replace("{left}", &money(left.max(0.0))),
            None => COST_BALANCE_UNAVAILABLE.replace("{provider}", state.llm.name()),
        };
        out.push_str("\n\n");
        out.push_str(&line);
    }
    Ok(out)
}

async fn model(state: &AppState, args: &str) -> String {
    let profiles = &state.config.profiles;
    if args.is_empty() {
        let mut out = format!(
            "Model: `{}`\n\nSwitch with `/model <id>` (an OpenRouter id like \
             `xiaomi/mimo-v2.5-pro`, `chatgpt/<model>` on your ChatGPT plan after \
             `/login chatgpt`, or `local/<model>` on Ollama or LM Studio, free), or \
             `/model @<profile>`. To change it for good, use Settings → \
             Models.",
            state.model()
        );
        match state.llm.plan_models().await {
            Ok(models) if !models.is_empty() => {
                out.push_str("\n\n**On your ChatGPT plan**\n\n");
                for m in models {
                    out.push_str(&format!("- `{m}`\n"));
                }
            }
            Ok(_) => {}
            Err(e) => out.push_str(&format!(
                "\n\n(Couldn't list your ChatGPT plan's models: {e})"
            )),
        }
        if !profiles.is_empty() {
            out.push_str("\n\n**Profiles**\n\n");
            for (name, p) in profiles {
                let about = if p.description.is_empty() {
                    String::new()
                } else {
                    format!(": {}", p.description)
                };
                out.push_str(&format!("- `@{name}` → `{}`{about}\n", p.model));
            }
        }
        return out;
    }
    if let Some(name) = args.strip_prefix('@') {
        return match profiles.get(name.trim()) {
            Some(p) => {
                state.set_model(&p.model);
                format!(
                    "Switched to **@{}** (`{}`) until Thrall restarts.",
                    name.trim(),
                    p.model
                )
            }
            None if profiles.is_empty() => {
                format!("There's no profile `@{name}`. Add profiles in Settings → Models.")
            }
            None => format!(
                "There's no profile `@{name}`. You have: {}.",
                profiles
                    .keys()
                    .map(|k| format!("`@{k}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
    }
    state.set_model(args);
    format!("Switched to `{args}` until Thrall restarts.")
}

async fn title(state: &AppState, session_id: &str, args: &str) -> Result<String, AgentError> {
    if !args.is_empty() && session::get(state, session_id).await?.home_day.is_some() {
        return Ok(AgentError::HomeIsBuiltIn.to_string());
    }
    if args.is_empty() {
        let s = session::get(state, session_id).await?;
        return Ok(format!(
            "Title: **{}**\n\nRename with `/title <new title>`.",
            s.title
        ));
    }
    state
        .store
        .set_session_title(&state.user_id, session_id, args)
        .await?;
    Ok(format!("Renamed to **{args}**."))
}

async fn folder(state: &AppState, session_id: &str, args: &str) -> Result<String, AgentError> {
    let s = session::get(state, session_id).await?;
    if args.is_empty() {
        let shown = session::folder_of(&s).display().to_string();
        let note = if s.workspace.is_none() {
            " (none picked, so the folder Thrall was started in)"
        } else {
            ""
        };
        return Ok(format!(
            "Folder: `{shown}`{note}\n\nMove this session with `/folder <path>`."
        ));
    }
    if s.home_day.is_some() {
        return Ok(AgentError::HomeIsBuiltIn.to_string());
    }
    if state.is_running(session_id) {
        return Ok(
            "Thrall is working in this session; change the folder when it's done.".to_owned(),
        );
    }
    Ok(match session::set_folder(state, session_id, args).await? {
        Ok(path) => format!(
            "This session now works in `{path}`. Files from earlier turns stay where they were \
             made, and rewinding still puts them back there."
        ),
        Err(why) => why,
    })
}

async fn tasks(state: &AppState, session_id: &str) -> Result<String, AgentError> {
    let tasks = state.store.tasks(&state.user_id, session_id).await?;
    if tasks.is_empty() {
        return Ok("No tasks in this session.".to_owned());
    }
    Ok(tasks
        .iter()
        .map(|t| {
            let mark = match t.status.as_str() {
                "done" => "[x]",
                "in_progress" => "[~]",
                _ => "[ ]",
            };
            format!("- {mark} {}", t.text)
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

async fn memory(state: &AppState, args: &str) -> Result<String, AgentError> {
    if args.is_empty() {
        return Ok("Give some words to look for: `/memory editor`.".to_owned());
    }
    let hits = state
        .store
        .search_knowledge(&state.user_id, args, MEMORY_COMMAND_LIMIT)
        .await?;
    if hits.is_empty() {
        return Ok(format!("Nothing remembered about “{args}”."));
    }
    Ok(hits
        .iter()
        .map(|k| format!("- **{}**: {}", k.topic, k.content))
        .collect::<Vec<_>>()
        .join("\n"))
}

async fn status(state: &AppState, session_id: &str) -> Result<String, AgentError> {
    let s = session::get(state, session_id).await?;
    let usage = state
        .store
        .usage_totals(&state.user_id, Some(session_id))
        .await?;
    let age_mins = (now_ms() - s.created_at).max(0) / 60_000;
    Ok(format!(
        "- **Model:** `{}`\n- **Session:** {} (`{}`, started {age_mins} min ago)\n- **Folder:** \
         `{}`\n- **This session:** {} · {} tokens in · {} out",
        state.model(),
        s.title,
        s.id,
        session::folder_of(&s).display(),
        money(usage.cost_usd),
        usage.input_tokens,
        usage.output_tokens
    ))
}
