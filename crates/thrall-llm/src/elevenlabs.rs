//! ElevenLabs: voices only. `speak` with model `eleven_multilingual_v2` (or another of theirs) and a
//! voice id; the audio comes back as raw 16-bit samples, wrapped into a WAV file.

use std::time::Duration;

use futures::future::BoxFuture;
use futures::{StreamExt, stream};
use serde_json::json;

use crate::constants::{
    CONNECT_TIMEOUT_SECS, ELEVENLABS_KEY_HEADER, ELEVENLABS_OUTPUT_FORMAT, ELEVENLABS_SPEECH_PATH,
    READ_TIMEOUT_SECS, SPEECH_SAMPLE_RATE,
};
use crate::error::LlmError;
use crate::openrouter::{error_from_response, network, wav_from_pcm16};
use crate::provider::{LlmProvider, ReplyStream};
use crate::types::{ChatRequest, SpeechReply, SpeechRequest, TokenUsage};

const NAME: &str = "ElevenLabs";

/// The ElevenLabs provider.
pub struct ElevenLabs {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl ElevenLabs {
    /// A provider using `api_key` against `base_url` (normally `https://api.elevenlabs.io/v1`).
    pub fn new(api_key: String, base_url: &str) -> Result<Self, LlmError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .read_timeout(Duration::from_secs(READ_TIMEOUT_SECS))
            .build()
            .map_err(|e| network(&e))?;
        Ok(Self {
            client,
            api_key,
            base_url: base_url.trim_end_matches('/').to_owned(),
        })
    }
}

impl LlmProvider for ElevenLabs {
    fn name(&self) -> &str {
        NAME
    }

    fn stream(&self, request: ChatRequest) -> ReplyStream {
        stream::once(async move {
            Err(LlmError::Rejected {
                provider: NAME.to_owned(),
                status: 0,
                message: format!(
                    "{} is a voice, not a chat model: pick it only for speaking",
                    request.model
                ),
            })
        })
        .boxed()
    }

    fn speak(&self, request: SpeechRequest) -> BoxFuture<'_, Result<SpeechReply, LlmError>> {
        Box::pin(async move {
            let url = format!(
                "{}{ELEVENLABS_SPEECH_PATH}{}?output_format={ELEVENLABS_OUTPUT_FORMAT}",
                self.base_url,
                request.voice.trim()
            );
            let response = self
                .client
                .post(url)
                .header(ELEVENLABS_KEY_HEADER, &self.api_key)
                .json(&json!({ "text": request.text, "model_id": request.model }))
                .send()
                .await
                .map_err(|e| network(&e).relabel(NAME, crate::constants::ELEVENLABS_ID))?;
            if !response.status().is_success() {
                return Err(error_from_response(response)
                    .await
                    .relabel(NAME, crate::constants::ELEVENLABS_ID));
            }
            let pcm = response
                .bytes()
                .await
                .map_err(|e| network(&e).relabel(NAME, crate::constants::ELEVENLABS_ID))?;
            if pcm.is_empty() {
                return Err(LlmError::BadResponse {
                    provider: NAME.to_owned(),
                    detail: "no audio came back".to_owned(),
                });
            }
            Ok(SpeechReply {
                wav: wav_from_pcm16(&pcm, SPEECH_SAMPLE_RATE),
                // Charged by the character on the person's ElevenLabs plan; not reported here.
                usage: TokenUsage::default(),
            })
        })
    }
}
