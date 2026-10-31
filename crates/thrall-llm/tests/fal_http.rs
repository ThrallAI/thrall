//! The fal.ai queue client against a small fake fal server.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use thrall_llm::fal::{self, Progress};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Serve: POST submit → job; GET /status → queued once, then done; GET /result → video URL;
/// GET /video.mp4 → bytes. Every request must carry `Authorization: Key test-key` (except the
/// download, which is a plain link).
async fn fake_fal() -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let polls = Arc::new(AtomicUsize::new(0));
    let (b, p) = (base.clone(), polls.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let (base, polls) = (b.clone(), p.clone());
            tokio::spawn(async move {
                let mut buf = vec![0; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                let line = req.lines().next().unwrap_or("").to_owned();
                let authed = req.to_lowercase().contains("authorization: key test-key");
                let (status, body): (&str, String) = if line.starts_with("POST /bytedance/") {
                    if authed {
                        (
                            "200 OK",
                            format!(
                                r#"{{"request_id":"r1","status_url":"{base}/status","response_url":"{base}/result"}}"#
                            ),
                        )
                    } else {
                        ("401 Unauthorized", "{}".to_owned())
                    }
                } else if line.starts_with("GET /status") {
                    let n = polls.fetch_add(1, Ordering::SeqCst);
                    let s = if n == 0 {
                        r#"{"status":"IN_QUEUE","queue_position":2}"#
                    } else {
                        r#"{"status":"COMPLETED"}"#
                    };
                    ("200 OK", s.to_owned())
                } else if line.starts_with("GET /result") {
                    (
                        "200 OK",
                        format!(r#"{{"video":{{"url":"{base}/video.mp4"}},"seed":42}}"#),
                    )
                } else if line.starts_with("GET /video.mp4") {
                    ("200 OK", "MP4DATA".to_owned())
                } else {
                    ("404 Not Found", String::new())
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    (base, polls)
}

#[tokio::test]
async fn a_video_job_is_submitted_followed_and_downloaded() {
    let (base, _) = fake_fal().await;
    let client = reqwest::Client::new();
    let body = serde_json::json!({ "prompt": "a cat", "duration": "5" });
    let job = fal::submit(
        &client,
        &base,
        "test-key",
        "bytedance/seedance-2.0/text-to-video",
        &body,
    )
    .await
    .unwrap();
    assert_eq!(job.request_id, "r1");
    assert_eq!(
        fal::status(&client, "test-key", &job).await.unwrap(),
        Progress::Queued(Some(2))
    );
    assert_eq!(
        fal::status(&client, "test-key", &job).await.unwrap(),
        Progress::Done
    );
    let (url, seed) = fal::video_url(&client, "test-key", &job).await.unwrap();
    assert_eq!(seed, Some(42));
    assert_eq!(fal::download(&client, &url).await.unwrap(), b"MP4DATA");
}

#[tokio::test]
async fn a_wrong_key_is_reported_as_such() {
    let (base, _) = fake_fal().await;
    let client = reqwest::Client::new();
    let err = fal::submit(
        &client,
        &base,
        "wrong",
        "bytedance/seedance-2.0/text-to-video",
        &serde_json::json!({}),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, thrall_llm::LlmError::Rejected { status: 401, .. }),
        "{err:?}"
    );
    assert!(err.to_string().contains("thrall key set fal"), "{err}");
}
