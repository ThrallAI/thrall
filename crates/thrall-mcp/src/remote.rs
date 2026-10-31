//! MCP servers on the web (`url` in `[mcp.servers.<name>]`), over MCP's streamable HTTP, with the
//! MCP standard's sign-in: the server says where to sign in, Thrall registers itself there as its
//! own app, the person signs in in their browser, and the tokens stay in the OS keychain (renewed
//! as they run out).

use std::collections::HashMap;

use reqwest::header::{HeaderName, HeaderValue};
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_client::{
    AuthRequiredError, InsufficientScopeError, StreamableHttpClientTransportConfig,
};
use rmcp::transport::{
    AuthClient, AuthError, AuthorizationManager, AuthorizationRequest, AuthorizationSession,
    CredentialStore, StoredCredentials, StreamableHttpClientTransport,
};
use thrall_core::config::McpServerConfig;

use crate::constants::{CLIENT_NAME, CONNECT_TIMEOUT, KEY_PREFIX, PROBE_TIMEOUT};
use crate::{Client, McpError, secret_value};

/// The keychain name for `server`'s sign-in (`mcp-<server>`, made a valid key name).
pub fn key_name(server: &str) -> String {
    let clean: String = server
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{KEY_PREFIX}{clean}")
}

/// Whether there's a stored sign-in for `server`.
pub fn signed_in(server: &str) -> bool {
    matches!(
        thrall_core::secrets::get_long(&key_name(server)),
        Ok(Some(_))
    )
}

/// Forget `server`'s sign-in. Returns whether there was one.
pub fn sign_out(server: &str) -> Result<bool, String> {
    thrall_core::secrets::delete_long(&key_name(server)).map_err(|e| e.to_string())
}

/// A server's sign-in, kept in the keychain (split over several entries: tokens are long).
struct KeychainCredentials {
    key: String,
}

fn store_error(e: impl std::fmt::Display) -> AuthError {
    AuthError::CredentialStoreError(e.to_string())
}

#[async_trait::async_trait]
impl CredentialStore for KeychainCredentials {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        match thrall_core::secrets::get_long(&self.key).map_err(store_error)? {
            // Unreadable (from an older version, say): as if not signed in.
            Some(text) => Ok(serde_json::from_str(&text).ok()),
            None => Ok(None),
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let text = serde_json::to_string(&credentials).map_err(store_error)?;
        thrall_core::secrets::set_long(&self.key, &text).map_err(store_error)
    }

    async fn clear(&self) -> Result<(), AuthError> {
        thrall_core::secrets::delete_long(&self.key)
            .map(|_| ())
            .map_err(store_error)
    }
}

/// The server's extra headers, with `keychain:` values fetched now.
fn headers(
    server: &str,
    config: &McpServerConfig,
) -> Result<HashMap<HeaderName, HeaderValue>, McpError> {
    let start_error = |detail: String| McpError::Start {
        server: server.to_owned(),
        detail,
    };
    let mut out = HashMap::with_capacity(config.headers.len());
    for (name, value) in &config.headers {
        let value = secret_value(value).map_err(start_error)?;
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| start_error(format!("`{name}` isn't a valid header name")))?;
        let value = HeaderValue::from_str(&value)
            .map_err(|_| start_error(format!("the `{name}` header's value isn't valid")))?;
        out.insert(name, value);
    }
    Ok(out)
}

/// Whether a connection failed because the server wants a sign-in (or more permission).
fn wants_sign_in(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut next = Some(error);
    while let Some(e) = next {
        if e.is::<AuthRequiredError>() || e.is::<InsufficientScopeError>() {
            return true;
        }
        next = e.source();
    }
    false
}

/// Connect to a server on the web, signed in when there's a stored sign-in.
pub(crate) async fn connect(server: &str, config: &McpServerConfig) -> Result<Client, McpError> {
    let stored = signed_in(server).then(|| KeychainCredentials {
        key: key_name(server),
    });
    connect_with(server, config, stored).await
}

/// Connect, signed in with `stored` when given.
async fn connect_with<S: CredentialStore + 'static>(
    server: &str,
    config: &McpServerConfig,
    stored: Option<S>,
) -> Result<Client, McpError> {
    let start_error = |detail: String| McpError::Start {
        server: server.to_owned(),
        detail,
    };
    let transport_config = StreamableHttpClientTransportConfig::with_uri(config.url.as_str())
        .custom_headers(headers(server, config)?);
    let http = reqwest::Client::new();
    let served = if let Some(stored) = stored {
        let mut manager = AuthorizationManager::new(config.url.as_str())
            .await
            .map_err(|e| start_error(e.to_string()))?;
        manager.set_credential_store(stored);
        manager
            .initialize_from_store()
            .await
            .map_err(|e| start_error(format!("couldn't use your sign-in: {e}")))?;
        let transport = StreamableHttpClientTransport::with_client(
            AuthClient::new(http, manager),
            transport_config,
        );
        tokio::time::timeout(CONNECT_TIMEOUT, ().serve(transport)).await
    } else {
        let transport = StreamableHttpClientTransport::with_client(http, transport_config);
        tokio::time::timeout(CONNECT_TIMEOUT, ().serve(transport)).await
    };
    match served {
        Err(_) => Err(start_error(format!(
            "no answer within {}s",
            CONNECT_TIMEOUT.as_secs()
        ))),
        // The transport's error isn't linked as the source here, so look inside it.
        Ok(Err(rmcp::service::ClientInitializeError::TransportError { error, .. }))
            if wants_sign_in(&error) =>
        {
            Err(McpError::SignInNeeded(server.to_owned()))
        }
        Ok(Err(e)) => Err(start_error(e.to_string())),
        Ok(Ok(client)) => Ok(client),
    }
}

/// Ask the server without a sign-in: its `WWW-Authenticate` answer when it wants one.
async fn challenge(server: &str, config: &McpServerConfig) -> Result<Option<String>, String> {
    let hello = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": rmcp::model::ProtocolVersion::LATEST_WITH_INITIALIZE,
            "capabilities": {},
            "clientInfo": {"name": CLIENT_NAME, "version": env!("CARGO_PKG_VERSION")}
        }
    });
    let mut request = reqwest::Client::new()
        .post(config.url.as_str())
        .header("Accept", "application/json, text/event-stream")
        .json(&hello)
        .timeout(PROBE_TIMEOUT);
    for (name, value) in headers(server, config).map_err(|e| e.to_string())? {
        request = request.header(name, value);
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("couldn't reach {}: {e}", config.url))?;
    if response.status() != reqwest::StatusCode::UNAUTHORIZED
        && response.status() != reqwest::StatusCode::FORBIDDEN
    {
        return Ok(None);
    }
    Ok(Some(
        response
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned(),
    ))
}

/// A sign-in in progress: open `url` in the browser; it comes back to the redirect address with
/// `state` and a code for [`SignIn::finish`].
pub struct SignIn {
    /// The page to open.
    pub url: String,
    /// What the browser brings back, to match it to this sign-in.
    pub state: String,
    session: AuthorizationSession,
}

/// Start signing in to `server`: find where it signs in, register Thrall there (or use the
/// configured `client_id`) and make the page to open. `redirect_uri` is where the browser comes
/// back to.
pub async fn begin_sign_in(
    server: &str,
    config: &McpServerConfig,
    redirect_uri: &str,
) -> Result<SignIn, String> {
    let store = KeychainCredentials {
        key: key_name(server),
    };
    begin_sign_in_with(server, config, redirect_uri, store).await
}

/// [`begin_sign_in`], keeping the sign-in in `store`.
async fn begin_sign_in_with<S: CredentialStore + 'static>(
    server: &str,
    config: &McpServerConfig,
    redirect_uri: &str,
    store: S,
) -> Result<SignIn, String> {
    if !config.is_remote() {
        return Err(format!(
            "{server} runs on this computer, so there's nothing to sign in to. Sign-in is for \
             servers on the web (a `url`)."
        ));
    }
    let Some(challenge) = challenge(server, config).await? else {
        return Err(format!(
            "{server} didn't ask for a sign-in, so it works as it is."
        ));
    };
    let mut manager = AuthorizationManager::new(config.url.as_str())
        .await
        .map_err(|e| e.to_string())?;
    manager.set_credential_store(store);
    let found = manager
        .resolve_metadata_from_challenge((!challenge.is_empty()).then_some(challenge.as_str()))
        .await
        .map_err(|e| format!("couldn't find where {server} signs in: {e}"))?;
    manager.set_metadata(found.metadata);
    let mut request = AuthorizationRequest::new(redirect_uri).with_client_name(CLIENT_NAME);
    if !config.client_id.is_empty() {
        request = request.with_preregistered_client(config.client_id.as_str());
    }
    let session = AuthorizationSession::new(manager, request)
        .await
        .map_err(|(_, e)| match e {
            AuthError::RegistrationFailed(detail) => format!(
                "{server} didn't let Thrall register itself ({detail}). If the service gave you \
                 an app id for Thrall, put it in `client_id` for this server in config.toml."
            ),
            other => other.to_string(),
        })?;
    let url = session.get_authorization_url().to_owned();
    let state = reqwest::Url::parse(&url)
        .ok()
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "state")
                .map(|(_, v)| v.into_owned())
        })
        .ok_or("the sign-in page address has no state")?;
    Ok(SignIn {
        url,
        state,
        session,
    })
}

impl SignIn {
    /// Finish with the code the browser brought back: the tokens go to the keychain.
    pub async fn finish(self, code: &str) -> Result<(), String> {
        self.session
            .handle_callback(code, &self.state)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A web MCP server that wants a sign-in, with its own sign-in server (as the MCP standard
    /// lays out: protected-resource metadata, server metadata, self-registration, PKCE). Returns
    /// its base address.
    async fn serve_with_sign_in() -> String {
        use axum::Json;
        use axum::extract::{Form, Query};
        use axum::routing::{get, post};
        use serde_json::{Value, json};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let challenge =
            format!("Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\"");
        let gate = move |request: axum::extract::Request, next: axum::middleware::Next| {
            let challenge = challenge.clone();
            async move {
                let token = request
                    .headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok());
                if token != Some("Bearer tok-123") {
                    return axum::response::Response::builder()
                        .status(401)
                        .header("WWW-Authenticate", challenge)
                        .body(axum::body::Body::empty())
                        .unwrap_or_default();
                }
                next.run(request).await
            }
        };
        let resource = json!({"resource": format!("{base}/mcp"), "authorization_servers": [base]});
        let metadata = json!({
            "issuer": base,
            "authorization_endpoint": format!("{base}/authorize"),
            "token_endpoint": format!("{base}/token"),
            "registration_endpoint": format!("{base}/register"),
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["none"],
        });
        let app = axum::Router::new()
            .nest_service("/mcp", crate::tests::web::service())
            .layer(axum::middleware::from_fn(gate))
            .route(
                "/.well-known/oauth-protected-resource/mcp",
                get(move || async move { Json(resource) }),
            )
            .route(
                "/.well-known/oauth-authorization-server",
                get(move || async move { Json(metadata) }),
            )
            .route(
                "/register",
                post(|Json(body): Json<Value>| async move {
                    (
                        axum::http::StatusCode::CREATED,
                        Json(json!({
                            "client_id": "test-client",
                            "client_name": body["client_name"],
                            "redirect_uris": body["redirect_uris"],
                            "token_endpoint_auth_method": "none",
                        })),
                    )
                }),
            )
            .route(
                "/authorize",
                get(|Query(q): Query<HashMap<String, String>>| async move {
                    // The person signs in here; the server sends the browser back with a code.
                    let ok = q.get("client_id").map(String::as_str) == Some("test-client")
                        && q.get("code_challenge_method").map(String::as_str) == Some("S256");
                    let to = format!(
                        "{}?code={}&state={}",
                        q["redirect_uri"],
                        if ok { "code-1" } else { "refused" },
                        q["state"]
                    );
                    axum::response::Redirect::to(&to)
                }),
            )
            .route(
                "/token",
                post(|Form(f): Form<HashMap<String, String>>| async move {
                    let ok = f.get("grant_type").map(String::as_str) == Some("authorization_code")
                        && f.get("code").map(String::as_str) == Some("code-1")
                        && f.get("code_verifier").is_some_and(|v| !v.is_empty());
                    if !ok {
                        return (
                            axum::http::StatusCode::BAD_REQUEST,
                            Json(json!({"error": "invalid_grant"})),
                        );
                    }
                    (
                        axum::http::StatusCode::OK,
                        Json(json!({"access_token": "tok-123", "token_type": "Bearer",
                            "expires_in": 3600, "refresh_token": "refresh-1"})),
                    )
                }),
            );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        base
    }

    #[tokio::test]
    async fn signing_in_to_a_web_server_follows_the_mcp_standard_end_to_end() {
        use rmcp::transport::InMemoryCredentialStore;

        let base = serve_with_sign_in().await;
        let config = McpServerConfig {
            url: format!("{base}/mcp"),
            ..Default::default()
        };
        let server = "thrall-test-sign-in";
        let before = connect_with::<InMemoryCredentialStore>(server, &config, None).await;
        assert!(
            matches!(before, Err(McpError::SignInNeeded(_))),
            "a server that wants a sign-in says so"
        );

        let store = InMemoryCredentialStore::new();
        let attempt = begin_sign_in_with(server, &config, "http://127.0.0.1:9/cb", store.clone())
            .await
            .unwrap();
        // The browser: open the page, follow it back to Thrall's address.
        let back = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .get(&attempt.url)
            .send()
            .await
            .unwrap();
        let to = back.headers()[reqwest::header::LOCATION].to_str().unwrap();
        let to = reqwest::Url::parse(to).unwrap();
        let get = |k: &str| {
            to.query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.into_owned())
                .unwrap()
        };
        assert!(to.as_str().starts_with("http://127.0.0.1:9/cb"));
        assert_eq!(get("state"), attempt.state);
        assert_eq!(
            get("code"),
            "code-1",
            "Thrall registered itself and used PKCE"
        );
        attempt.finish(&get("code")).await.unwrap();
        let saved = store.load().await.unwrap().unwrap();
        assert_eq!(saved.client_id, "test-client");
        assert!(saved.token_response.is_some());

        let client = connect_with(server, &config, Some(store)).await.unwrap();
        let tools = client.list_all_tools().await.unwrap();
        assert_eq!(tools[0].name, "hello");
    }

    #[tokio::test]
    async fn a_server_that_doesnt_ask_for_a_sign_in_is_told_so() {
        let url = crate::tests::web::serve(None).await;
        let config = McpServerConfig {
            url,
            ..Default::default()
        };
        let e = begin_sign_in("thrall-test-open", &config, "http://127.0.0.1:9/cb")
            .await
            .err()
            .unwrap();
        assert!(e.contains("didn't ask for a sign-in"), "{e}");
        let local = McpServerConfig {
            command: "x".into(),
            ..Default::default()
        };
        let e = begin_sign_in("thrall-test-local", &local, "http://127.0.0.1:9/cb")
            .await
            .err()
            .unwrap();
        assert!(e.contains("runs on this computer"), "{e}");
    }

    #[test]
    fn key_names_are_valid_keychain_names() {
        assert_eq!(key_name("docs"), "mcp-docs");
        assert_eq!(key_name("My Server.io"), "mcp-my_server_io");
        assert!(thrall_core::secrets::valid_name(&key_name("My Server.io")));
    }

    #[test]
    fn a_sign_in_challenge_is_found_in_the_error_chain() {
        #[derive(Debug, thiserror::Error)]
        #[error("outer")]
        struct Outer(#[source] AuthRequiredError);
        let e = Outer(AuthRequiredError::new("Bearer".into()));
        assert!(wants_sign_in(&e));
        let plain = std::io::Error::other("refused");
        assert!(!wants_sign_in(&plain));
    }
}
