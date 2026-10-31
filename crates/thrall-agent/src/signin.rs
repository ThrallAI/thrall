//! Signing in to ChatGPT: open OpenAI's page in the browser, wait for it to come back to
//! `localhost:1455`, swap the code for tokens, keep the refresh token in the OS keychain.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::constants::{
    CHATGPT_CALLBACK_PATH, CHATGPT_KEY_NAME, SIGN_IN_PAGE_DONE, SIGN_IN_PAGE_FAILED,
    SIGN_IN_TIMEOUT_SECS,
};
use crate::keys::KeyStore;

/// What's kept in the keychain for ChatGPT.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredChatGpt {
    /// For getting new access tokens.
    pub refresh_token: String,
    /// The ChatGPT account.
    #[serde(default)]
    pub account_id: Option<String>,
}

/// The stored ChatGPT sign-in, if any.
pub fn stored_chatgpt(keys: &dyn KeyStore) -> Option<StoredChatGpt> {
    serde_json::from_str(&keys.get(CHATGPT_KEY_NAME)?).ok()
}

/// Keep a ChatGPT sign-in in the keychain.
pub fn save_chatgpt(refresh_token: &str, account_id: Option<&str>) -> Result<(), String> {
    let stored = StoredChatGpt {
        refresh_token: refresh_token.to_owned(),
        account_id: account_id.map(str::to_owned),
    };
    let text = serde_json::to_string(&stored).map_err(|e| e.to_string())?;
    thrall_core::secrets::set_named(CHATGPT_KEY_NAME, &text).map_err(|e| e.to_string())
}

/// Forget the ChatGPT sign-in. Returns whether there was one.
pub fn forget_chatgpt() -> Result<bool, String> {
    thrall_core::secrets::delete_named(CHATGPT_KEY_NAME).map_err(|e| e.to_string())
}

/// Listen where OpenAI sends the browser back. Both IPv4 and IPv6 loopback, since browsers try
/// either for `localhost`.
pub async fn listen() -> Result<Vec<TcpListener>, String> {
    let port = thrall_llm::constants::OPENAI_CALLBACK_PORT;
    let v4 = TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| {
        format!(
            "couldn't listen on port {port} for the sign-in ({e}). Is another sign-in (or the \
             Codex app's) still open? Close it and try again."
        )
    })?;
    let mut all = vec![v4];
    if let Ok(v6) = TcpListener::bind(("::1", port)).await {
        all.push(v6);
    }
    Ok(all)
}

/// Listen where a web MCP server's sign-in sends the browser back (`http://127.0.0.1:<port>`).
pub async fn listen_for_mcp() -> Result<Vec<TcpListener>, String> {
    let port = thrall_mcp::constants::SIGN_IN_PORT;
    let v4 = TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| {
        format!(
            "couldn't listen on port {port} for the sign-in ({e}). Is another MCP sign-in still \
             open? Finish or close it and try again."
        )
    })?;
    Ok(vec![v4])
}

/// Where a web MCP server's sign-in sends the browser back.
pub fn mcp_redirect_uri() -> String {
    format!(
        "http://127.0.0.1:{}{}",
        thrall_mcp::constants::SIGN_IN_PORT,
        thrall_mcp::constants::SIGN_IN_PATH
    )
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The query of a request line `GET /auth/callback?a=b HTTP/1.1`, if it's for `callback` (the
/// path the browser is sent back to).
fn callback_query(request: &str, callback: &str) -> Option<Vec<(String, String)>> {
    let target = request.lines().next()?.split_whitespace().nth(1)?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != callback {
        return None;
    }
    Some(
        query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(k, v)| (percent_decode(k), percent_decode(v)))
            .collect(),
    )
}

/// `text` safe to put in a page.
fn html_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn answer(stream: &mut tokio::net::TcpStream, status: &str, page: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{page}",
        page.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Wait for the browser to come back from ChatGPT's sign-in with `state`; the code it brings.
pub async fn wait_for_code(listeners: Vec<TcpListener>, state: &str) -> Result<String, String> {
    wait_for(listeners, CHATGPT_CALLBACK_PATH, state, "ChatGPT").await
}

/// Wait for the browser to come back to `callback` with `state` from signing in to `who`; the
/// code it brings. Other requests (the favicon) get a 404 and another attempt's `state` a 400,
/// and the wait goes on; an error from the sign-in for this attempt ends it.
pub async fn wait_for(
    listeners: Vec<TcpListener>,
    callback: &str,
    state: &str,
    who: &str,
) -> Result<String, String> {
    let done_page = SIGN_IN_PAGE_DONE.replace("{who}", &html_text(who));
    /// Stops the listeners when the wait ends, however it ends (finished, timed out, cancelled),
    /// so the port is free for the next attempt.
    struct Stop(Vec<tokio::task::JoinHandle<()>>);
    impl Drop for Stop {
        fn drop(&mut self) {
            for task in &self.0 {
                task.abort();
            }
        }
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let mut accepting = Stop(Vec::new());
    for listener in listeners {
        let tx = tx.clone();
        accepting.0.push(tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                if tx.send(stream).await.is_err() {
                    break;
                }
            }
        }));
    }
    drop(tx);
    let result = tokio::time::timeout(Duration::from_secs(SIGN_IN_TIMEOUT_SECS), async {
        while let Some(mut stream) = rx.recv().await {
            let mut buf = vec![0u8; 8192];
            let n = match tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buf)).await
            {
                Ok(Ok(n)) => n,
                _ => continue,
            };
            let request = String::from_utf8_lossy(&buf[..n]);
            let Some(query) = callback_query(&request, callback) else {
                answer(&mut stream, "404 Not Found", "").await;
                continue;
            };
            let get = |k: &str| {
                query
                    .iter()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v.clone())
            };
            // An older attempt's tab coming back (or anything else) doesn't end this one.
            if get("state").as_deref() != Some(state) {
                answer(&mut stream, "400 Bad Request", SIGN_IN_PAGE_FAILED).await;
                continue;
            }
            if let Some(error) = get("error") {
                answer(&mut stream, "400 Bad Request", SIGN_IN_PAGE_FAILED).await;
                let detail = get("error_description").unwrap_or(error);
                return Err(format!("{who} didn't sign you in: {detail}"));
            }
            let Some(code) = get("code").filter(|c| !c.is_empty()) else {
                answer(&mut stream, "400 Bad Request", SIGN_IN_PAGE_FAILED).await;
                return Err("the sign-in came back without a code. Try again.".to_owned());
            };
            answer(&mut stream, "200 OK", &done_page).await;
            return Ok(code);
        }
        Err("stopped listening for the sign-in".to_owned())
    })
    .await
    .unwrap_or_else(|_| {
        Err(format!(
            "the sign-in wasn't finished within {} minutes. Try again.",
            SIGN_IN_TIMEOUT_SECS / 60
        ))
    });
    drop(accepting);
    result
}

/// Open `url` in the person's browser (the engine runs on their computer).
pub fn open_browser(url: &str) -> bool {
    #[cfg(windows)]
    let mut command = {
        let mut c = std::process::Command::new("rundll32.exe");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_callback_query_is_read() {
        let q = callback_query(
            "GET /auth/callback?code=ab%2Fc&state=x+y HTTP/1.1\r\nHost: l\r\n",
            CHATGPT_CALLBACK_PATH,
        )
        .unwrap();
        assert_eq!(
            q,
            vec![
                ("code".into(), "ab/c".into()),
                ("state".into(), "x y".into())
            ]
        );
        assert!(callback_query("GET /favicon.ico HTTP/1.1", CHATGPT_CALLBACK_PATH).is_none());
    }

    #[tokio::test]
    async fn an_mcp_sign_in_comes_back_to_its_own_path_and_names_the_server() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let path = thrall_mcp::constants::SIGN_IN_PATH;
        let waiting =
            tokio::spawn(async move { wait_for(vec![listener], path, "s9", "<docs>").await });
        let chatgpt = send_line(port, b"GET /auth/callback?code=x&state=s9 HTTP/1.1\r\n\r\n").await;
        assert!(chatgpt.starts_with("HTTP/1.1 404"));
        let line = format!("GET {path}?code=c9&state=s9 HTTP/1.1\r\n\r\n");
        let page = send_line(port, line.as_bytes()).await;
        assert!(page.contains("Signed in to &lt;docs&gt;"), "{page}");
        assert_eq!(waiting.await.unwrap().unwrap(), "c9");
        assert!(mcp_redirect_uri().ends_with(path));
    }

    #[tokio::test]
    async fn the_browser_coming_back_gives_the_code() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiting = tokio::spawn(async move { wait_for_code(vec![listener], "s1").await });
        let send = |path: String| async move {
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            s.write_all(format!("GET {path} HTTP/1.1\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut reply = String::new();
            let _ = s.read_to_string(&mut reply).await;
            reply
        };
        assert!(
            send("/favicon.ico".into())
                .await
                .starts_with("HTTP/1.1 404")
        );
        assert!(
            send("/auth/callback?code=c1&state=s1".into())
                .await
                .contains("200 OK")
        );
        assert_eq!(waiting.await.unwrap().unwrap(), "c1");
    }

    async fn send_line(port: u16, line: &[u8]) -> String {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        s.write_all(line).await.unwrap();
        let mut reply = String::new();
        let _ = s.read_to_string(&mut reply).await;
        reply
    }

    #[tokio::test]
    async fn another_attempt_is_refused_and_the_wait_goes_on() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiting = tokio::spawn(async move { wait_for_code(vec![listener], "mine").await });
        let stale = send_line(
            port,
            b"GET /auth/callback?code=c&state=old HTTP/1.1\r\n\r\n",
        )
        .await;
        assert!(stale.contains("400"));
        let mine = send_line(
            port,
            b"GET /auth/callback?code=c2&state=mine HTTP/1.1\r\n\r\n",
        )
        .await;
        assert!(mine.contains("200"));
        assert_eq!(waiting.await.unwrap().unwrap(), "c2");
    }

    #[tokio::test]
    async fn openai_saying_no_ends_the_wait() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiting = tokio::spawn(async move { wait_for_code(vec![listener], "mine").await });
        send_line(
            port,
            b"GET /auth/callback?error=access_denied&state=mine HTTP/1.1\r\n\r\n",
        )
        .await;
        let e = waiting.await.unwrap().unwrap_err();
        assert!(e.contains("access_denied"));
    }
}
