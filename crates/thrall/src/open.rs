//! `thrall open`: the web app in your browser. The page carries no token, so this asks the engine
//! for a one-time sign-in link for you and opens that.

use std::process::ExitCode;

use thrall_core::{Config, Paths};
use thrall_protocol::{ClientKind, Reply, Request};
use thrall_tui::client::connect_as;

use crate::attach;
use crate::constants::{EXIT_SETUP_NEEDED, OPEN_CLIENT_NAME};

/// Open the web app, signed in as whoever the access token is (the owner).
pub async fn run(paths: &Paths, config: &Config) -> anyhow::Result<ExitCode> {
    let Some(found) = attach::engine(paths, config).await? else {
        return Ok(ExitCode::from(EXIT_SETUP_NEEDED));
    };
    let connected = connect_as(
        &found.ws_url,
        &attach::token(paths)?,
        ClientKind::Headless,
        OPEN_CLIENT_NAME,
    )
    .await?;
    let reply = connected
        .client
        .request(Request::SigninCreate {
            person: connected.user.id.clone(),
        })
        .await?;
    let Reply::SigninCode { code, .. } = reply else {
        anyhow::bail!("Thrall's engine didn't give a sign-in link.");
    };
    let link = found.signin_url(&code);
    if thrall_agent::signin::open_browser(&link) {
        println!("Opened Thrall in your browser.");
    } else {
        println!("Open this link in your browser (it works once):\n  {link}");
    }
    Ok(ExitCode::SUCCESS)
}
