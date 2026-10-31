//! fal.ai's queue API, for video models (Seedance): submit a job, follow it, fetch the result.

use std::time::Duration;

use serde::Deserialize;

use crate::constants::{FAL_DOWNLOAD_TIMEOUT_SECS, FAL_QUEUE_BASE, MAX_ERROR_BODY_CHARS};
use crate::error::LlmError;

const NAME: &str = "fal.ai";

/// Where a submitted job can be followed.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Job {
    /// fal's id for it.
    pub request_id: String,
    /// Polled for progress.
    pub status_url: String,
    /// The result, once done.
    pub response_url: String,
}

/// How a job is getting on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// Waiting its turn (position, when fal says).
    Queued(Option<u64>),
    /// Being made.
    Working,
    /// Finished.
    Done,
}

fn auth(key: &str) -> String {
    format!("Key {key}")
}

async fn failure(response: reqwest::Response) -> LlmError {
    let status = response.status().as_u16();
    let body: String = response
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .take(MAX_ERROR_BODY_CHARS)
        .collect();
    match status {
        // fal says why (a bad key, a key from a deleted account, no credit left): pass it on.
        401 | 403 => LlmError::Rejected {
            provider: NAME.to_owned(),
            status,
            message: format!(
                "it didn't accept the key{}. Check the key (and the account's credit at fal.ai), then run `thrall key set fal`.",
                if body.trim().is_empty() {
                    String::new()
                } else {
                    format!(" ({})", body.trim())
                }
            ),
        },
        429 => LlmError::RateLimited {
            provider: NAME.to_owned(),
            retry_after_secs: None,
        },
        s if s >= 500 => LlmError::ServerError {
            provider: NAME.to_owned(),
            status: s,
        },
        s => LlmError::Rejected {
            provider: NAME.to_owned(),
            status: s,
            message: body,
        },
    }
}

fn network(e: &reqwest::Error) -> LlmError {
    LlmError::Network {
        provider: NAME.to_owned(),
        detail: e.to_string(),
    }
}

/// Submit a job to `endpoint` (like `bytedance/seedance-2.0/text-to-video`) at `base` (normally
/// [`FAL_QUEUE_BASE`]).
pub async fn submit(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    endpoint: &str,
    body: &serde_json::Value,
) -> Result<Job, LlmError> {
    let response = client
        .post(format!("{}/{endpoint}", base.trim_end_matches('/')))
        .header(reqwest::header::AUTHORIZATION, auth(key))
        .json(body)
        .send()
        .await
        .map_err(|e| network(&e))?;
    if !response.status().is_success() {
        return Err(failure(response).await);
    }
    response.json().await.map_err(|e| LlmError::BadResponse {
        provider: NAME.to_owned(),
        detail: e.to_string(),
    })
}

/// The job's progress now.
pub async fn status(client: &reqwest::Client, key: &str, job: &Job) -> Result<Progress, LlmError> {
    #[derive(Deserialize)]
    struct Status {
        status: String,
        queue_position: Option<u64>,
        error: Option<String>,
    }
    let response = client
        .get(&job.status_url)
        .header(reqwest::header::AUTHORIZATION, auth(key))
        .send()
        .await
        .map_err(|e| network(&e))?;
    if !response.status().is_success() {
        return Err(failure(response).await);
    }
    let s: Status = response.json().await.map_err(|e| LlmError::BadResponse {
        provider: NAME.to_owned(),
        detail: e.to_string(),
    })?;
    if let Some(error) = s.error.filter(|e| !e.is_empty()) {
        return Err(LlmError::Rejected {
            provider: NAME.to_owned(),
            status: 0,
            message: error,
        });
    }
    Ok(match s.status.as_str() {
        "IN_QUEUE" => Progress::Queued(s.queue_position),
        "COMPLETED" => Progress::Done,
        _ => Progress::Working,
    })
}

/// A finished video job's video: where to download it, and the seed used.
pub async fn video_url(
    client: &reqwest::Client,
    key: &str,
    job: &Job,
) -> Result<(String, Option<i64>), LlmError> {
    #[derive(Deserialize)]
    struct Video {
        url: String,
    }
    #[derive(Deserialize)]
    struct Result_ {
        video: Option<Video>,
        seed: Option<i64>,
    }
    let response = client
        .get(&job.response_url)
        .header(reqwest::header::AUTHORIZATION, auth(key))
        .send()
        .await
        .map_err(|e| network(&e))?;
    if !response.status().is_success() {
        return Err(failure(response).await);
    }
    let text = response.text().await.map_err(|e| network(&e))?;
    let result: Result_ = serde_json::from_str(&text).map_err(|e| LlmError::BadResponse {
        provider: NAME.to_owned(),
        detail: e.to_string(),
    })?;
    let url = result
        .video
        .map(|v| v.url)
        .ok_or_else(|| LlmError::BadResponse {
            provider: NAME.to_owned(),
            detail: format!(
                "finished without a video: {}",
                text.chars().take(MAX_ERROR_BODY_CHARS).collect::<String>()
            ),
        })?;
    Ok((url, result.seed))
}

/// Download a finished file.
pub async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, LlmError> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(FAL_DOWNLOAD_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| network(&e))?;
    if !response.status().is_success() {
        return Err(failure(response).await);
    }
    Ok(response.bytes().await.map_err(|e| network(&e))?.to_vec())
}

/// The base URL of fal's queue.
pub fn queue_base() -> &'static str {
    FAL_QUEUE_BASE
}
