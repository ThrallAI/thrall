//! Native tools, one file per tool group, each implementing the `Tool` trait.

pub mod agents;
pub mod background;
pub mod browser;
pub mod constants;
pub mod diff;
pub mod documents;
pub mod office;
pub mod paths;
pub mod process;
pub mod saved;
pub mod skills;
pub mod style;
pub mod tool;
pub mod tools;

pub use thrall_gates::Effect;
pub use tool::{Cancel, Interrupter, ReadLog, Tool, ToolContext, ToolOutput};

/// The HTTP client web tools share: sensible timeouts and a browser-like user agent.
pub fn http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(constants::USER_AGENT)
        .timeout(std::time::Duration::from_secs(constants::WEB_TIMEOUT_SECS))
        .build()
}
