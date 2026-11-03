//! OpenRouter: an OpenAI-compatible chat API with streaming (server-sent events) and per-call cost.

use std::time::Duration;

use eventsource_stream::Eventsource;
use futures::future::BoxFuture;
use futures::{StreamExt, TryStreamExt, stream};
use serde::Deserialize;
use serde_json::json;

use crate::constants::{
    CHAT_COMPLETIONS_PATH, CONNECT_TIMEOUT_SECS, CREDITS_PATH, EMBEDDINGS_PATH,
    MAX_ERROR_BODY_CHARS, MAX_RETRIES, MAX_RETRY_AFTER_SECS, MODELS_PATH, OPENROUTER_APP_TITLE,
    READ_TIMEOUT_SECS, RETRY_BASE_DELAY_MS, SPEAK_PROMPT, SPEECH_SAMPLE_RATE, STREAM_DONE_MARKER,
};
use crate::error::LlmError;
use crate::provider::{Embedder, Embeddings, LlmProvider, ReplyStream};
use crate::types::{
    ChatMessage, ChatRequest, ImagePart, ImageReply, ImageRequest, ModelPrices, SpeechReply,
    SpeechRequest, StreamEvent, TokenUsage, ToolCallDelta, ToolSpec,
};

const NAME: &str = "OpenRouter";
const ID: &str = "openrouter";

/// The OpenRouter provider.
pub struct OpenRouter {
    client: reqwest::Client,
    api_key: String,
    endpoint: String,
    embeddings_endpoint: String,
    models_endpoint: String,
    credits_endpoint: String,
    /// What each model accepts and costs, fetched once from the model list.
    models: std::sync::Mutex<Option<std::collections::HashMap<String, ModelInfo>>>,
    /// Talking to a plain OpenAI-compatible API (OpenAI, Anthropic, Google) instead of
    /// OpenRouter: the name it gives the reply-length limit. Such APIs refuse OpenRouter's own
    /// fields and don't list what models accept or cost.
    standard: Option<&'static str>,
}

/// One model's entry in the model list.
#[derive(Debug, Clone, Copy)]
struct ModelInfo {
    accepts_images: bool,
    prices: Option<ModelPrices>,
}

impl OpenRouter {
    /// A provider using `api_key` against `base_url` (normally `https://openrouter.ai/api/v1`).
    pub fn new(api_key: String, base_url: &str) -> Result<Self, LlmError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .read_timeout(Duration::from_secs(READ_TIMEOUT_SECS))
            .build()
            .map_err(|e| network(&e))?;
        Ok(Self {
            client,
            api_key,
            endpoint: format!("{}{CHAT_COMPLETIONS_PATH}", base_url.trim_end_matches('/')),
            embeddings_endpoint: format!("{}{EMBEDDINGS_PATH}", base_url.trim_end_matches('/')),
            models_endpoint: format!("{}{MODELS_PATH}", base_url.trim_end_matches('/')),
            credits_endpoint: format!("{}{CREDITS_PATH}", base_url.trim_end_matches('/')),
            models: std::sync::Mutex::new(None),
            standard: None,
        })
    }

    /// A client for a plain OpenAI-compatible API at `base_url`, which calls the reply-length
    /// limit `max_tokens_field` (`max_tokens`, or `max_completion_tokens` at OpenAI).
    pub fn standard(
        api_key: String,
        base_url: &str,
        max_tokens_field: &'static str,
    ) -> Result<Self, LlmError> {
        let mut client = Self::new(api_key, base_url)?;
        client.standard = Some(max_tokens_field);
        Ok(client)
    }

    /// Ask for token counts the way this API understands: OpenRouter's `usage.include` (which
    /// also reports cost), or the standard `stream_options` (streamed calls only).
    fn ask_for_usage(&self, body: &mut serde_json::Value, streamed: bool) {
        match self.standard {
            None => body["usage"] = json!({ "include": true }),
            Some(_) if streamed => body["stream_options"] = json!({ "include_usage": true }),
            Some(_) => {}
        }
    }

    /// `model`'s entry in the model list (fetched once, then remembered). A standard API's list
    /// doesn't say, so there it's always unknown.
    async fn model_info(&self, model: &str) -> Option<ModelInfo> {
        if self.standard.is_some() {
            return None;
        }
        let known = self
            .models
            .lock()
            .ok()
            .and_then(|m| m.as_ref().map(|map| map.get(model).copied()));
        if let Some(answer) = known {
            return answer;
        }
        #[derive(Deserialize)]
        struct Architecture {
            #[serde(default)]
            input_modalities: Vec<String>,
        }
        #[derive(Deserialize)]
        struct Pricing {
            prompt: Option<String>,
            completion: Option<String>,
            image_output: Option<String>,
        }
        #[derive(Deserialize)]
        struct Model {
            id: String,
            architecture: Option<Architecture>,
            pricing: Option<Pricing>,
        }
        #[derive(Deserialize)]
        struct List {
            data: Vec<Model>,
        }
        let reply = self
            .client
            .get(&self.models_endpoint)
            .bearer_auth(&self.api_key)
            .send()
            .await
            .ok()?;
        let list: List = reply.json().await.ok()?;
        let number = |s: &Option<String>| s.as_deref().and_then(|v| v.parse::<f64>().ok());
        let map: std::collections::HashMap<String, ModelInfo> = list
            .data
            .into_iter()
            .map(|m| {
                let accepts_images = m
                    .architecture
                    .is_some_and(|a| a.input_modalities.iter().any(|x| x == "image"));
                let prices = m.pricing.and_then(|p| {
                    Some(ModelPrices {
                        prompt: number(&p.prompt)?,
                        completion: number(&p.completion)?,
                        image_output: number(&p.image_output),
                    })
                });
                (
                    m.id,
                    ModelInfo {
                        accepts_images,
                        prices,
                    },
                )
            })
            .collect();
        let answer = map.get(model).copied();
        if let Ok(mut cache) = self.models.lock() {
            *cache = Some(map);
        }
        answer
    }
}

impl Embedder for OpenRouter {
    fn embed<'a>(
        &'a self,
        model: &'a str,
        inputs: Vec<String>,
    ) -> BoxFuture<'a, Result<Embeddings, LlmError>> {
        Box::pin(async move {
            #[derive(Deserialize)]
            struct Item {
                embedding: Vec<f32>,
                #[serde(default)]
                index: usize,
            }
            #[derive(Deserialize)]
            struct Reply {
                data: Vec<Item>,
                usage: Option<WireEmbedUsage>,
            }
            #[derive(Deserialize)]
            struct WireEmbedUsage {
                #[serde(default)]
                total_tokens: u64,
                cost: Option<f64>,
            }
            let body = json!({ "model": model, "input": inputs });
            let response = send_with_retry(
                &self.client,
                &self.embeddings_endpoint,
                &self.api_key,
                &body,
            )
            .await?;
            let mut reply: Reply = response.json().await.map_err(|e| LlmError::BadResponse {
                provider: NAME.to_owned(),
                detail: e.to_string(),
            })?;
            reply.data.sort_by_key(|i| i.index);
            let usage = reply.usage;
            Ok(Embeddings {
                vectors: reply.data.into_iter().map(|i| i.embedding).collect(),
                tokens: usage.as_ref().map_or(0, |u| u.total_tokens),
                cost_usd: usage.and_then(|u| u.cost),
            })
        })
    }
}

impl LlmProvider for OpenRouter {
    fn name(&self) -> &str {
        NAME
    }

    fn accepts_images<'a>(&'a self, model: &'a str) -> BoxFuture<'a, Option<bool>> {
        Box::pin(async move { self.model_info(model).await.map(|m| m.accepts_images) })
    }

    fn prices<'a>(&'a self, model: &'a str) -> BoxFuture<'a, Option<ModelPrices>> {
        Box::pin(async move { self.model_info(model).await.and_then(|m| m.prices) })
    }

    fn credit_left(&self) -> BoxFuture<'_, Option<f64>> {
        Box::pin(async move {
            // {"data": {"total_credits": 10.0, "total_usage": 9.97}}
            #[derive(Deserialize)]
            struct Credits {
                total_credits: f64,
                total_usage: f64,
            }
            #[derive(Deserialize)]
            struct Reply {
                data: Credits,
            }
            let reply = self
                .client
                .get(&self.credits_endpoint)
                .bearer_auth(&self.api_key)
                .send()
                .await
                .ok()?;
            if !reply.status().is_success() {
                return None;
            }
            let credits: Reply = reply.json().await.ok()?;
            Some(credits.data.total_credits - credits.data.total_usage)
        })
    }

    fn speak(&self, request: SpeechRequest) -> BoxFuture<'_, Result<SpeechReply, LlmError>> {
        Box::pin(async move {
            // Streamed audio comes as raw 16-bit samples; it's wrapped into a WAV file below.
            let mut body = json!({
                "model": request.model,
                "messages": [
                    { "role": "system", "content": SPEAK_PROMPT },
                    { "role": "user", "content": request.text },
                ],
                "modalities": ["text", "audio"],
                "audio": { "voice": request.voice, "format": "pcm16" },
                "stream": true,
            });
            self.ask_for_usage(&mut body, true);
            let response =
                send_with_retry(&self.client, &self.endpoint, &self.api_key, &body).await?;
            let mut events = response.bytes_stream().eventsource();
            let mut pcm = Vec::new();
            let mut usage = None;
            while let Some(event) = events.next().await {
                let event = event.map_err(|e| LlmError::Network {
                    provider: NAME.to_owned(),
                    detail: e.to_string(),
                })?;
                if event.data.trim() == STREAM_DONE_MARKER {
                    break;
                }
                let (chunk, used) = parse_audio_event(&event.data)?;
                pcm.extend(chunk);
                if used.is_some() {
                    usage = used;
                }
            }
            if pcm.is_empty() {
                return Err(LlmError::BadResponse {
                    provider: NAME.to_owned(),
                    detail: "no audio came back".to_owned(),
                });
            }
            Ok(SpeechReply {
                wav: wav_from_pcm16(&pcm, SPEECH_SAMPLE_RATE),
                usage: usage.unwrap_or_default(),
            })
        })
    }

    fn make_image(&self, request: ImageRequest) -> BoxFuture<'_, Result<ImageReply, LlmError>> {
        Box::pin(async move {
            let mut message = ChatMessage::new(crate::types::ChatRole::User, request.prompt);
            message.images = request.images;
            let mut body = json!({
                "model": request.model,
                "messages": wire_messages(&[message]),
                "modalities": ["image", "text"],
            });
            self.ask_for_usage(&mut body, false);
            if let Some(ratio) = request.aspect_ratio {
                body["image_config"] = json!({ "aspect_ratio": ratio });
            }
            let response =
                send_with_retry(&self.client, &self.endpoint, &self.api_key, &body).await?;
            let text = response.text().await.map_err(|e| network(&e))?;
            parse_image_reply(&text)
        })
    }
    fn stream(&self, request: ChatRequest) -> ReplyStream {
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let endpoint = self.endpoint.clone();
        let mut body = json!({
            "model": request.model,
            "messages": wire_messages(&request.messages),
            "stream": true,
        });
        body[self.standard.unwrap_or("max_tokens")] = json!(request.max_tokens);
        self.ask_for_usage(&mut body, true);
        if let (None, Some(effort)) = (self.standard, request.thinking) {
            body["reasoning"] = json!({ "effort": effort });
        }
        if !request.tools.is_empty() {
            body["tools"] = wire_tools(&request.tools);
        }

        let start = async move {
            let response = send_with_retry(&client, &endpoint, &api_key, &body).await?;
            let events = response
                .bytes_stream()
                .eventsource()
                .map(|item| match item {
                    Ok(event) => parse_event_data(&event.data),
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
}

/// One streamed speech event: its audio bytes (decoded) and the usage, if it carries it.
fn parse_audio_event(data: &str) -> Result<(Vec<u8>, Option<TokenUsage>), LlmError> {
    use base64::Engine as _;
    let bad = |detail: String| LlmError::BadResponse {
        provider: NAME.to_owned(),
        detail,
    };
    let value: serde_json::Value = serde_json::from_str(data).map_err(|e| bad(e.to_string()))?;
    if let Some(message) = value
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
    {
        return Err(LlmError::Rejected {
            provider: NAME.to_owned(),
            status: 0,
            message: message.to_owned(),
        });
    }
    let mut audio = Vec::new();
    if let Some(choices) = value.get("choices").and_then(|c| c.as_array()) {
        for choice in choices {
            if let Some(chunk) = choice.pointer("/delta/audio/data").and_then(|d| d.as_str()) {
                audio.extend(
                    base64::engine::general_purpose::STANDARD
                        .decode(chunk)
                        .map_err(|e| bad(format!("audio wasn't base64: {e}")))?,
                );
            }
        }
    }
    let usage = value.get("usage").and_then(|u| {
        Some(TokenUsage {
            input_tokens: u.get("prompt_tokens")?.as_u64()?,
            output_tokens: u.get("completion_tokens")?.as_u64()?,
            cached_tokens: 0,
            cost_usd: u.get("cost").and_then(serde_json::Value::as_f64),
        })
    });
    Ok((audio, usage))
}

/// Raw mono 16-bit little-endian samples as a WAV file.
pub(crate) fn wav_from_pcm16(pcm: &[u8], sample_rate: u32) -> Vec<u8> {
    let data_len = u32::try_from(pcm.len()).unwrap_or(u32::MAX);
    let mut wav = Vec::with_capacity(pcm.len() + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // bytes per second
    wav.extend_from_slice(&2u16.to_le_bytes()); // bytes per sample
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);
    wav
}

/// The pictures (data URLs), text and usage in a non-streamed picture reply.
fn parse_image_reply(body: &str) -> Result<ImageReply, LlmError> {
    #[derive(Deserialize)]
    struct Url {
        url: String,
    }
    #[derive(Deserialize)]
    struct Image {
        image_url: Url,
    }
    #[derive(Deserialize)]
    struct Message {
        content: Option<String>,
        #[serde(default)]
        images: Vec<Image>,
    }
    #[derive(Deserialize)]
    struct Choice {
        message: Message,
    }
    #[derive(Deserialize)]
    struct Reply {
        choices: Vec<Choice>,
        usage: Option<WireUsage>,
    }
    let bad = |detail: String| LlmError::BadResponse {
        provider: NAME.to_owned(),
        detail,
    };
    let reply: Reply = serde_json::from_str(body).map_err(|e| bad(e.to_string()))?;
    let mut images = Vec::new();
    let mut text = String::new();
    for choice in reply.choices {
        text.push_str(choice.message.content.as_deref().unwrap_or_default());
        for image in choice.message.images {
            // data:<mime>;base64,<data>
            let Some((head, data)) = image.image_url.url.split_once(',') else {
                continue;
            };
            let mime = head
                .strip_prefix("data:")
                .and_then(|h| h.strip_suffix(";base64"))
                .unwrap_or("image/png");
            images.push(ImagePart {
                mime: mime.to_owned(),
                base64: data.to_owned(),
            });
        }
    }
    let usage = reply
        .usage
        .map_or_else(TokenUsage::default, WireUsage::into_usage);
    Ok(ImageReply {
        images,
        text: text.trim().to_owned(),
        usage,
    })
}

/// Messages in the OpenAI chat format.
fn wire_messages(messages: &[ChatMessage]) -> serde_json::Value {
    let items: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let mut item = json!({ "role": m.role });
            // An assistant message that only calls tools has no content.
            if !m.images.is_empty() || !m.documents.is_empty() || !m.audio.is_empty() {
                // Text with pictures or documents goes as content parts.
                let mut parts = vec![json!({ "type": "text", "text": m.content })];
                parts.extend(m.images.iter().map(|img| {
                    json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:{};base64,{}", img.mime, img.base64) },
                    })
                }));
                parts.extend(m.audio.iter().map(|clip| {
                    json!({
                        "type": "input_audio",
                        "input_audio": { "data": clip.base64, "format": clip.format },
                    })
                }));
                parts.extend(m.documents.iter().map(|doc| {
                    json!({
                        "type": "file",
                        "file": {
                            "filename": doc.name,
                            "file_data": format!("data:{};base64,{}", doc.mime, doc.base64),
                        },
                    })
                }));
                item["content"] = json!(parts);
            } else if !(m.content.is_empty() && !m.tool_calls.is_empty()) {
                item["content"] = json!(m.content);
            }
            if !m.tool_calls.is_empty() {
                item["tool_calls"] = m
                    .tool_calls
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.id,
                            "type": "function",
                            "function": { "name": c.name, "arguments": c.arguments },
                        })
                    })
                    .collect();
            }
            if let Some(id) = &m.tool_call_id {
                item["tool_call_id"] = json!(id);
            }
            item
        })
        .collect();
    serde_json::Value::Array(items)
}

/// Tool definitions in the OpenAI function-calling format.
fn wire_tools(tools: &[ToolSpec]) -> serde_json::Value {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                },
            })
        })
        .collect()
}

/// Send the request, retrying temporary failures with exponential backoff.
async fn send_with_retry(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
    body: &serde_json::Value,
) -> Result<reqwest::Response, LlmError> {
    let mut attempt = 0;
    loop {
        let result = client
            .post(endpoint)
            .bearer_auth(api_key)
            .header("X-Title", OPENROUTER_APP_TITLE)
            .json(body)
            .send()
            .await;
        let error = match result {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => error_from_response(response).await,
            Err(e) => network(&e),
        };
        if !error.is_retryable() || attempt >= MAX_RETRIES {
            return Err(error);
        }
        let delay = match &error {
            LlmError::RateLimited {
                retry_after_secs: Some(secs),
                ..
            } if *secs > MAX_RETRY_AFTER_SECS => return Err(error),
            LlmError::RateLimited {
                retry_after_secs: Some(secs),
                ..
            } => Duration::from_secs(*secs),
            _ => Duration::from_millis(RETRY_BASE_DELAY_MS.saturating_mul(1 << attempt)),
        };
        tracing::warn!(attempt, ?delay, %error, "retrying model call");
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

pub(crate) async fn error_from_response(response: reqwest::Response) -> LlmError {
    let status = response.status().as_u16();
    let retry_after_secs = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    let body = response.text().await.unwrap_or_default();
    error_from_status(status, retry_after_secs, &body)
}

fn error_from_status(status: u16, retry_after_secs: Option<u64>, body: &str) -> LlmError {
    match status {
        401 | 403 => LlmError::Unauthorized {
            provider: NAME.to_owned(),
            provider_id: ID.to_owned(),
        },
        402 => LlmError::OutOfCredit {
            provider: NAME.to_owned(),
        },
        429 => LlmError::RateLimited {
            provider: NAME.to_owned(),
            retry_after_secs,
        },
        500..=599 => LlmError::ServerError {
            provider: NAME.to_owned(),
            status,
        },
        _ => LlmError::Rejected {
            provider: NAME.to_owned(),
            status,
            message: error_message(body),
        },
    }
}

/// The provider's own error message from a JSON body, or the body itself, shortened.
fn error_message(body: &str) -> String {
    #[derive(Deserialize)]
    struct Envelope {
        error: Option<Inner>,
    }
    #[derive(Deserialize)]
    struct Inner {
        message: Option<String>,
    }
    let text = serde_json::from_str::<Envelope>(body)
        .ok()
        .and_then(|e| e.error)
        .and_then(|e| e.message)
        .unwrap_or_else(|| body.trim().to_owned());
    text.chars().take(MAX_ERROR_BODY_CHARS).collect()
}

pub(crate) fn network(e: &reqwest::Error) -> LlmError {
    LlmError::Network {
        provider: NAME.to_owned(),
        detail: if e.is_timeout() {
            "the connection timed out".to_owned()
        } else {
            e.to_string()
        },
    }
}

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    usage: Option<WireUsage>,
    error: Option<WireError>,
}

#[derive(Deserialize)]
struct Choice {
    delta: Option<Delta>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct Delta {
    content: Option<String>,
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Vec<WireToolCallDelta>,
}

#[derive(Deserialize)]
struct WireToolCallDelta {
    #[serde(default)]
    index: u32,
    id: Option<String>,
    function: Option<WireFunctionDelta>,
}

#[derive(Deserialize)]
struct WireFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    cost: Option<f64>,
    prompt_tokens_details: Option<WirePromptDetails>,
}

/// `usage.prompt_tokens_details`: how much of the request the provider had cached.
#[derive(Deserialize)]
struct WirePromptDetails {
    #[serde(default)]
    cached_tokens: u64,
}

impl WireUsage {
    fn into_usage(self) -> TokenUsage {
        TokenUsage {
            input_tokens: self.prompt_tokens,
            output_tokens: self.completion_tokens,
            cached_tokens: self.prompt_tokens_details.map_or(0, |d| d.cached_tokens),
            cost_usd: self.cost,
        }
    }
}

#[derive(Deserialize)]
struct WireError {
    message: Option<String>,
    code: Option<serde_json::Value>,
}

/// Turn one server-sent event's `data` into stream events.
fn parse_event_data(data: &str) -> Result<Vec<StreamEvent>, LlmError> {
    let data = data.trim();
    if data.is_empty() || data == STREAM_DONE_MARKER {
        return Ok(Vec::new());
    }
    let chunk: Chunk = serde_json::from_str(data).map_err(|e| LlmError::BadResponse {
        provider: NAME.to_owned(),
        detail: e.to_string(),
    })?;
    if let Some(error) = chunk.error {
        let code = error
            .code
            .as_ref()
            .and_then(serde_json::Value::as_u64)
            .and_then(|c| u16::try_from(c).ok())
            .unwrap_or(502);
        let message = error.message.unwrap_or_default();
        return Err(error_from_status(
            code,
            None,
            &json!({ "error": { "message": message } }).to_string(),
        ));
    }
    let mut events = Vec::new();
    for choice in chunk.choices {
        if let Some(delta) = choice.delta {
            if let Some(text) = delta.reasoning.filter(|t| !t.is_empty()) {
                events.push(StreamEvent::Reasoning(text));
            }
            if let Some(text) = delta.content.filter(|t| !t.is_empty()) {
                events.push(StreamEvent::Text(text));
            }
            for call in delta.tool_calls {
                let (name, arguments) = match call.function {
                    Some(f) => (f.name, f.arguments.unwrap_or_default()),
                    None => (None, String::new()),
                };
                events.push(StreamEvent::ToolCall(ToolCallDelta {
                    index: call.index,
                    id: call.id,
                    name,
                    arguments,
                }));
            }
        }
        if let Some(reason) = choice.finish_reason {
            events.push(StreamEvent::Finished(Some(reason)));
        }
    }
    if let Some(usage) = chunk.usage {
        events.push(StreamEvent::Usage(usage.into_usage()));
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_delta() {
        let events =
            parse_event_data(r#"{"choices":[{"delta":{"content":"Hel"},"finish_reason":null}]}"#)
                .unwrap();
        assert_eq!(events, vec![StreamEvent::Text("Hel".into())]);
    }

    #[test]
    fn reasoning_comes_before_text() {
        let events =
            parse_event_data(r#"{"choices":[{"delta":{"reasoning":"hmm","content":"ok"}}]}"#)
                .unwrap();
        assert_eq!(
            events,
            vec![
                StreamEvent::Reasoning("hmm".into()),
                StreamEvent::Text("ok".into())
            ]
        );
    }

    #[test]
    fn finish_and_usage_with_cost() {
        let events = parse_event_data(
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":3,"cost":0.0004}}"#,
        )
        .unwrap();
        assert_eq!(
            events,
            vec![
                StreamEvent::Finished(Some("stop".into())),
                StreamEvent::Usage(TokenUsage {
                    input_tokens: 12,
                    output_tokens: 3,
                    cached_tokens: 0,
                    cost_usd: Some(0.0004)
                })
            ]
        );
    }

    #[test]
    fn cached_tokens_are_read() {
        let events = parse_event_data(
            r#"{"choices":[],"usage":{"prompt_tokens":1000,"completion_tokens":5,"cost":0.001,"prompt_tokens_details":{"cached_tokens":800}}}"#,
        )
        .unwrap();
        let [StreamEvent::Usage(usage)] = events.as_slice() else {
            panic!("expected one usage event, got {events:?}");
        };
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.cached_tokens, 800);
    }

    #[test]
    fn speech_events_and_wav_files() {
        let (audio, usage) = parse_audio_event(
            r#"{"choices":[{"delta":{"audio":{"data":"AAABAA==","transcript":"Hi"}}}]}"#,
        )
        .unwrap();
        assert_eq!(audio, vec![0, 0, 1, 0]);
        assert!(usage.is_none());
        let (_, usage) = parse_audio_event(
            r#"{"choices":[],"usage":{"prompt_tokens":20,"completion_tokens":90,"cost":0.0003}}"#,
        )
        .unwrap();
        assert_eq!(usage.unwrap().cost_usd, Some(0.0003));
        let wav = wav_from_pcm16(&[0, 0, 1, 0], 24_000);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(wav.len(), 44 + 4);
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            24_000
        );
        assert!(parse_audio_event(r#"{"error":{"message":"no audio for you"}}"#).is_err());
    }

    #[test]
    fn picture_replies_are_read() {
        let reply = parse_image_reply(
            r#"{"choices":[{"message":{"content":"Here you go.","images":[{"type":"image_url","image_url":{"url":"data:image/png;base64,iVBORw0K"}}]}}],"usage":{"prompt_tokens":1300,"completion_tokens":1290,"cost":0.078}}"#,
        )
        .unwrap();
        assert_eq!(reply.images.len(), 1);
        assert_eq!(reply.images[0].mime, "image/png");
        assert_eq!(reply.images[0].base64, "iVBORw0K");
        assert_eq!(reply.text, "Here you go.");
        assert_eq!(reply.usage.cost_usd, Some(0.078));
        // No pictures (refused or failed) is not an error here; the caller says so.
        let empty =
            parse_image_reply(r#"{"choices":[{"message":{"content":"I can't do that."}}]}"#)
                .unwrap();
        assert!(empty.images.is_empty());
    }

    #[test]
    fn pictures_go_as_content_parts() {
        let mut message = ChatMessage::new(crate::types::ChatRole::User, "what's this?");
        message.images.push(crate::types::ImagePart {
            mime: "image/png".into(),
            base64: "AAAA".into(),
        });
        let wire = wire_messages(&[
            message,
            ChatMessage::new(crate::types::ChatRole::User, "plain"),
        ]);
        assert_eq!(
            wire[0]["content"][0],
            json!({ "type": "text", "text": "what's this?" })
        );
        assert_eq!(
            wire[0]["content"][1]["image_url"]["url"],
            "data:image/png;base64,AAAA"
        );
        let mut with_pdf = ChatMessage::new(crate::types::ChatRole::User, "read this");
        with_pdf.documents.push(crate::types::DocumentPart {
            name: "a.pdf".into(),
            mime: "application/pdf".into(),
            base64: "JVBE".into(),
        });
        let pdf = wire_messages(&[with_pdf]);
        assert_eq!(pdf[0]["content"][1]["type"], "file");
        assert_eq!(pdf[0]["content"][1]["file"]["filename"], "a.pdf");
        assert_eq!(
            pdf[0]["content"][1]["file"]["file_data"],
            "data:application/pdf;base64,JVBE"
        );
        // Messages without pictures stay plain text.
        assert_eq!(wire[1]["content"], "plain");
    }

    #[test]
    fn tool_call_pieces() {
        let events = parse_event_data(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"pa"}}]}}]}"#,
        )
        .unwrap();
        assert_eq!(
            events,
            vec![StreamEvent::ToolCall(ToolCallDelta {
                index: 0,
                id: Some("call_1".into()),
                name: Some("read_file".into()),
                arguments: "{\"pa".into(),
            })]
        );
    }

    #[test]
    fn wire_format_for_tool_turns() {
        use crate::types::{ChatRole, ToolCall};
        let mut assistant = ChatMessage::new(ChatRole::Assistant, "");
        assistant.tool_calls.push(ToolCall {
            id: "c1".into(),
            name: "list_dir".into(),
            arguments: "{}".into(),
        });
        let wire = wire_messages(&[assistant, ChatMessage::tool_result("c1", "a.txt")]);
        assert!(wire[0].get("content").is_none());
        assert_eq!(wire[0]["tool_calls"][0]["function"]["name"], "list_dir");
        assert_eq!(wire[1]["role"], "tool");
        assert_eq!(wire[1]["tool_call_id"], "c1");
    }

    #[test]
    fn done_marker_and_blank_are_empty() {
        assert!(parse_event_data("[DONE]").unwrap().is_empty());
        assert!(parse_event_data("  ").unwrap().is_empty());
    }

    #[test]
    fn mid_stream_error_becomes_an_error() {
        let err =
            parse_event_data(r#"{"error":{"message":"model overloaded","code":503}}"#).unwrap_err();
        assert!(matches!(err, LlmError::ServerError { status: 503, .. }));
    }

    #[test]
    fn garbage_is_a_bad_response() {
        assert!(matches!(
            parse_event_data("{not json"),
            Err(LlmError::BadResponse { .. })
        ));
    }

    #[test]
    fn status_mapping() {
        assert!(matches!(
            error_from_status(401, None, ""),
            LlmError::Unauthorized { .. }
        ));
        assert!(error_from_status(429, Some(2), "").is_retryable());
        assert!(error_from_status(502, None, "").is_retryable());
        let rejected = error_from_status(400, None, r#"{"error":{"message":"bad model"}}"#);
        assert!(!rejected.is_retryable());
        assert!(rejected.to_string().contains("bad model"));
    }
}
