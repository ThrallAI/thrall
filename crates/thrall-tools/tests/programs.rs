//! Programs on this computer: pictures from a (fake) Stable Diffusion web UI, what they refuse, and
//! what finding them reports when nothing is there.
// Integration tests are their own crate, so the in-tests unwrap exemption doesn't reach helpers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use futures::StreamExt;
use thrall_llm::{ChatMessage, ChatRequest, ChatRole, ImageRequest, LlmProvider};
use thrall_tools::programs::{Programs, find};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A web UI that answers every request with one picture, keeping what it was sent.
async fn fake_sd() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let keep = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            // Read until the body the request declares has arrived.
            loop {
                let n = sock.read(&mut chunk).await.unwrap_or(0);
                if n == 0 {
                    break;
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
                        break;
                    }
                }
            }
            keep.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf).to_string());
            let body = r#"{"images":["iVBORw0K"]}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(reply.as_bytes()).await;
        }
    });
    (base, seen)
}

#[tokio::test]
async fn pictures_come_from_the_web_ui_in_the_shape_asked_for() {
    let (base, seen) = fake_sd().await;
    let programs = Programs::new(std::env::temp_dir(), &base);
    let reply = programs
        .make_image(ImageRequest {
            model: "sd-webui".into(),
            prompt: "a red barn".into(),
            images: Vec::new(),
            aspect_ratio: Some("16:9".into()),
        })
        .await
        .unwrap();
    assert_eq!(reply.images.len(), 1);
    assert_eq!(reply.images[0].base64, "iVBORw0K");
    assert_eq!(reply.usage.cost_usd, Some(0.0));
    let sent = seen.lock().unwrap()[0].clone();
    assert!(sent.starts_with("POST /sdapi/v1/txt2img"), "{sent}");
    assert!(
        sent.contains("\"width\":1344") && sent.contains("\"height\":768"),
        "{sent}"
    );
    assert!(sent.contains("a red barn"), "{sent}");
}

#[tokio::test]
async fn programs_refuse_what_they_cant_do() {
    let programs = Programs::new(std::env::temp_dir(), "");
    let chat = programs
        .stream(ChatRequest {
            model: "llama".into(),
            messages: vec![ChatMessage::new(ChatRole::User, "hi")],
            max_tokens: 10,
            tools: Vec::new(),
        })
        .next()
        .await
        .unwrap()
        .unwrap_err();
    assert!(chat.to_string().contains("can't chat"), "{chat}");
    let no_ui = programs
        .make_image(ImageRequest {
            model: "sd-webui".into(),
            prompt: "x".into(),
            images: Vec::new(),
            aspect_ratio: None,
        })
        .await
        .unwrap_err();
    assert!(no_ui.to_string().contains("sd_webui_url"), "{no_ui}");
}

#[tokio::test]
async fn finding_programs_says_what_is_missing_and_how_to_add_it() {
    let empty = std::env::temp_dir().join("thrall-find-nothing-here");
    // Nothing listens on these ports in a test, so every server reads as not running.
    let found = find(&empty, "http://127.0.0.1:9/v1", "http://127.0.0.1:9").await;
    let jobs: Vec<&str> = found.iter().map(|f| f.job.as_str()).collect();
    assert!(jobs.contains(&"chat") && jobs.contains(&"transcribe"));
    assert!(jobs.contains(&"speak") && jobs.contains(&"pictures"));
    let ollama = found.iter().find(|f| f.name == "Ollama").unwrap();
    assert!(!ollama.ready && ollama.note.contains("Not running"));
    let pictures = found.iter().find(|f| f.job == "pictures").unwrap();
    assert!(!pictures.ready && pictures.note.contains("--api"));
}
