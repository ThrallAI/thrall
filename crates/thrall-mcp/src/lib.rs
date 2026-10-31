//! MCP client with deferred, on-demand tool loading.
//!
//! Servers from `[mcp.servers.<name>]` are started only when Thrall first needs them, and their
//! tools are never all put in front of the model: it searches for what it needs (`mcp_search`) and
//! calls it (`mcp_call`). A Python Thrall lesson — hundreds of MCP tools in every prompt cost a
//! fortune and confused the model.
//!
//! A server is either a program on this computer (`command`, spoken to over its input and output)
//! or a server on the web (`url`, see [`remote`]).

pub mod constants;
pub mod remote;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;
use thrall_core::config::McpServerConfig;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::constants::{
    CALL_TIMEOUT, CONNECT_TIMEOUT, RESULT_MAX_CHARS, RESULT_TRIMMED_NOTE, SEARCH_MAX_RESULTS,
    SERVER_NAME_MIN_CHARS,
};

pub(crate) type Client = RunningService<RoleClient, ()>;

/// A config value, with `"keychain:name"` fetched from the keychain now.
pub(crate) fn secret_value(value: &str) -> Result<String, String> {
    match thrall_core::secrets::keychain_ref(value) {
        Some(name) => match thrall_core::secrets::get_named(name) {
            Ok(Some(key)) => Ok(key),
            Ok(None) => Err(format!(
                "its `{name}` key isn't saved yet: run `thrall key set {name}`"
            )),
            Err(e) => Err(e.to_string()),
        },
        None => Ok(value.to_owned()),
    }
}

/// Something went wrong talking to a server.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// No server by that name in config.toml.
    #[error("there's no MCP server called {0} (see [mcp.servers] in config.toml)")]
    UnknownServer(String),
    /// The server didn't start or didn't answer.
    #[error("the {server} MCP server couldn't start: {detail}")]
    Start {
        /// Server name.
        server: String,
        /// What happened.
        detail: String,
    },
    /// A request to the server failed.
    #[error("the {server} MCP server failed: {detail}")]
    Call {
        /// Server name.
        server: String,
        /// What happened.
        detail: String,
    },
    /// A server on the web wants the person to sign in first.
    #[error(
        "the {0} MCP server needs you to sign in: type `/mcp login {0}`, or use Sign in under \
         Settings → Tools"
    )]
    SignInNeeded(String),
}

/// One tool a server offers.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInfo {
    /// Server name.
    pub server: String,
    /// Tool name.
    pub name: String,
    /// What it does.
    pub description: String,
    /// JSON Schema of its arguments.
    pub schema: Value,
}

/// Start a server that's a program on this computer.
async fn start_local(server: &str, config: &McpServerConfig) -> Result<Client, McpError> {
    let start_error = |detail: String| McpError::Start {
        server: server.to_owned(),
        detail,
    };
    // On Windows, bare launchers like npx and uvx are .cmd scripts: start those through cmd.
    // Full paths (which may contain spaces) start directly.
    let bare = !config.command.contains(['\\', '/', '.']);
    let mut command = if cfg!(windows) && bare {
        let mut c = Command::new("cmd");
        c.arg("/c").arg(&config.command);
        c
    } else {
        Command::new(&config.command)
    };
    // `"keychain:name"` values are keys from the keychain, fetched only now.
    let mut env = Vec::with_capacity(config.env.len());
    for (var, value) in &config.env {
        env.push((var.clone(), secret_value(value).map_err(start_error)?));
    }
    command.args(&config.args).envs(env);
    // No console window: it would pop up for every server, and closing it kills the server.
    thrall_core::process::no_window(command.as_std_mut());
    let transport = TokioChildProcess::new(command).map_err(|e| start_error(e.to_string()))?;
    tokio::time::timeout(CONNECT_TIMEOUT, ().serve(transport))
        .await
        .map_err(|_| start_error(format!("no answer within {}s", CONNECT_TIMEOUT.as_secs())))?
        .map_err(|e| start_error(e.to_string()))
}

/// The configured servers, started on demand and kept running. A shared view (for the household's
/// members) sees only the servers shared with everyone, over the same running connections.
pub struct McpHub {
    /// The servers this hub may use.
    servers: BTreeMap<String, McpServerConfig>,
    clients: Arc<Mutex<HashMap<String, Arc<Client>>>>,
    tools: Arc<Mutex<HashMap<String, Vec<ToolInfo>>>>,
    /// `server/tool` for tools their server marks read-only (`readOnlyHint`). Plain lock: the
    /// permission check that reads it isn't async.
    read_only: Arc<std::sync::Mutex<HashSet<String>>>,
}

fn tool_key(server: &str, tool: &str) -> String {
    format!("{server}/{tool}")
}

/// A tool's result, cut to [`RESULT_MAX_CHARS`] with a note on how to ask for less.
fn trimmed(text: String) -> String {
    let total = text.chars().count();
    if total <= RESULT_MAX_CHARS {
        return text;
    }
    let mut out: String = text.chars().take(RESULT_MAX_CHARS).collect();
    out.push_str(
        &RESULT_TRIMMED_NOTE
            .replace("{shown}", &RESULT_MAX_CHARS.to_string())
            .replace("{total}", &total.to_string()),
    );
    out
}

/// Whether a request failed because the connection to the server is gone.
fn closed(e: &rmcp::service::ServiceError) -> bool {
    matches!(e, rmcp::service::ServiceError::TransportClosed)
}

impl McpHub {
    /// A hub for these servers (nothing starts yet).
    pub fn new(servers: BTreeMap<String, McpServerConfig>) -> Self {
        Self {
            servers,
            clients: Arc::new(Mutex::new(HashMap::new())),
            tools: Arc::new(Mutex::new(HashMap::new())),
            read_only: Arc::new(std::sync::Mutex::new(HashSet::new())),
        }
    }

    /// Only the servers shared with everyone (`shared = true`), using the same running
    /// connections: what the household's members may use.
    pub fn shared_view(&self) -> Self {
        Self {
            servers: self
                .servers
                .iter()
                .filter(|(_, c)| c.shared)
                .map(|(n, c)| (n.clone(), c.clone()))
                .collect(),
            clients: self.clients.clone(),
            tools: self.tools.clone(),
            read_only: self.read_only.clone(),
        }
    }

    /// `server`, if this hub may use it.
    fn allowed(&self, server: &str) -> Result<(), McpError> {
        if self.servers.contains_key(server) {
            Ok(())
        } else {
            Err(McpError::UnknownServer(server.to_owned()))
        }
    }

    /// Whether `server` says `tool` only reads (known once its tools have been listed).
    pub fn is_read_only(&self, server: &str, tool: &str) -> bool {
        self.read_only
            .lock()
            .is_ok_and(|set| set.contains(&tool_key(server, tool)))
    }

    /// Servers named in `query` (a word that is, or is part of, a server's name).
    pub fn named_in(&self, query: &str) -> Vec<String> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        self.servers
            .keys()
            .filter(|name| {
                let name = name.to_lowercase();
                words
                    .iter()
                    .any(|w| w.len() >= SERVER_NAME_MIN_CHARS && name.contains(w.as_str()))
            })
            .cloned()
            .collect()
    }

    /// Servers whose tools are already known (they've been started this run).
    pub async fn listed(&self) -> BTreeMap<String, Vec<String>> {
        self.tools
            .lock()
            .await
            .iter()
            .filter(|(s, _)| self.servers.contains_key(*s))
            .map(|(s, t)| (s.clone(), t.iter().map(|t| t.name.clone()).collect()))
            .collect()
    }

    /// Whether any servers are configured.
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    /// Configured server names.
    pub fn names(&self) -> Vec<String> {
        self.servers.keys().cloned().collect()
    }

    /// A configured server's settings.
    pub fn server(&self, name: &str) -> Option<&McpServerConfig> {
        self.servers.get(name)
    }

    /// Drop the connection to `server` (and what it said its tools were), so the next use
    /// connects afresh, e.g. after signing in or out.
    pub async fn reconnect(&self, server: &str) {
        if self.allowed(server).is_err() {
            return;
        }
        self.clients.lock().await.remove(server);
        self.tools.lock().await.remove(server);
    }

    async fn client(&self, server: &str) -> Result<Arc<Client>, McpError> {
        self.allowed(server)?;
        let mut clients = self.clients.lock().await;
        if let Some(c) = clients.get(server) {
            if !c.is_transport_closed() {
                return Ok(c.clone());
            }
            // The program ended (it crashed, or its window was closed): start it again.
            tracing::info!(server, "MCP server had stopped; starting it again");
            clients.remove(server);
        }
        let config = self
            .servers
            .get(server)
            .ok_or_else(|| McpError::UnknownServer(server.to_owned()))?;
        let client = if config.is_remote() {
            remote::connect(server, config).await?
        } else {
            start_local(server, config).await?
        };
        let client = Arc::new(client);
        clients.insert(server.to_owned(), client.clone());
        // A freshly started server may be a newer version with other tools: ask for them again.
        self.tools.lock().await.remove(server);
        Ok(client)
    }

    /// A server's tools (asked once, then remembered).
    pub async fn tools(&self, server: &str) -> Result<Vec<ToolInfo>, McpError> {
        self.allowed(server)?;
        if let Some(t) = self.tools.lock().await.get(server) {
            return Ok(t.clone());
        }
        let client = self.client(server).await?;
        let listed = match client.list_all_tools().await {
            Err(e) if closed(&e) => {
                // It died between starting and now: once more with a fresh start.
                self.clients.lock().await.remove(server);
                self.client(server).await?.list_all_tools().await
            }
            other => other,
        }
        .map_err(|e| McpError::Call {
            server: server.to_owned(),
            detail: e.to_string(),
        })?;
        if let Ok(mut set) = self.read_only.lock() {
            for t in &listed {
                if t.annotations.as_ref().and_then(|a| a.read_only_hint) == Some(true) {
                    set.insert(tool_key(server, &t.name));
                }
            }
        }
        let tools: Vec<ToolInfo> = listed
            .into_iter()
            .map(|t| ToolInfo {
                server: server.to_owned(),
                name: t.name.to_string(),
                description: t.description.map(|d| d.to_string()).unwrap_or_default(),
                schema: Value::Object((*t.input_schema).clone()),
            })
            .collect();
        self.tools
            .lock()
            .await
            .insert(server.to_owned(), tools.clone());
        Ok(tools)
    }

    /// Tools matching `query`, plus a line for each server that couldn't be reached. A query that
    /// names servers ("cadcraft", "cadcraft layer") starts and searches only those, and a word
    /// that only names the server needn't match the tool. Otherwise every server is searched
    /// (each starts if it isn't running).
    pub async fn search(&self, query: &str) -> (Vec<ToolInfo>, Vec<String>) {
        let named = self.named_in(query);
        let mut words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let servers: Vec<String> = if named.is_empty() {
            self.servers.keys().cloned().collect()
        } else {
            let lower: Vec<String> = named.iter().map(|n| n.to_lowercase()).collect();
            words.retain(|w| !lower.iter().any(|n| n.contains(w.as_str())));
            named
        };
        let mut found = Vec::new();
        let mut problems = Vec::new();
        for server in &servers {
            match self.tools(server).await {
                Ok(tools) => {
                    for tool in tools {
                        let haystack =
                            format!("{} {} {}", tool.server, tool.name, tool.description)
                                .to_lowercase();
                        if words.iter().all(|w| haystack.contains(w.as_str())) {
                            found.push(tool);
                        }
                    }
                }
                Err(e) => problems.push(e.to_string()),
            }
        }
        found.truncate(SEARCH_MAX_RESULTS);
        (found, problems)
    }

    /// Every server with its tool names (or why it couldn't be reached).
    pub async fn overview(&self) -> Vec<(String, Result<Vec<String>, String>)> {
        let mut out = Vec::new();
        for server in self.servers.keys() {
            let entry = match self.tools(server).await {
                Ok(tools) => Ok(tools.into_iter().map(|t| t.name).collect()),
                Err(e) => Err(e.to_string()),
            };
            out.push((server.clone(), entry));
        }
        out
    }

    /// Call a tool. Returns its text output and whether the server said it failed.
    pub async fn call(
        &self,
        server: &str,
        tool: &str,
        arguments: serde_json::Map<String, Value>,
    ) -> Result<(String, bool), McpError> {
        let call_error = |detail: String| McpError::Call {
            server: server.to_owned(),
            detail,
        };
        let timed_out = || {
            call_error(format!(
                "{tool} took longer than {}s",
                CALL_TIMEOUT.as_secs()
            ))
        };
        let params = CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments);
        let client = self.client(server).await?;
        let result = match tokio::time::timeout(CALL_TIMEOUT, client.call_tool(params.clone()))
            .await
            .map_err(|_| timed_out())?
        {
            // The server died since it was started: start it again and try once more.
            Err(e) if closed(&e) => {
                tracing::info!(server, "MCP server connection closed; starting it again");
                self.clients.lock().await.remove(server);
                let client = self.client(server).await?;
                tokio::time::timeout(CALL_TIMEOUT, client.call_tool(params))
                    .await
                    .map_err(|_| timed_out())?
            }
            other => other,
        }
        .map_err(|e| call_error(e.to_string()))?;
        let mut text: String = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty()
            && let Some(structured) = &result.structured_content
        {
            text = structured.to_string();
        }
        if text.is_empty() {
            text = "(the tool returned no text)".to_owned();
        }
        Ok((trimmed(text), result.is_error.unwrap_or(false)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_results_are_cut_with_a_way_to_ask_for_less() {
        assert_eq!(trimmed("short".into()), "short");
        let big = "x".repeat(RESULT_MAX_CHARS + 5_000);
        let cut = trimmed(big);
        assert!(cut.starts_with(&"x".repeat(RESULT_MAX_CHARS)));
        assert!(cut.contains(&format!(
            "cut at {RESULT_MAX_CHARS} of {}",
            RESULT_MAX_CHARS + 5_000
        )));
        assert!(cut.contains("filter"));
    }

    #[tokio::test]
    async fn unknown_servers_are_a_clear_error() {
        let hub = McpHub::new(BTreeMap::new());
        assert!(hub.is_empty());
        let err = hub
            .call("nope", "x", serde_json::Map::new())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no MCP server called nope"));
    }

    #[tokio::test]
    async fn the_shared_view_sees_only_shared_servers() {
        let mut servers = BTreeMap::new();
        let mine = McpServerConfig {
            command: "thrall-no-such-program".into(),
            ..McpServerConfig::default()
        };
        let ours = McpServerConfig {
            shared: true,
            ..mine.clone()
        };
        servers.insert("mine".to_owned(), mine);
        servers.insert("ours".to_owned(), ours);
        let hub = McpHub::new(servers);
        let view = hub.shared_view();
        assert_eq!(hub.names(), vec!["mine", "ours"]);
        assert_eq!(view.names(), vec!["ours"]);
        assert!(view.server("mine").is_none() && view.named_in("mine").is_empty());
        // What the owner's hub already knows about stays out of the view.
        hub.tools.lock().await.insert("mine".to_owned(), Vec::new());
        assert!(view.listed().await.is_empty());
        for err in [
            view.tools("mine").await.unwrap_err(),
            view.call("mine", "x", serde_json::Map::new())
                .await
                .unwrap_err(),
        ] {
            assert!(
                err.to_string().contains("no MCP server called mine"),
                "{err}"
            );
        }
        assert!(McpHub::new(BTreeMap::new()).shared_view().is_empty());
    }

    #[tokio::test]
    async fn a_server_that_cant_start_is_reported_not_fatal() {
        let mut servers = BTreeMap::new();
        servers.insert(
            "broken".to_owned(),
            McpServerConfig {
                command: "thrall-no-such-program-xyz".into(),
                ..Default::default()
            },
        );
        let hub = McpHub::new(servers);
        let (found, problems) = hub.search("anything").await;
        assert!(found.is_empty());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("broken"));
    }

    pub(crate) mod web {
        use rmcp::handler::server::router::tool::ToolRouter;
        use rmcp::model::{ServerCapabilities, ServerConfig};
        use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService,
        };
        use rmcp::{ServerHandler, tool, tool_handler, tool_router};

        /// A tiny MCP server on the web with one tool.
        #[derive(Debug, Clone)]
        pub struct Greeter {
            tool_router: ToolRouter<Self>,
        }

        #[tool_router]
        impl Greeter {
            #[tool(description = "Say hello from the test server")]
            fn hello(&self) -> String {
                "hello from the web".to_owned()
            }

            #[tool(
                description = "Show what the test server has, changing nothing",
                annotations(read_only_hint = true)
            )]
            fn look(&self) -> String {
                "nothing much".to_owned()
            }
        }

        #[tool_handler(router = self.tool_router)]
        impl ServerHandler for Greeter {
            fn get_info(&self) -> ServerConfig {
                ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            }
        }

        /// The server, ready to mount on a router.
        pub fn service() -> StreamableHttpService<Greeter, LocalSessionManager> {
            StreamableHttpService::new(
                || {
                    Ok(Greeter {
                        tool_router: Greeter::tool_router(),
                    })
                },
                Default::default(),
                StreamableHttpServerConfig::default().with_sse_keep_alive(None),
            )
        }

        /// Serve it on a free local port; with `key`, only requests carrying `X-Api-Key: key` are
        /// let in (others get 401). Returns its address.
        pub async fn serve(key: Option<&'static str>) -> String {
            let router = axum::Router::new()
                .nest_service("/mcp", service())
                .layer(axum::middleware::from_fn(
                    move |request: axum::extract::Request, next: axum::middleware::Next| async move {
                        let given = request
                            .headers()
                            .get("x-api-key")
                            .and_then(|v| v.to_str().ok());
                        if key.is_some() && given != key {
                            return axum::response::Response::builder()
                                .status(401)
                                .header("WWW-Authenticate", "Bearer")
                                .body(axum::body::Body::empty())
                                .unwrap_or_default();
                        }
                        next.run(request).await
                    },
                ));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let _ = axum::serve(listener, router).await;
            });
            format!("http://{address}/mcp")
        }
    }

    fn web_hub(name: &str, url: String, headers: &[(&str, &str)]) -> McpHub {
        let mut servers = BTreeMap::new();
        servers.insert(
            name.to_owned(),
            McpServerConfig {
                url,
                headers: headers
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
                ..Default::default()
            },
        );
        McpHub::new(servers)
    }

    #[tokio::test]
    async fn a_server_on_the_web_is_searched_and_called() {
        let url = web::serve(None).await;
        let hub = web_hub("thrall-test-web", url, &[]);
        let (found, problems) = hub.search("hello").await;
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "hello");
        let (text, failed) = hub
            .call("thrall-test-web", "hello", serde_json::Map::new())
            .await
            .unwrap();
        assert_eq!((text.as_str(), failed), ("hello from the web", false));
    }

    #[tokio::test]
    async fn naming_a_server_searches_and_starts_only_that_one() {
        let url = web::serve(None).await;
        let mut servers = BTreeMap::new();
        servers.insert(
            "greeter".to_owned(),
            McpServerConfig {
                url,
                ..Default::default()
            },
        );
        // A local program that can't start: trying it would show as a problem.
        servers.insert(
            "broken".to_owned(),
            McpServerConfig {
                command: "thrall-no-such-program-xyz".into(),
                ..Default::default()
            },
        );
        let hub = McpHub::new(servers);
        assert!(hub.listed().await.is_empty(), "nothing starts up front");
        let (found, problems) = hub.search("greeter").await;
        assert!(
            problems.is_empty(),
            "the other server was started: {problems:?}"
        );
        let mut names: Vec<_> = found.iter().map(|t| t.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["hello", "look"]);
        // A word beside the server's name narrows its tools.
        let (found, _) = hub.search("greeter look").await;
        assert_eq!(found.len(), 1);
        assert_eq!(hub.listed().await.keys().collect::<Vec<_>>(), ["greeter"]);
        // Without a server's name, every server is searched.
        let (found, problems) = hub.search("hello").await;
        assert_eq!(found.len(), 1);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(hub.named_in("a b").is_empty(), "short words name nothing");
    }

    /// Against a real local MCP server, by hand: `THRALL_LIVE_MCP="<program>|<arg>|<tool>|<kill>"`,
    /// where `<kill>` is a command that ends the server (`{pid}` is this test's process id). The
    /// server must answer, then answer again after being killed.
    #[tokio::test]
    #[ignore = "needs a real MCP server; run by hand"]
    async fn a_real_server_comes_back_after_it_dies() {
        let Ok(spec) = std::env::var("THRALL_LIVE_MCP") else {
            return;
        };
        // The kill command may itself contain `|`.
        let parts: Vec<&str> = spec.splitn(4, '|').collect();
        let (program, arg, tool, kill) = (parts[0], parts[1], parts[2], parts[3]);
        let mut servers = BTreeMap::new();
        servers.insert(
            "live".to_owned(),
            McpServerConfig {
                command: program.into(),
                args: vec![arg.into()],
                ..Default::default()
            },
        );
        let hub = McpHub::new(servers);
        let (first, failed) = hub
            .call("live", tool, serde_json::Map::new())
            .await
            .unwrap();
        assert!(!failed, "{first}");
        println!("first call: {} characters", first.len());
        let kill = kill.replace("{pid}", &std::process::id().to_string());
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &kill])
            .status()
            .unwrap();
        assert!(status.success());
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let (again, failed) = hub
            .call("live", tool, serde_json::Map::new())
            .await
            .unwrap();
        assert!(!failed, "{again}");
        println!("after the kill: {} characters", again.len());
    }

    #[tokio::test]
    async fn tools_a_server_marks_read_only_are_known() {
        let url = web::serve(None).await;
        let hub = web_hub("greeter", url, &[]);
        assert!(!hub.is_read_only("greeter", "look"), "unknown until listed");
        hub.tools("greeter").await.unwrap();
        assert!(hub.is_read_only("greeter", "look"));
        assert!(!hub.is_read_only("greeter", "hello"));
    }

    #[tokio::test]
    async fn a_web_server_that_wants_a_key_says_to_sign_in_until_it_has_one() {
        let url = web::serve(Some("open-sesame")).await;
        let hub = web_hub("thrall-test-locked", url.clone(), &[]);
        let (_, problems) = hub.search("hello").await;
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0].contains("needs you to sign in")
                && problems[0].contains("/mcp login thrall-test-locked"),
            "{problems:?}"
        );
        let hub = web_hub("thrall-test-locked", url, &[("X-Api-Key", "open-sesame")]);
        let (found, problems) = hub.search("hello").await;
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(found.len(), 1);
    }
}
