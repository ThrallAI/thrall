//! Programs on this computer that do Thrall's jobs for free, with nothing leaving it: whisper.cpp
//! transcribes (`program/whisper`), Piper reads aloud (`program/piper`), and a Stable Diffusion web
//! UI (AUTOMATIC1111, Forge) makes pictures (`program/sd-webui`). They're found, never installed:
//! the person installs the program and puts its models in the data folder's `models/`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use futures::future::BoxFuture;
use futures::{StreamExt, stream};
use serde::Deserialize;
use serde_json::json;
use thrall_core::constants::id_prefix;
use thrall_core::ids::new_id;
use thrall_llm::{
    ChatRequest, ImagePart, ImageReply, ImageRequest, LlmError, LlmProvider, ReplyStream,
    SpeechReply, SpeechRequest, StreamEvent, TokenUsage,
};
use thrall_protocol::LocalFound;
use tokio::io::AsyncWriteExt;

use crate::constants::{
    LM_STUDIO_URL, LOCAL_MODELS_DIR, PIPER_PROGRAM, PIPER_VOICE_EXT, PIPER_VOICES_DIR,
    PROGRAM_MAX_SECS, PROGRAM_PIPER, PROGRAM_PROBE_SECS, PROGRAM_SD_WEBUI, PROGRAM_WHISPER,
    SD_IMG2IMG_PATH, SD_MODELS_PATH, SD_SIZES, SD_STEPS, SD_TXT2IMG_PATH, WHISPER_MODEL_EXT,
    WHISPER_MODELS_DIR, WHISPER_PROGRAMS, WHISPER_SAMPLE_RATE,
};
use crate::tools::audio::find_program;

const NAME: &str = "This computer";

/// The programs on this computer, as a provider for `program/…` models.
#[derive(Clone)]
pub struct Programs {
    models_dir: PathBuf,
    sd_webui_url: String,
    client: reqwest::Client,
}

impl Programs {
    /// Programs whose models are in `models_dir`, and a picture web UI at `sd_webui_url`.
    pub fn new(models_dir: PathBuf, sd_webui_url: &str) -> Self {
        Self {
            models_dir,
            sd_webui_url: sd_webui_url.trim().trim_end_matches('/').to_owned(),
            client: reqwest::Client::new(),
        }
    }
}

/// The models folder inside `data_dir`.
pub fn models_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(LOCAL_MODELS_DIR)
}

fn refused(message: String) -> LlmError {
    LlmError::Rejected {
        provider: NAME.to_owned(),
        status: 0,
        message,
    }
}

/// whisper.cpp's program, if installed.
fn whisper() -> Option<PathBuf> {
    WHISPER_PROGRAMS
        .iter()
        .find_map(|p| find_program(p, "ggerganov.whisper"))
}

/// Piper's program, if installed.
fn piper() -> Option<PathBuf> {
    find_program(PIPER_PROGRAM, "rhasspy.piper")
}

/// The files ending `.ext` in `dir`, by name.
fn files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == ext))
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The file in `dir` called `name` (with or without its ending), else the first one there.
fn pick(dir: &Path, ext: &str, name: Option<&str>) -> Option<PathBuf> {
    let all = files(dir, ext);
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => all
            .into_iter()
            .find(|p| stem(p) == n || p.file_name().is_some_and(|f| f == n)),
        None => all.into_iter().next(),
    }
}

/// Run `cmd` until it finishes or `PROGRAM_MAX_SECS` pass, writing `input` to it first.
async fn run(
    mut cmd: tokio::process::Command,
    input: Option<&str>,
    what: &str,
) -> Result<std::process::Output, LlmError> {
    thrall_core::process::no_window(cmd.as_std_mut());
    cmd.kill_on_drop(true)
        .stdin(if input.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| refused(format!("couldn't start {what}: {e}")))?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin
            .write_all(text.as_bytes())
            .await
            .map_err(|e| refused(format!("couldn't give {what} the text: {e}")))?;
    }
    let output = tokio::time::timeout(
        Duration::from_secs(PROGRAM_MAX_SECS),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| refused(format!("{what} took too long and was stopped")))?
    .map_err(|e| refused(format!("{what} failed: {e}")))?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        let last = said
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        return Err(refused(format!(
            "{what} stopped with an error: {}",
            last.trim()
        )));
    }
    Ok(output)
}

/// A temporary folder for one program run; removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(what: &str) -> Result<Self, LlmError> {
        let dir = std::env::temp_dir().join(format!("thrall-{what}-{}", new_id(id_prefix::FILE)));
        std::fs::create_dir_all(&dir)
            .map_err(|e| refused(format!("couldn't make a work folder: {e}")))?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Programs {
    /// Transcribe a recording (`format` like `mp3`) with whisper.cpp and the model `name` (or the
    /// first one there).
    async fn transcribe(
        &self,
        name: Option<&str>,
        audio: &[u8],
        format: &str,
    ) -> Result<String, LlmError> {
        let exe = whisper().ok_or_else(|| {
            refused(
                "whisper.cpp isn't installed (its program is whisper-cli). Install it, or pick \
                 another transcription model."
                    .to_owned(),
            )
        })?;
        let dir = self.models_dir.join(WHISPER_MODELS_DIR);
        let model = pick(&dir, WHISPER_MODEL_EXT, name).ok_or_else(|| {
            refused(format!(
                "there's no whisper.cpp model{} in {}. Put one there (a ggml-*.bin file).",
                name.map(|n| format!(" called {n}")).unwrap_or_default(),
                dir.display()
            ))
        })?;
        let work = Scratch::new("whisper")?;
        let given = work.0.join(format!("in.{format}"));
        std::fs::write(&given, audio)
            .map_err(|e| refused(format!("couldn't save the recording: {e}")))?;
        // whisper.cpp wants 16 kHz mono sound; ffmpeg makes it from anything.
        let wav = match crate::tools::ffmpeg() {
            Some(ffmpeg) => {
                let wav = work.0.join("in16.wav");
                let mut cmd = tokio::process::Command::new(ffmpeg);
                cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
                    .arg(&given)
                    .args(["-ar", WHISPER_SAMPLE_RATE, "-ac", "1", "-c:a", "pcm_s16le"])
                    .arg(&wav);
                run(cmd, None, "ffmpeg").await?;
                wav
            }
            None if format == "wav" => given,
            None => {
                return Err(refused(
                    "whisper.cpp reads WAV recordings; ffmpeg is needed to turn this one into \
                     WAV (winget install Gyan.FFmpeg)."
                        .to_owned(),
                ));
            }
        };
        let out = work.0.join("out");
        let mut cmd = tokio::process::Command::new(exe);
        cmd.arg("-m")
            .arg(&model)
            .arg("-f")
            .arg(&wav)
            .args(["-otxt", "-nt", "-np", "-of"])
            .arg(&out);
        run(cmd, None, "whisper.cpp").await?;
        let text = std::fs::read_to_string(out.with_extension("txt"))
            .map_err(|e| refused(format!("whisper.cpp wrote no text: {e}")))?;
        Ok(text.trim().to_owned())
    }

    /// Read `text` aloud with Piper, in `voice` (a voice's name in the voices folder, or a path to
    /// one; else the first voice there).
    async fn piper_speak(&self, text: &str, voice: &str) -> Result<Vec<u8>, LlmError> {
        let exe = piper().ok_or_else(|| {
            refused("Piper isn't installed. Install it, or pick another voice model.".to_owned())
        })?;
        let dir = self.models_dir.join(PIPER_VOICES_DIR);
        let given = Path::new(voice.trim());
        let model = if given.is_file() {
            Some(given.to_path_buf())
        } else {
            pick(&dir, PIPER_VOICE_EXT, Some(voice)).or_else(|| pick(&dir, PIPER_VOICE_EXT, None))
        };
        let model = model.ok_or_else(|| {
            refused(format!(
                "there's no Piper voice in {}. Put one there (an .onnx file with its .onnx.json).",
                dir.display()
            ))
        })?;
        let work = Scratch::new("piper")?;
        let out = work.0.join("speech.wav");
        let mut cmd = tokio::process::Command::new(exe);
        cmd.arg("-m").arg(&model).arg("-f").arg(&out);
        run(cmd, Some(text), "Piper").await?;
        std::fs::read(&out).map_err(|e| refused(format!("Piper made no sound: {e}")))
    }

    /// Make a picture with the Stable Diffusion web UI (from `request.images[0]` when given).
    async fn sd_picture(&self, request: &ImageRequest) -> Result<Vec<ImagePart>, LlmError> {
        if self.sd_webui_url.is_empty() {
            return Err(refused(
                "no Stable Diffusion web UI address is set ([provider] sd_webui_url)".to_owned(),
            ));
        }
        let (width, height) = sd_size(request.aspect_ratio.as_deref());
        let mut body = json!({
            "prompt": request.prompt,
            "width": width,
            "height": height,
            "steps": SD_STEPS,
        });
        let path = match request.images.first() {
            Some(start) => {
                body["init_images"] = json!([start.base64]);
                SD_IMG2IMG_PATH
            }
            None => SD_TXT2IMG_PATH,
        };
        let unreachable = |e: reqwest::Error| {
            refused(format!(
                "couldn't reach the Stable Diffusion web UI at {} ({e}). Start it with --api.",
                self.sd_webui_url
            ))
        };
        let reply = self
            .client
            .post(format!("{}{path}", self.sd_webui_url))
            .json(&body)
            .send()
            .await
            .map_err(unreachable)?;
        if !reply.status().is_success() {
            return Err(refused(format!(
                "the Stable Diffusion web UI refused it ({}). Is it started with --api?",
                reply.status()
            )));
        }
        #[derive(Deserialize)]
        struct Made {
            #[serde(default)]
            images: Vec<String>,
        }
        let made: Made = reply.json().await.map_err(unreachable)?;
        Ok(made
            .images
            .into_iter()
            .map(|b64| ImagePart {
                mime: "image/png".to_owned(),
                base64: b64,
            })
            .collect())
    }
}

/// The picture size for a shape like `16:9` (square when unknown).
pub fn sd_size(ratio: Option<&str>) -> (u32, u32) {
    let ratio = ratio.map(str::trim).unwrap_or_default();
    SD_SIZES
        .iter()
        .find(|(r, ..)| *r == ratio)
        .or_else(|| SD_SIZES.first())
        .map_or((1024, 1024), |&(_, w, h)| (w, h))
}

/// Nothing to pay for a program's work.
fn free() -> TokenUsage {
    TokenUsage {
        cost_usd: Some(0.0),
        ..TokenUsage::default()
    }
}

impl LlmProvider for Programs {
    fn name(&self) -> &str {
        NAME
    }

    /// Only transcribing goes through chat: a recording in the request, its words back.
    fn stream(&self, request: ChatRequest) -> ReplyStream {
        let model = request.model.clone();
        let name = match model.split_once('/') {
            Some((program, name)) if program == PROGRAM_WHISPER => Some(name.to_owned()),
            None if model == PROGRAM_WHISPER => None,
            _ => {
                return stream::once(async move {
                    Err(refused(format!(
                        "program/{model} can't chat; local programs only transcribe, speak or \
                         make pictures"
                    )))
                })
                .boxed();
            }
        };
        // The stream must own what it uses.
        let programs = self.clone();
        let clip = request
            .messages
            .iter()
            .rev()
            .find_map(|m| m.audio.first().cloned());
        stream::once(async move {
            let clip =
                clip.ok_or_else(|| refused("there was no recording to transcribe".into()))?;
            let audio = base64::engine::general_purpose::STANDARD
                .decode(&clip.base64)
                .map_err(|e| refused(format!("the recording couldn't be read: {e}")))?;
            programs
                .transcribe(name.as_deref(), &audio, &clip.format)
                .await
        })
        .flat_map(|done| {
            let items = match done {
                Ok(text) => vec![
                    Ok(StreamEvent::Text(text)),
                    Ok(StreamEvent::Usage(free())),
                    Ok(StreamEvent::Finished(Some("stop".to_owned()))),
                ],
                Err(e) => vec![Err(e)],
            };
            stream::iter(items)
        })
        .boxed()
    }

    fn speak(&self, request: SpeechRequest) -> BoxFuture<'_, Result<SpeechReply, LlmError>> {
        Box::pin(async move {
            if request.model != PROGRAM_PIPER {
                return Err(refused(format!("program/{} can't speak", request.model)));
            }
            Ok(SpeechReply {
                wav: self.piper_speak(&request.text, &request.voice).await?,
                usage: free(),
            })
        })
    }

    fn make_image(&self, request: ImageRequest) -> BoxFuture<'_, Result<ImageReply, LlmError>> {
        Box::pin(async move {
            if request.model != PROGRAM_SD_WEBUI {
                return Err(refused(format!(
                    "program/{} can't make pictures",
                    request.model
                )));
            }
            Ok(ImageReply {
                images: self.sd_picture(&request).await?,
                text: String::new(),
                usage: free(),
            })
        })
    }
}

/// What's on this computer for each job: model servers (Ollama at `local_base_url`, LM Studio),
/// whisper.cpp and Piper with the models in `models_dir`, and a Stable Diffusion web UI at
/// `sd_webui_url`. Only looks: nothing is installed or downloaded.
pub async fn find(models_dir: &Path, local_base_url: &str, sd_webui_url: &str) -> Vec<LocalFound> {
    let mut found = Vec::new();
    // Model servers for chat (and reading pictures).
    let mut servers = vec![("Ollama", local_base_url.trim().to_owned())];
    if local_base_url.trim().trim_end_matches('/') != LM_STUDIO_URL {
        servers.push(("LM Studio", LM_STUDIO_URL.to_owned()));
    }
    for (name, url) in servers {
        if url.is_empty() {
            continue;
        }
        let looked = tokio::time::timeout(
            Duration::from_secs(PROGRAM_PROBE_SECS),
            thrall_llm::direct::check_local(&url),
        )
        .await;
        let in_use = url.trim_end_matches('/') == local_base_url.trim().trim_end_matches('/');
        let (ready, note, models) = match looked {
            Ok(Ok(models)) if !models.is_empty() && in_use => (
                true,
                format!("Running at {url} with {} model(s).", models.len()),
                models,
            ),
            Ok(Ok(models)) if !models.is_empty() => (
                false,
                format!("Running at {url}. To use it, set the local model server to that address."),
                models,
            ),
            Ok(Ok(_)) => (
                false,
                format!("Running at {url}, but it has no models yet."),
                Vec::new(),
            ),
            _ => (false, format!("Not running at {url}."), Vec::new()),
        };
        found.push(LocalFound {
            job: "chat".to_owned(),
            name: name.to_owned(),
            ready,
            note,
            models,
        });
    }
    // whisper.cpp for transcribing.
    let dir = models_dir.join(WHISPER_MODELS_DIR);
    let models: Vec<String> = files(&dir, WHISPER_MODEL_EXT)
        .iter()
        .map(|p| {
            format!(
                "{}{PROGRAM_WHISPER}/{}",
                thrall_llm::constants::PROGRAM_MODEL_PREFIX,
                stem(p)
            )
        })
        .collect();
    let (ready, note) = match (whisper(), models.is_empty()) {
        (Some(_), false) => (true, format!("Installed, with {} model(s).", models.len())),
        (Some(_), true) => (
            false,
            format!(
                "Installed, but no model: put a ggml-*.bin model in {}.",
                dir.display()
            ),
        ),
        (None, _) => (
            false,
            "Not installed (its program is whisper-cli).".to_owned(),
        ),
    };
    found.push(LocalFound {
        job: "transcribe".to_owned(),
        name: "whisper.cpp".to_owned(),
        ready,
        note,
        models,
    });
    // Piper for speaking.
    let dir = models_dir.join(PIPER_VOICES_DIR);
    let voices = files(&dir, PIPER_VOICE_EXT);
    let (ready, note) = match (piper(), voices.is_empty()) {
        (Some(_), false) => (
            true,
            format!(
                "Installed, with voices: {}. Pick one as the Voice.",
                voices
                    .iter()
                    .map(|v| stem(v))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        (Some(_), true) => (
            false,
            format!(
                "Installed, but no voice: put an .onnx voice (with its .onnx.json) in {}.",
                dir.display()
            ),
        ),
        (None, _) => (false, "Not installed.".to_owned()),
    };
    found.push(LocalFound {
        job: "speak".to_owned(),
        name: "Piper".to_owned(),
        ready,
        note,
        models: if ready {
            vec![format!(
                "{}{PROGRAM_PIPER}",
                thrall_llm::constants::PROGRAM_MODEL_PREFIX
            )]
        } else {
            Vec::new()
        },
    });
    // A Stable Diffusion web UI for pictures.
    let url = sd_webui_url.trim().trim_end_matches('/');
    let running = !url.is_empty()
        && reqwest::Client::new()
            .get(format!("{url}{SD_MODELS_PATH}"))
            .timeout(Duration::from_secs(PROGRAM_PROBE_SECS))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
    found.push(LocalFound {
        job: "pictures".to_owned(),
        name: "Stable Diffusion web UI".to_owned(),
        ready: running,
        note: if running {
            format!("Running at {url}.")
        } else {
            format!("Not running at {url} (AUTOMATIC1111 or Forge, started with --api).")
        },
        models: if running {
            vec![format!(
                "{}{PROGRAM_SD_WEBUI}",
                thrall_llm::constants::PROGRAM_MODEL_PREFIX
            )]
        } else {
            Vec::new()
        },
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picture_sizes_follow_the_shape_asked_for() {
        assert_eq!(sd_size(Some("16:9")), (1344, 768));
        assert_eq!(sd_size(Some("9:16")), (768, 1344));
        assert_eq!(sd_size(None), (1024, 1024));
        assert_eq!(sd_size(Some("7:5")), (1024, 1024));
    }

    #[test]
    fn models_are_picked_by_name_or_the_first_one_there() {
        let dir = std::env::temp_dir().join(format!("thrall-pick-{}", new_id(id_prefix::FILE)));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["ggml-small.bin", "ggml-base.en.bin", "notes.txt"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        assert_eq!(stem(&pick(&dir, "bin", None).unwrap()), "ggml-base.en");
        assert_eq!(
            stem(&pick(&dir, "bin", Some("ggml-small")).unwrap()),
            "ggml-small"
        );
        assert!(pick(&dir, "bin", Some("ggml-large")).is_none());
        assert_eq!(files(&dir, "bin").len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
