//! ChatGPT: chat on the person's own ChatGPT plan (Plus/Pro) instead of a paid API key. Signing in
//! is OpenAI's PKCE flow (the one the official Codex app uses); replies come from the Codex
//! backend's Responses API, streamed. Only the refresh token is kept (the caller stores it in the
//! keychain); access tokens live in memory and are renewed before they expire.

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use eventsource_stream::Eventsource;
use futures::{StreamExt, TryStreamExt, stream};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Digest as _;

use crate::constants::{
    CHATGPT_CLIENT_VERSION, CHATGPT_MODELS_URL, CHATGPT_READ_TIMEOUT_SECS, CHATGPT_RESPONSES_URL,
    CONNECT_TIMEOUT_SECS, MAX_ERROR_BODY_CHARS, MAX_RETRIES, OPENAI_AUTH_CLAIM,
    OPENAI_AUTHORIZE_URL, OPENAI_CLIENT_ID, OPENAI_REDIRECT_URI, OPENAI_SCOPES, OPENAI_TOKEN_URL,
    RETRY_BASE_DELAY_MS, TOKEN_RENEW_MARGIN_SECS,
};
use crate::error::LlmError;
use crate::provider::{LlmProvider, ReplyStream};
use crate::types::{ChatMessage, ChatRequest, ChatRole, StreamEvent, TokenUsage, ToolCallDelta};

const NAME: &str = "ChatGPT";
const ID: &str = "chatgpt";

/// What a sign-in or renewal returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokens {
    /// Short-lived token for API calls.
    pub access_token: String,
    /// Long-lived token for getting new access tokens (OpenAI may replace it on renewal).
    pub refresh_token: String,
    /// The ChatGPT account the plan belongs to.
    pub account_id: Option<String>,
    /// Seconds until `access_token` expires.
    pub expires_in: u64,
}

/// One sign-in attempt's secrets: the browser gets `challenge` and `state`; `verifier` stays here.
#[derive(Debug, Clone)]
pub struct SignIn {
    /// Sent with the code to prove this app started the sign-in.
    pub verifier: String,
    /// The browser must come back with this.
    pub state: String,
    /// The page to open.
    pub url: String,
}

fn random_text(bytes: usize) -> Result<String, LlmError> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| LlmError::BadResponse {
        provider: NAME.to_owned(),
        detail: format!("no randomness available: {e}"),
    })?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf))
}

fn form_encode(pairs: &[(&str, &str)]) -> String {
    let enc = |s: &str| {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    };
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Start a sign-in: fresh secrets and the page to open in the browser.
pub fn begin_sign_in() -> Result<SignIn, LlmError> {
    let verifier = random_text(48)?;
    let state = random_text(32)?;
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));
    let url = format!(
        "{OPENAI_AUTHORIZE_URL}?{}",
        form_encode(&[
            ("response_type", "code"),
            ("client_id", OPENAI_CLIENT_ID),
            ("redirect_uri", OPENAI_REDIRECT_URI),
            ("scope", OPENAI_SCOPES),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
        ])
    );
    Ok(SignIn {
        verifier,
        state,
        url,
    })
}

/// The ChatGPT account id inside a sign-in token (a JWT; read, not verified: it came straight
/// from OpenAI over TLS).
pub fn account_id(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get(OPENAI_AUTH_CLAIM)
        .and_then(|a| a.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

async fn token_request(
    client: &reqwest::Client,
    pairs: &[(&str, &str)],
    old_refresh: &str,
) -> Result<Tokens, LlmError> {
    #[derive(Deserialize)]
    struct Reply {
        access_token: String,
        refresh_token: Option<String>,
        id_token: Option<String>,
        expires_in: Option<u64>,
    }
    let response = client
        .post(OPENAI_TOKEN_URL)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form_encode(pairs))
        .send()
        .await
        .map_err(|e| network(&e))?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|e| network(&e))?;
    if !(200..300).contains(&status) {
        return Err(if status == 400 || status == 401 {
            signed_out()
        } else {
            error_from_status(status, &body)
        });
    }
    let reply: Reply = serde_json::from_str(&body).map_err(|e| LlmError::BadResponse {
        provider: NAME.to_owned(),
        detail: e.to_string(),
    })?;
    let account_id = reply
        .id_token
        .as_deref()
        .and_then(account_id)
        .or_else(|| account_id(&reply.access_token));
    Ok(Tokens {
        access_token: reply.access_token,
        refresh_token: reply
            .refresh_token
            .unwrap_or_else(|| old_refresh.to_owned()),
        account_id,
        expires_in: reply.expires_in.unwrap_or(3600),
    })
}

/// Swap the code the browser brought back for tokens.
pub async fn finish_sign_in(
    client: &reqwest::Client,
    code: &str,
    verifier: &str,
) -> Result<Tokens, LlmError> {
    token_request(
        client,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", OPENAI_CLIENT_ID),
            ("code", code),
            ("redirect_uri", OPENAI_REDIRECT_URI),
            ("code_verifier", verifier),
        ],
        "",
    )
    .await
}

/// A new access token from the refresh token.
pub async fn renew(client: &reqwest::Client, refresh_token: &str) -> Result<Tokens, LlmError> {
    token_request(
        client,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", OPENAI_CLIENT_ID),
            ("refresh_token", refresh_token),
        ],
        refresh_token,
    )
    .await
}

/// Called when OpenAI hands out a new refresh token, so it can be stored.
pub type SaveRefresh = Arc<dyn Fn(&str, Option<&str>) + Send + Sync>;

struct Auth {
    refresh_token: String,
    account_id: Option<String>,
    access: Option<(String, Instant)>,
}

/// The ChatGPT provider.
pub struct ChatGpt {
    client: reqwest::Client,
    auth: Arc<tokio::sync::Mutex<Auth>>,
    save: SaveRefresh,
    /// The plan's models, once listed.
    models: tokio::sync::Mutex<Option<Vec<String>>>,
}

impl ChatGpt {
    /// A provider for the account behind `refresh_token`. `save` stores a replaced refresh token.
    pub fn new(
        refresh_token: String,
        account_id: Option<String>,
        save: SaveRefresh,
    ) -> Result<Self, LlmError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .read_timeout(Duration::from_secs(CHATGPT_READ_TIMEOUT_SECS))
            .build()
            .map_err(|e| network(&e))?;
        Ok(Self {
            client,
            auth: Arc::new(tokio::sync::Mutex::new(Auth {
                refresh_token,
                account_id,
                access: None,
            })),
            save,
            models: tokio::sync::Mutex::new(None),
        })
    }
}

/// A usable access token and account id, renewing first when needed.
async fn access(
    client: &reqwest::Client,
    auth: &tokio::sync::Mutex<Auth>,
    save: &SaveRefresh,
) -> Result<(String, Option<String>), LlmError> {
    let mut a = auth.lock().await;
    if let Some((token, until)) = &a.access
        && Instant::now() < *until
    {
        return Ok((token.clone(), a.account_id.clone()));
    }
    let fresh = renew(client, &a.refresh_token).await?;
    if fresh.refresh_token != a.refresh_token {
        save(&fresh.refresh_token, fresh.account_id.as_deref());
        a.refresh_token = fresh.refresh_token.clone();
    }
    if fresh.account_id.is_some() {
        a.account_id = fresh.account_id.clone();
    }
    let lifetime = fresh
        .expires_in
        .saturating_sub(TOKEN_RENEW_MARGIN_SECS)
        .max(60);
    a.access = Some((
        fresh.access_token.clone(),
        Instant::now() + Duration::from_secs(lifetime),
    ));
    Ok((fresh.access_token, a.account_id.clone()))
}

/// The conversation as Responses API input: system text becomes `instructions`.
fn wire_input(messages: &[ChatMessage]) -> (String, Vec<Value>) {
    let mut instructions = Vec::new();
    let mut input = Vec::new();
    for m in messages {
        match m.role {
            ChatRole::System => instructions.push(m.content.clone()),
            ChatRole::User => {
                let mut parts = vec![json!({ "type": "input_text", "text": m.content })];
                parts.extend(m.images.iter().map(|img| {
                    json!({
                        "type": "input_image",
                        "image_url": format!("data:{};base64,{}", img.mime, img.base64),
                    })
                }));
                parts.extend(m.documents.iter().map(|doc| {
                    json!({
                        "type": "input_file",
                        "filename": doc.name,
                        "file_data": format!("data:{};base64,{}", doc.mime, doc.base64),
                    })
                }));
                input.push(json!({ "role": "user", "content": parts }));
            }
            ChatRole::Assistant => {
                if !m.content.is_empty() {
                    input.push(json!({
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": m.content }],
                    }));
                }
                for call in &m.tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": call.arguments,
                    }));
                }
            }
            ChatRole::Tool => input.push(json!({
                "type": "function_call_output",
                "call_id": m.tool_call_id.clone().unwrap_or_default(),
                "output": m.content,
            })),
        }
    }
    (instructions.join("\n\n"), input)
}

/// The request body. The Codex backend refuses `temperature`, `max_output_tokens`,
/// `tool_choice`, `parallel_tool_calls` and `strict` on tools, and needs `store: false`.
fn body(request: &ChatRequest, model: &str) -> Value {
    let (instructions, input) = wire_input(&request.messages);
    let mut body = json!({
        "model": model,
        "instructions": instructions,
        "input": input,
        "stream": true,
        "store": false,
    });
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            })
            .collect();
    }
    body
}

/// What the stream has seen so far, to turn Responses events into Thrall's.
#[derive(Default)]
struct Reading {
    /// Output positions of tool calls already announced (their name was passed on).
    started: Vec<u64>,
    /// Output positions of tool calls whose arguments were already passed on.
    streamed_args: Vec<u64>,
    /// Whether any tool call came.
    called: bool,
}

/// One server-sent event as Thrall's stream events.
fn parse_event(data: &str, seen: &mut Reading) -> Result<Vec<StreamEvent>, LlmError> {
    let ev: Value = serde_json::from_str(data).map_err(|e| LlmError::BadResponse {
        provider: NAME.to_owned(),
        detail: e.to_string(),
    })?;
    let text = |k: &str| ev.get(k).and_then(Value::as_str).unwrap_or_default();
    let index = ev.get("output_index").and_then(Value::as_u64).unwrap_or(0);
    let call_index = u32::try_from(index).unwrap_or(u32::MAX);
    Ok(match text("type") {
        "response.output_text.delta" => vec![StreamEvent::Text(text("delta").to_owned())],
        "response.reasoning_summary_text.delta" => {
            vec![StreamEvent::Reasoning(text("delta").to_owned())]
        }
        "response.output_item.added" | "response.output_item.done" => {
            let item = ev.get("item").cloned().unwrap_or_default();
            if item.get("type").and_then(Value::as_str) != Some("function_call") {
                return Ok(Vec::new());
            }
            seen.called = true;
            let field = |k: &str| item.get(k).and_then(Value::as_str).map(str::to_owned);
            // `done` repeats the whole call: pass on only what wasn't passed on already.
            let name = if seen.started.contains(&index) {
                None
            } else {
                seen.started.push(index);
                field("name")
            };
            let arguments = field("arguments").unwrap_or_default();
            let arguments = if arguments.is_empty() || seen.streamed_args.contains(&index) {
                String::new()
            } else {
                seen.streamed_args.push(index);
                arguments
            };
            vec![StreamEvent::ToolCall(ToolCallDelta {
                index: call_index,
                id: field("call_id"),
                name,
                arguments,
            })]
        }
        "response.function_call_arguments.delta" => {
            if !seen.streamed_args.contains(&index) {
                seen.streamed_args.push(index);
            }
            vec![StreamEvent::ToolCall(ToolCallDelta {
                index: call_index,
                id: None,
                name: None,
                arguments: text("delta").to_owned(),
            })]
        }
        "response.completed" | "response.incomplete" => {
            let usage = ev.pointer("/response/usage");
            let n = |k: &str| {
                usage
                    .and_then(|u| u.get(k))
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            };
            let reason = if text("type") == "response.incomplete" {
                "length"
            } else if seen.called {
                "tool_calls"
            } else {
                "stop"
            };
            vec![
                StreamEvent::Usage(TokenUsage {
                    input_tokens: n("input_tokens"),
                    output_tokens: n("output_tokens"),
                    cached_tokens: usage
                        .and_then(|u| u.pointer("/input_tokens_details/cached_tokens"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    cost_usd: Some(0.0),
                }),
                StreamEvent::Finished(Some(reason.to_owned())),
            ]
        }
        "response.failed" | "error" => {
            let message = ev
                .pointer("/response/error/message")
                .or_else(|| ev.pointer("/error/message"))
                .or_else(|| ev.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("the reply failed");
            return Err(LlmError::Rejected {
                provider: NAME.to_owned(),
                status: 0,
                message: message.to_owned(),
            });
        }
        _ => Vec::new(),
    })
}

fn signed_out() -> LlmError {
    LlmError::Rejected {
        provider: NAME.to_owned(),
        status: 401,
        message: "the ChatGPT sign-in has expired or was revoked. Sign in again with \
                  `/login chatgpt` (or Settings → Models)."
            .to_owned(),
    }
}

fn error_from_status(status: u16, body: &str) -> LlmError {
    match status {
        401 | 403 => signed_out(),
        429 => LlmError::RateLimited {
            provider: NAME.to_owned(),
            retry_after_secs: None,
        },
        500..=599 => LlmError::ServerError {
            provider: NAME.to_owned(),
            status,
        },
        _ => {
            let v: Value = serde_json::from_str(body).unwrap_or_default();
            let message = v
                .pointer("/error/message")
                .or_else(|| v.get("detail"))
                .and_then(Value::as_str)
                .map_or_else(|| body.trim().to_owned(), str::to_owned);
            let mut message: String = message.chars().take(MAX_ERROR_BODY_CHARS).collect();
            if message.contains("not supported") {
                message.push_str(" Type /model to see the models your ChatGPT plan has.");
            }
            LlmError::Rejected {
                provider: NAME.to_owned(),
                status,
                message,
            }
        }
    }
}

fn network(e: &reqwest::Error) -> LlmError {
    LlmError::Network {
        provider: NAME.to_owned(),
        detail: if e.is_timeout() {
            "the connection timed out".to_owned()
        } else {
            e.to_string()
        },
    }
}

/// Send `body`, renewing the token once on 401 and retrying temporary failures.
async fn send(
    client: &reqwest::Client,
    auth: &tokio::sync::Mutex<Auth>,
    save: &SaveRefresh,
    body: &Value,
) -> Result<reqwest::Response, LlmError> {
    let mut attempt = 0;
    let mut renewed = false;
    loop {
        let (token, account) = access(client, auth, save).await?;
        let mut req = client
            .post(CHATGPT_RESPONSES_URL)
            .bearer_auth(token)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(body);
        if let Some(account) = account {
            req = req.header("chatgpt-account-id", account);
        }
        let error = match req.send().await {
            Ok(r) if r.status().is_success() => return Ok(r),
            Ok(r) => {
                let status = r.status().as_u16();
                let text = r.text().await.unwrap_or_default();
                if status == 401 && !renewed {
                    renewed = true;
                    auth.lock().await.access = None;
                    continue;
                }
                error_from_status(status, &text)
            }
            Err(e) => network(&e),
        };
        if !error.is_retryable() || attempt >= MAX_RETRIES {
            return Err(error);
        }
        let delay = Duration::from_millis(RETRY_BASE_DELAY_MS.saturating_mul(1 << attempt));
        tracing::warn!(attempt, ?delay, %error, "retrying ChatGPT call");
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

impl LlmProvider for ChatGpt {
    fn name(&self) -> &str {
        NAME
    }

    fn stream(&self, request: ChatRequest) -> ReplyStream {
        let model = request
            .model
            .strip_prefix(crate::constants::CHATGPT_MODEL_PREFIX)
            .unwrap_or(&request.model)
            .to_owned();
        let body = body(&request, &model);
        let (client, auth, save) = (self.client.clone(), self.auth.clone(), self.save.clone());
        let start = async move {
            let response = send(&client, &auth, &save, &body).await?;
            let mut seen = Reading::default();
            let events = response
                .bytes_stream()
                .eventsource()
                .map(move |item| match item {
                    Ok(event) => parse_event(&event.data, &mut seen),
                    Err(e) => Err(LlmError::Network {
                        provider: NAME.to_owned(),
                        detail: e.to_string(),
                    }),
                })
                .flat_map(|parsed| {
                    let items: Vec<Result<StreamEvent, LlmError>> = match parsed {
                        Ok(events) => events.into_iter().map(Ok).collect(),
                        Err(e) => vec![Err(e)],
                    };
                    stream::iter(items)
                });
            Ok::<_, LlmError>(events)
        };
        stream::once(start).try_flatten().boxed()
    }

    fn plan_models(&self) -> futures::future::BoxFuture<'_, Result<Vec<String>, LlmError>> {
        Box::pin(async move {
            let mut cached = self.models.lock().await;
            if let Some(models) = cached.as_ref() {
                return Ok(models.clone());
            }
            let (token, account) = access(&self.client, &self.auth, &self.save).await?;
            let mut req = self
                .client
                .get(format!(
                    "{CHATGPT_MODELS_URL}?client_version={CHATGPT_CLIENT_VERSION}"
                ))
                .bearer_auth(token);
            if let Some(account) = account {
                req = req.header("chatgpt-account-id", account);
            }
            let response = req.send().await.map_err(|e| network(&e))?;
            let status = response.status().as_u16();
            let body = response.text().await.map_err(|e| network(&e))?;
            if !(200..300).contains(&status) {
                return Err(error_from_status(status, &body));
            }
            let models = parse_models(&body);
            if models.is_empty() {
                tracing::warn!(shape = %shape_of(&body), "ChatGPT's model list had no models");
                if let Ok(v) = serde_json::from_str::<Value>(&body) {
                    for m in v["models"].as_array().into_iter().flatten() {
                        tracing::warn!(slug = %m["slug"], visibility = %m["visibility"], min = %m["minimal_client_version"], "listed model");
                    }
                }
            }
            *cached = Some(models.clone());
            Ok(models)
        })
    }

    fn accepts_images<'a>(
        &'a self,
        _model: &'a str,
    ) -> futures::future::BoxFuture<'a, Option<bool>> {
        Box::pin(async { Some(true) })
    }
}

/// Model ids (`chatgpt/<slug>`) from the backend's model list, hidden ones left out, in its order.
fn parse_models(body: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(body).unwrap_or_default();
    let list = v
        .get("models")
        .or_else(|| v.get("data"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    list.iter()
        .filter(|m| m.get("visibility").and_then(Value::as_str) != Some("hide"))
        .filter_map(|m| {
            m.get("slug")
                .or_else(|| m.get("id"))
                .and_then(Value::as_str)
                .map(|s| format!("{}{s}", crate::constants::CHATGPT_MODEL_PREFIX))
        })
        .collect()
}

/// The keys of a JSON reply and of its lists' first items (no values), for the log.
fn shape_of(body: &str) -> String {
    fn keys(v: &Value) -> String {
        match v {
            Value::Object(m) => {
                let inner: Vec<String> = m
                    .iter()
                    .map(|(k, v)| match v {
                        Value::Array(a) => {
                            format!("{k}[{}]:{}", a.len(), a.first().map_or(String::new(), keys))
                        }
                        _ => k.clone(),
                    })
                    .collect();
                format!("{{{}}}", inner.join(","))
            }
            Value::Array(a) => format!("[{}]", a.len()),
            _ => "value".to_owned(),
        }
    }
    match serde_json::from_str::<Value>(body) {
        Ok(v) => keys(&v),
        Err(_) => format!("not JSON ({} bytes)", body.len()),
    }
}

/// The id used in messages about this provider.
pub const PROVIDER_ID: &str = ID;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ToolCall, ToolCallAssembler, ToolSpec};

    #[test]
    fn sign_in_url_carries_pkce_and_state() {
        let s = begin_sign_in().unwrap();
        assert!(s.url.starts_with(OPENAI_AUTHORIZE_URL));
        assert!(s.url.contains("code_challenge_method=S256"));
        assert!(s.url.contains(&format!("state={}", s.state)));
        assert!(
            !s.url.contains(&s.verifier),
            "the verifier never leaves the app"
        );
        assert!(
            s.url
                .contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback")
        );
    }

    #[test]
    fn account_id_comes_from_the_auth_claim() {
        let claims = json!({ OPENAI_AUTH_CLAIM: { "chatgpt_account_id": "acc-1" } });
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
        assert_eq!(
            account_id(&format!("h.{payload}.s")).as_deref(),
            Some("acc-1")
        );
        assert_eq!(account_id("not-a-jwt"), None);
    }

    #[test]
    fn conversation_becomes_responses_input() {
        let mut assistant = ChatMessage::new(ChatRole::Assistant, "Looking");
        assistant.tool_calls.push(ToolCall {
            id: "c1".into(),
            name: "read_file".into(),
            arguments: "{}".into(),
        });
        let request = ChatRequest {
            model: "chatgpt/gpt-5.3-codex".into(),
            messages: vec![
                ChatMessage::new(ChatRole::System, "Be brief"),
                ChatMessage::new(ChatRole::User, "hi"),
                assistant,
                ChatMessage::tool_result("c1", "contents"),
            ],
            max_tokens: 100,
            tools: vec![ToolSpec {
                name: "read_file".into(),
                description: "Read".into(),
                parameters: json!({"type": "object"}),
            }],
        };
        let b = body(&request, "gpt-5.3-codex");
        assert_eq!(b["instructions"], "Be brief");
        assert_eq!(b["store"], false);
        assert!(b.get("max_output_tokens").is_none() && b.get("tool_choice").is_none());
        assert_eq!(b["tools"][0]["name"], "read_file");
        assert!(b["tools"][0].get("strict").is_none());
        let input = b["input"].as_array().unwrap();
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "c1");
    }

    #[test]
    fn streamed_events_become_text_tool_calls_and_usage() {
        let mut seen = Reading::default();
        let mut text = String::new();
        let mut calls = ToolCallAssembler::default();
        let mut finished = None;
        let events = [
            r#"{"type":"response.output_text.delta","delta":"Hel"}"#,
            r#"{"type":"response.output_text.delta","delta":"lo"}"#,
            r#"{"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"c9","name":"list_dir","arguments":""}}"#,
            r#"{"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"pa"}"#,
            r#"{"type":"response.function_call_arguments.delta","output_index":1,"delta":"th\":\".\"}"}"#,
            r#"{"type":"response.output_item.done","output_index":1,"item":{"type":"function_call","call_id":"c9","name":"list_dir","arguments":"{\"path\":\".\"}"}}"#,
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":12,"output_tokens":5}}}"#,
        ];
        for e in events {
            for ev in parse_event(e, &mut seen).unwrap() {
                match ev {
                    StreamEvent::Text(t) => text.push_str(&t),
                    StreamEvent::ToolCall(d) => calls.push(d),
                    StreamEvent::Finished(r) => finished = r,
                    _ => {}
                }
            }
        }
        assert_eq!(text, "Hello");
        let calls = calls.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "c9");
        assert_eq!(calls[0].name, "list_dir");
        assert_eq!(
            calls[0].arguments, "{\"path\":\".\"}",
            "done doesn't repeat streamed args"
        );
        assert_eq!(finished.as_deref(), Some("tool_calls"));
    }

    #[test]
    fn a_call_sent_whole_in_done_keeps_its_arguments() {
        let mut seen = Reading::default();
        let mut calls = ToolCallAssembler::default();
        for e in [
            r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"c1","name":"glob","arguments":""}}"#,
            r#"{"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","call_id":"c1","name":"glob","arguments":"{\"p\":1}"}}"#,
        ] {
            for ev in parse_event(e, &mut seen).unwrap() {
                if let StreamEvent::ToolCall(d) = ev {
                    calls.push(d);
                }
            }
        }
        let calls = calls.finish();
        assert_eq!(
            (calls[0].name.as_str(), calls[0].arguments.as_str()),
            ("glob", "{\"p\":1}")
        );
    }

    #[test]
    fn the_model_list_skips_hidden_models() {
        let body = r#"{"models":[{"slug":"gpt-5.5","visibility":"list"},{"slug":"old","visibility":"hide"},{"slug":"gpt-5.4-mini"}]}"#;
        assert_eq!(
            parse_models(body),
            vec!["chatgpt/gpt-5.5", "chatgpt/gpt-5.4-mini"]
        );
        assert!(parse_models("not json").is_empty());
    }

    #[test]
    fn failures_are_errors() {
        let mut seen = Reading::default();
        let e = parse_event(
            r#"{"type":"response.failed","response":{"error":{"message":"model not found"}}}"#,
            &mut seen,
        )
        .unwrap_err();
        assert!(e.to_string().contains("model not found"));
    }
}
