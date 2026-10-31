//! Named constants for `thrall-mcp`. Every limit, timeout and repeated string lives here.

use std::time::Duration;

/// How long a server gets to start and say hello.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(45);

/// How long one tool call may take.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(180);

/// Shortest query word taken as naming a server (so  doesn't pick every server).
pub const SERVER_NAME_MIN_CHARS: usize = 3;

/// Most tools one search returns.
pub const SEARCH_MAX_RESULTS: usize = 15;

/// Most characters of a tool result handed back. Bigger results (a whole command catalogue) bury
/// smaller models: they're cut here, with [`RESULT_TRIMMED_NOTE`] saying how to narrow the call.
pub const RESULT_MAX_CHARS: usize = 20_000;

/// Added to a result that was cut ({shown} and {total} are character counts).
pub const RESULT_TRIMMED_NOTE: &str = "\n\n[Thrall: this result was cut at {shown} of {total} \
     characters. If what you need isn't above, call the tool again asking for less: many tools \
     take a filter, search, page or limit argument (see its arguments with mcp_search).]";

/// The name Thrall registers under with a web server's sign-in.
pub const CLIENT_NAME: &str = "Thrall";

/// Keychain names of web servers' sign-ins start with this (`mcp-<server>`).
pub const KEY_PREFIX: &str = "mcp-";

/// How long asking a web server whether it wants a sign-in may take.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// The port the browser comes back to after signing in to a web server (on 127.0.0.1). Fixed, so
/// a server that needs `client_id` can be told the address once.
pub const SIGN_IN_PORT: u16 = 7418;

/// The path the browser comes back to after signing in to a web server.
pub const SIGN_IN_PATH: &str = "/mcp/callback";
