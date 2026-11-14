//! MCP tools, loaded on demand: search the configured servers' tools, then call one. Only offered
//! when `[mcp.servers]` has entries.

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use thrall_gates::Effect;
use thrall_protocol::Risk;

use crate::constants::MCP_NOT_STARTED_NOTE;
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str, parse_args};

/// The `mcp_search` tool's name.
pub const MCP_SEARCH: &str = "mcp_search";
/// The `mcp_call` tool's name.
pub const MCP_CALL: &str = "mcp_call";

// ---------- mcp_search ----------

/// Find tools on the user's MCP servers.
pub struct McpSearch;

impl Tool for McpSearch {
    fn name(&self) -> &'static str {
        MCP_SEARCH
    }
    fn description(&self) -> &'static str {
        "Search the tools on the user's MCP servers (other apps and services connected to Thrall). \
         Give a server's name to start it and see its tools (e.g. \"cadcraft\", \"cadcraft \
         layer\"). Key words alone (e.g. \"calendar\") search only the servers already started. \
         An empty query lists the servers without starting any. Results include each tool's \
         server, name, description and arguments; then use mcp_call."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"]
        })
    }
    fn title(&self, args: &Value) -> String {
        let query = arg_str(args, "query");
        if query.trim().is_empty() {
            "Listing MCP servers".to_owned()
        } else {
            format!("Searching MCP tools for {query}")
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let Some(hub) = &ctx.mcp else {
                return ToolOutput::error(
                    "no MCP servers are set up ([mcp.servers] in config.toml)",
                );
            };
            let query = arg_str(&args, "query").trim().to_owned();
            if query.is_empty() {
                // No query: the servers, and the tools of those already running. Nothing is
                // started (each local server is a program; starting them all is slow and heavy).
                let listed = hub.listed().await;
                let mut out = String::new();
                for server in hub.names() {
                    match listed.get(&server) {
                        Some(names) => out.push_str(&format!(
                            "- {server} ({} tools): {}\n",
                            names.len(),
                            names.join(", ")
                        )),
                        None => out.push_str(&format!("- {server}: not started yet\n")),
                    }
                }
                out.push_str(
                    "\nSearch with a server's name or key words to see its tools' descriptions \
                     and arguments before calling one.",
                );
                return ToolOutput::ok(out, format!("{} servers", hub.names().len()));
            }
            let (found, problems) = hub.search(&query).await;
            let mut out = String::new();
            for tool in &found {
                out.push_str(&format!(
                    "- server `{}` · tool `{}`: {}\n  arguments: {}\n",
                    tool.server, tool.name, tool.description, tool.schema
                ));
            }
            if found.is_empty() {
                out.push_str("No matching tools.");
                let listed = hub.listed().await;
                let not_started: Vec<String> = hub
                    .names()
                    .into_iter()
                    .filter(|s| !listed.contains_key(s))
                    .collect();
                if hub.named_in(&query).is_empty() && !not_started.is_empty() {
                    out.push_str(
                        &MCP_NOT_STARTED_NOTE.replace("{servers}", &not_started.join(", ")),
                    );
                } else if !problems.is_empty() || hub.names().is_empty() {
                    out.push_str(&format!(" Servers: {}.", hub.names().join(", ")));
                }
            }
            for p in &problems {
                out.push_str(&format!("\nNote: {p}"));
            }
            ToolOutput::ok(out, format!("{} tools", found.len()))
        })
    }
}

/// After a failed call: the arguments the tool really takes (a wrong guess at their names is
/// the usual mistake), or the server's tool names when there's no tool by that name.
async fn usage(hub: &thrall_mcp::McpHub, server: &str, tool: &str) -> String {
    let Ok(tools) = hub.tools(server).await else {
        return String::new();
    };
    match tools.iter().find(|t| t.name == tool) {
        Some(t) => format!("\n{tool} takes these arguments: {}", t.schema),
        None => format!(
            "\n{server} has no tool {tool}; its tools: {}",
            tools
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

// ---------- mcp_call ----------

/// Call a tool on an MCP server.
pub struct McpCall;

#[derive(Deserialize)]
struct CallArgs {
    server: String,
    tool: String,
    #[serde(default)]
    arguments: serde_json::Map<String, Value>,
}

impl Tool for McpCall {
    fn name(&self) -> &'static str {
        MCP_CALL
    }
    fn description(&self) -> &'static str {
        "Call a tool on one of the user's MCP servers (find it first with mcp_search). The user \
         approves calls unless they've said always for that tool."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server": { "type": "string" },
                "tool": { "type": "string" },
                "arguments": { "type": "object" }
            },
            "required": ["server", "tool"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("{} · {}", arg_str(args, "server"), arg_str(args, "tool"))
    }
    fn precheck(&self, ctx: &ToolContext, args: &Value) -> Option<String> {
        // A call naming no server or tool, or a server that isn't set up, can't work: say so before
        // anyone is asked to approve it (models sometimes drop the names or invent a server).
        let (server, tool) = (arg_str(args, "server"), arg_str(args, "tool"));
        let names = ctx.mcp.as_ref().map(|hub| hub.names()).unwrap_or_default();
        if server.trim().is_empty() || tool.trim().is_empty() {
            return Some(format!(
                "mcp_call needs \"server\" and \"tool\" next to \"arguments\", e.g. {{\"server\": \
                 \"ark\", \"tool\": \"devkit_defaults\", \"arguments\": {{...}}}}. Servers: {}.",
                names.join(", ")
            ));
        }
        if !names.iter().any(|n| n == server) {
            return Some(format!(
                "there's no MCP server called {server}. Servers: {}. Use mcp_search to find tools.",
                names.join(", ")
            ));
        }
        None
    }
    fn prepare<'a>(&'a self, ctx: &'a ToolContext, args: &'a Value) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            // Which tools only read is learnt from the server's tool list. Without it (e.g. the
            // first call after a restart, made without mcp_search) a read-only tool would ask.
            let (server, tool) = (arg_str(args, "server"), arg_str(args, "tool"));
            if let Some(hub) = &ctx.mcp
                && !server.is_empty()
                && !hub.is_read_only(server, tool)
            {
                // Remembered after the first time; failures show up when the call itself runs.
                let _ = hub.tools(server).await;
            }
        })
    }
    fn effect(&self, ctx: &ToolContext, args: &Value) -> Effect {
        let (server, tool) = (arg_str(args, "server"), arg_str(args, "tool"));
        // A tool its server marks read-only just looks, so it runs even while planning.
        if ctx
            .mcp
            .as_ref()
            .is_some_and(|hub| hub.is_read_only(server, tool))
        {
            return Effect::Read;
        }
        // Anything else might change something: plan mode asks rather than refusing, since a
        // plan often needs a look at what an app offers.
        Effect::Probe {
            summary: format!("Use {tool} on the {server} MCP server"),
            risk: Risk::Medium,
            grant: Some(format!("mcp:{server}:{tool}")),
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let Some(hub) = &ctx.mcp else {
                return ToolOutput::error(
                    "no MCP servers are set up ([mcp.servers] in config.toml)",
                );
            };
            let a: CallArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            match hub.call(&a.server, &a.tool, a.arguments).await {
                Ok((text, false)) => {
                    let summary = format!("{} characters", text.chars().count());
                    ToolOutput::ok(text, summary)
                }
                Ok((text, true)) => ToolOutput::error(format!(
                    "the tool reported an error: {text}{}",
                    usage(hub, &a.server, &a.tool).await
                )),
                Err(e) => ToolOutput::error(format!("{e}{}", usage(hub, &a.server, &a.tool).await)),
            }
        })
    }
}
