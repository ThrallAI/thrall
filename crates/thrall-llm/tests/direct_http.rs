//! Suppliers reached directly, against a tiny local HTTP server: what a standard OpenAI-compatible
//! API is sent, ElevenLabs' voices, and the Test that lists a supplier's models.
// Integration tests are their own crate, so the in-tests unwrap exemption doesn't reach helpers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use futures::StreamExt;
use thrall_llm::{
    ChatMessage, ChatRequest, ChatRole, ElevenLabs, LlmError, LlmProvider, OpenRouter,
    SpeechRequest, StreamEvent, direct,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serve one canned response per connection, keeping each request as text.
async fn serve(responses: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let keep = seen.clone();
    tokio::spawn(async move {
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            keep.lock().unwrap().push(request);
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.ok();
        }
    });
    (base, seen)
}

/// Headers and the declared body.
async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = socket.read(&mut chunk).await.unwrap();
        if n == 0 {
            return String::from_utf8_lossy(&buf).to_string();
        }
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf).to_string();
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text[..end]
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            if buf.len() >= end + 4 + length {
                return text;
            }
        }
    }
}

fn reply(code: u16, kind: &str, body: &[u8]) -> String {
    let mut head = format!(
        "HTTP/1.1 {code} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    head.push_str(&String::from_utf8_lossy(body));
    head
}

#[tokio::test]
async fn a_standard_api_gets_its_own_fields_not_openrouters() {
    let stream = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\n",
        "data: [DONE]\n\n",
    );
    let (base, seen) = serve(vec![reply(200, "text/event-stream", stream.as_bytes())]).await;
    let openai = OpenRouter::standard("sk-test".into(), &base, "max_completion_tokens").unwrap();
    let events: Vec<_> = openai
        .stream(ChatRequest {
            model: "gpt-5".into(),
            messages: vec![ChatMessage::new(ChatRole::User, "hello")],
            max_tokens: 16,
            tools: Vec::new(),
        })
        .collect()
        .await;
    assert_eq!(events[0].as_ref().unwrap(), &StreamEvent::Text("Hi".into()));
    assert!(events.iter().any(|e| matches!(
        e,
        Ok(StreamEvent::Usage(u)) if u.input_tokens == 5 && u.cost_usd.is_none()
    )));
    let sent = seen.lock().unwrap()[0].clone();
    assert!(sent.contains("\"max_completion_tokens\":16"), "{sent}");
    assert!(sent.contains("\"include_usage\":true"), "{sent}");
    assert!(
        !sent.contains("\"max_tokens\"") && !sent.contains("\"include\":true"),
        "{sent}"
    );
    // An unknown model's pictures and prices aren't guessed.
    assert_eq!(openai.accepts_images("gpt-5").await, None);
}

#[tokio::test]
async fn elevenlabs_reads_aloud_with_the_voice_asked_for() {
    let pcm = [1u8, 0, 2, 0];
    let (base, seen) = serve(vec![reply(200, "audio/pcm", &pcm)]).await;
    let voices = ElevenLabs::new("xi-test".into(), &base).unwrap();
    let speech = voices
        .speak(SpeechRequest {
            model: "eleven_multilingual_v2".into(),
            text: "Good morning".into(),
            voice: "voice123".into(),
        })
        .await
        .unwrap();
    assert_eq!(&speech.wav[..4], b"RIFF");
    assert_eq!(&speech.wav[44..], &pcm);
    let sent = seen.lock().unwrap()[0].clone();
    assert!(
        sent.starts_with("POST /text-to-speech/voice123?output_format=pcm_24000"),
        "{sent}"
    );
    assert!(
        sent.to_ascii_lowercase().contains("xi-api-key: xi-test"),
        "{sent}"
    );
    assert!(
        sent.contains("\"model_id\":\"eleven_multilingual_v2\""),
        "{sent}"
    );
}

#[tokio::test]
async fn the_test_lists_a_suppliers_models_or_says_the_key_is_wrong() {
    let list = br#"{"data":[{"id":"gpt-5"},{"id":"text-embedding-3-small"}]}"#;
    let (base, seen) = serve(vec![
        reply(200, "application/json", list),
        reply(401, "application/json", b"{}"),
    ])
    .await;
    let openai = direct::supplier("openai").unwrap();
    let models = direct::check(openai, "sk-test", &base).await.unwrap();
    assert_eq!(
        models,
        vec![
            "direct/openai/gpt-5",
            "direct/openai/text-embedding-3-small"
        ]
    );
    assert!(seen.lock().unwrap()[0].starts_with("GET /models"));
    let e = direct::check(openai, "sk-wrong", &base).await.unwrap_err();
    assert!(matches!(e, LlmError::Unauthorized { .. }));
    assert!(e.to_string().contains("OpenAI") && e.to_string().contains("key set openai"));
}
