//! The OpenRouter client against a tiny local HTTP server: streaming, auth failure and retry.
// Integration tests are their own crate, so the in-tests unwrap exemption doesn't reach helpers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::StreamExt;
use thrall_llm::{
    ChatMessage, ChatRequest, ChatRole, LlmError, LlmProvider, OpenRouter, StreamEvent,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serve one canned HTTP response per connection, in order. Returns the base URL and a counter of
/// requests received.
async fn serve(responses: Vec<String>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            counter.fetch_add(1, Ordering::SeqCst);
            read_request(&mut socket).await;
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.ok();
        }
    });
    (base, hits)
}

/// Read headers and the declared body so the client sees its request fully consumed.
async fn read_request(socket: &mut tokio::net::TcpStream) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = socket.read(&mut chunk).await.unwrap();
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf).to_string();
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text[..end]
                .lines()
                .find_map(|l| {
                    let lower = l.to_ascii_lowercase();
                    lower
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            if buf.len() >= end + 4 + length {
                return;
            }
        }
    }
}

fn sse(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn status(code: u16, reason: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn request() -> ChatRequest {
    ChatRequest {
        model: "test/model".into(),
        messages: vec![ChatMessage::new(ChatRole::User, "hello")],
        max_tokens: 16,
        tools: Vec::new(),
        thinking: None,
    }
}

const STREAM: &str = concat!(
    ": OPENROUTER PROCESSING\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"cost\":0.0001}}\n\n",
    "data: [DONE]\n\n",
);

#[tokio::test]
async fn streams_text_and_usage() {
    let (base, _) = serve(vec![sse(STREAM)]).await;
    let provider = OpenRouter::new("sk-test".into(), &base).unwrap();
    let events: Vec<_> = provider.stream(request()).collect().await;
    let events: Vec<StreamEvent> = events.into_iter().map(Result::unwrap).collect();
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hello");
    assert!(events.iter().any(|e| matches!(
        e,
        StreamEvent::Usage(u) if u.input_tokens == 5 && u.output_tokens == 2
    )));
}

#[tokio::test]
async fn bad_key_is_not_retried() {
    let (base, hits) = serve(vec![status(401, "Unauthorized", "{}")]).await;
    let provider = OpenRouter::new("sk-wrong".into(), &base).unwrap();
    let events: Vec<_> = provider.stream(request()).collect().await;
    assert!(matches!(
        events.as_slice(),
        [Err(LlmError::Unauthorized { .. })]
    ));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn out_of_credit_says_so_and_is_not_retried() {
    let body = r#"{"error":{"code":402,"message":"Insufficient credits"}}"#;
    let (base, hits) = serve(vec![status(402, "Payment Required", body)]).await;
    let provider = OpenRouter::new("sk-test".into(), &base).unwrap();
    let events: Vec<_> = provider.stream(request()).collect().await;
    let [Err(error @ LlmError::OutOfCredit { .. })] = events.as_slice() else {
        panic!("expected OutOfCredit, got {events:?}");
    };
    assert!(error.to_string().contains("Top up"));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn server_error_is_retried_then_succeeds() {
    let (base, hits) = serve(vec![status(503, "Service Unavailable", "{}"), sse(STREAM)]).await;
    let provider = OpenRouter::new("sk-test".into(), &base).unwrap();
    let events: Vec<_> = provider.stream(request()).collect().await;
    assert!(events.iter().all(Result::is_ok));
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}
