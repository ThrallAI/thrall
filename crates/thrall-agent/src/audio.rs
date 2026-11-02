//! Audio: recordings into text (attached recordings, the `transcribe` tool, the microphone
//! button), what a video says (`video_transcript`: its own captions first, its sound transcribed
//! only when it has none) and text into speech (`speak`). Long recordings are made small and cut
//! into parts with ffmpeg, each part transcribed in turn.

use std::path::Path;

use base64::Engine as _;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use thrall_core::constants::{UPLOADS_DIR, id_prefix, usage_role};
use thrall_core::ids::{new_id, now_ms};
use thrall_gates::{Effect, Mode};
use thrall_llm::{AudioPart, ChatMessage, ChatRequest, ChatRole, SpeechRequest, StreamEvent};
use thrall_protocol::{Event, StepStatus, Usage};
use thrall_store::UsageRecord;
use thrall_tools::paths::{display, inside, resolve};
use thrall_tools::tools::{TRANSCRIBE, VIDEO_TRANSCRIPT};
use thrall_tools::{Cancel, ToolOutput};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::constants::{
    AUDIO_DIRECT_MAX_BYTES, AUDIO_MAX_BYTES, AUDIO_MAX_PARTS, AUDIO_PART_SECS, BYTES_PER_MB,
    CAPTIONS_DEFAULT_LANGUAGE, SPEAK_MAX_CHARS, TRANSCRIBE_AUDIO_PROMPT, TRANSCRIPT_MAX_CHARS,
    TRANSCRIPT_MAX_TOKENS, TRANSCRIPT_SHOWN_MAX_CHARS, VIDEO_FOLDER_DEFAULT,
};
use crate::error::AgentError;
use crate::state::AppState;
use crate::steps::gate_call;
use crate::turn::Emitter;

/// The format name models take for a recording, from its extension.
pub fn audio_format(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    match ext.as_str() {
        "mp3" => Some("mp3"),
        "wav" => Some("wav"),
        "m4a" | "mp4a" => Some("m4a"),
        "ogg" | "oga" | "opus" => Some("ogg"),
        "flac" => Some("flac"),
        "aac" => Some("aac"),
        "webm" | "weba" => Some("webm"),
        _ => None,
    }
}

/// `more` added to `total`.
fn add_usage(total: &mut Usage, more: Usage) {
    total.input_tokens += more.input_tokens;
    total.output_tokens += more.output_tokens;
    total.cost_usd = match (total.cost_usd, more.cost_usd) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
    };
}

/// Tell whoever is following how far listening has got.
fn tell(progress: Option<&UnboundedSender<String>>, text: String) {
    if let Some(p) = progress {
        let _ = p.send(text);
    }
}

/// A recording's words, from the transcription model; what it cost. Recordings over
/// `AUDIO_DIRECT_MAX_BYTES` are made small and cut into parts first (with ffmpeg), each part
/// transcribed in turn and its time marked; `progress` hears how far it has got.
pub(crate) async fn listen(
    state: &AppState,
    path: &Path,
    progress: Option<&UnboundedSender<String>>,
) -> Result<(String, Usage), String> {
    let model = state.config.provider.transcribe_model.trim().to_owned();
    if model.is_empty() {
        return Err("no transcription model is set ([provider] transcribe_model)".to_owned());
    }
    let format =
        audio_format(path).ok_or_else(|| "it isn't a recording Thrall can read".to_owned())?;
    let size = std::fs::metadata(path)
        .map_err(|e| format!("can't read it: {e}"))?
        .len();
    if size <= AUDIO_DIRECT_MAX_BYTES {
        return listen_once(state, &model, path, format).await;
    }
    let Some(ffmpeg) = thrall_tools::tools::ffmpeg() else {
        if size <= AUDIO_MAX_BYTES {
            return listen_once(state, &model, path, format).await;
        }
        return Err(format!(
            "it's {} MB; recordings over {} MB need ffmpeg to cut them into parts (the user can \
             install it with: winget install Gyan.FFmpeg)",
            size / BYTES_PER_MB,
            AUDIO_MAX_BYTES / BYTES_PER_MB
        ));
    };
    let dir = std::env::temp_dir().join(format!("thrall-listen-{}", new_id(id_prefix::FILE)));
    let heard = listen_in_parts(state, &model, &ffmpeg, path, &dir, progress).await;
    let _ = std::fs::remove_dir_all(&dir);
    heard
}

/// A long recording made small, cut into parts in `dir` and transcribed part by part. When a
/// later part fails, the parts before it are kept and the rest is marked as missing.
async fn listen_in_parts(
    state: &AppState,
    model: &str,
    ffmpeg: &Path,
    path: &Path,
    dir: &Path,
    progress: Option<&UnboundedSender<String>>,
) -> Result<(String, Usage), String> {
    tell(
        progress,
        "Making the recording small for listening…\n".to_owned(),
    );
    let parts = thrall_tools::tools::split_recording(ffmpeg, path, dir, AUDIO_PART_SECS).await?;
    if parts.is_empty() {
        return Err("ffmpeg found no sound in it".to_owned());
    }
    if parts.len() > AUDIO_MAX_PARTS {
        let hours = AUDIO_MAX_PARTS as u64 * AUDIO_PART_SECS / 3600;
        return Err(format!(
            "it's over {hours} hours long; recordings up to {hours} hours can be transcribed"
        ));
    }
    let n = parts.len();
    let mut text = String::new();
    let mut usage = Usage::default();
    for (i, part) in parts.iter().enumerate() {
        if n > 1 {
            tell(progress, format!("Transcribing part {} of {n}…\n", i + 1));
        }
        match listen_once(state, model, part, "mp3").await {
            Ok((words, cost)) => {
                add_usage(&mut usage, cost);
                if n > 1 {
                    if !text.is_empty() {
                        text.push_str("\n\n");
                    }
                    let at = thrall_tools::tools::clock(i as u64 * AUDIO_PART_SECS);
                    text.push_str(&format!("[{at}]\n"));
                }
                text.push_str(&words);
            }
            Err(e) if i == 0 => return Err(e),
            Err(e) => {
                text.push_str(&format!(
                    "\n\n[Parts {} to {n} couldn't be transcribed: {e}]",
                    i + 1
                ));
                break;
            }
        }
    }
    Ok((text, usage))
}

/// `listen`, showing how far it has got on `step` as it goes.
async fn listen_shown(
    emitter: &Emitter<'_>,
    state: &AppState,
    path: &Path,
    step: &str,
) -> Result<(String, Usage), String> {
    let (tx, mut rx) = unbounded_channel();
    let work = async move {
        let heard = listen(state, path, Some(&tx)).await;
        drop(tx);
        heard
    };
    let show = async {
        while let Some(text) = rx.recv().await {
            let _ = emitter
                .emit(Event::StepOutput {
                    step: step.to_owned(),
                    text,
                })
                .await;
        }
    };
    futures::join!(work, show).0
}

/// One recording, sent whole to the transcription model.
async fn listen_once(
    state: &AppState,
    model: &str,
    path: &Path,
    format: &str,
) -> Result<(String, Usage), String> {
    let model = model.to_owned();
    let bytes = std::fs::read(path).map_err(|e| format!("can't read it: {e}"))?;
    let mut message = ChatMessage::new(ChatRole::User, TRANSCRIBE_AUDIO_PROMPT);
    message.audio.push(AudioPart {
        format: format.to_owned(),
        base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    });
    let request = ChatRequest {
        model: model.clone(),
        messages: vec![message],
        max_tokens: TRANSCRIPT_MAX_TOKENS,
        tools: Vec::new(),
    };
    let mut text = String::new();
    let mut usage = Usage::default();
    let mut stream = state.llm.stream(request);
    while let Some(item) = stream.next().await {
        match item {
            Ok(StreamEvent::Text(t)) => text.push_str(&t),
            Ok(StreamEvent::Usage(u)) => {
                usage = Usage {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cost_usd: u.cost_usd,
                }
            }
            Ok(_) => {}
            Err(e) => return Err(format!("{model}: {e}")),
        }
    }
    let text: String = text.trim().chars().take(TRANSCRIPT_MAX_CHARS).collect();
    if text.is_empty() {
        return Err(format!("{model} returned no text"));
    }
    Ok((text, usage))
}

/// Record usage outside a turn (the microphone button, a settings action) by `model`, against
/// `session` when there is one.
pub(crate) async fn record_loose_usage(
    state: &AppState,
    session: Option<&str>,
    model: &str,
    usage: Usage,
) -> Result<(), AgentError> {
    if usage == Usage::default() {
        return Ok(());
    }
    state
        .store
        .record_usage(UsageRecord {
            id: new_id(id_prefix::USAGE),
            user_id: state.user_id.clone(),
            session_id: session.map(str::to_owned),
            turn_id: None,
            provider: state.config.provider.kind.as_str().to_owned(),
            model: model.to_owned(),
            role: usage_role::CHAT.to_owned(),
            input_tokens: usage.input_tokens,
            cached_tokens: 0,
            output_tokens: usage.output_tokens,
            cost_usd: usage.cost_usd,
            at: now_ms(),
        })
        .await?;
    Ok(())
}

/// An attached recording, transcribed as a step; the text to add to the message.
pub(crate) async fn transcribe_attachment(
    emitter: &Emitter<'_>,
    state: &AppState,
    path: &Path,
) -> Result<String, AgentError> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let step = new_id(id_prefix::STEP);
    emitter
        .emit(Event::StepStarted {
            turn: emitter.turn_id.to_owned(),
            step: step.clone(),
            tool: "transcribe".to_owned(),
            title: format!("Listening to {name}"),
            helper: None,
        })
        .await?;
    let result = listen_shown(emitter, state, path, &step).await;
    let (status, summary, out) = match result {
        Ok((text, usage)) => {
            emitter.record_side_usage(state, usage).await?;
            (
                StepStatus::Ok,
                format!("Transcribed by {}", state.config.provider.transcribe_model),
                format!("\n\n[{name}, transcribed:]\n{text}"),
            )
        }
        Err(e) => (
            StepStatus::Error,
            format!("Couldn't transcribe it: {e}"),
            format!(
                "\n\n[Thrall couldn't transcribe {name}: {e}. Tell the user plainly; Retry tries again.]"
            ),
        ),
    };
    emitter
        .emit(Event::StepCompleted {
            step,
            status,
            summary,
            diff: None,
            files: Vec::new(),
        })
        .await?;
    Ok(out)
}

#[derive(Deserialize)]
struct TranscribeArgs {
    path: String,
}

/// The `transcribe` tool. A transcript too long to show whole is kept beside the recording
/// (a file change, so it asks like one).
pub(crate) async fn transcribe_tool(
    emitter: &Emitter<'_>,
    state: &AppState,
    workspace: &Path,
    step_id: &str,
    args: &Value,
    mode: Mode,
    cancel: &Cancel,
) -> Result<ToolOutput, AgentError> {
    let a: TranscribeArgs = match thrall_tools::tool::parse_args(TRANSCRIBE, args.clone()) {
        Ok(a) => a,
        Err(e) => return Ok(ToolOutput::error(e)),
    };
    let path = resolve(workspace, &a.path);
    let shown = display(workspace, &path);
    let (text, usage) = match listen_shown(emitter, state, &path, step_id).await {
        Ok(heard) => heard,
        Err(e) => return Ok(ToolOutput::error(format!("{shown}: {e}"))),
    };
    emitter.record_side_usage(state, usage).await?;
    let words = text.split_whitespace().count();
    let total = text.chars().count();
    if total <= TRANSCRIPT_SHOWN_MAX_CHARS {
        return Ok(ToolOutput::ok(
            format!("Transcript of {shown}:\n{text}"),
            format!("{words} words"),
        ));
    }
    let saved = path.with_extension("transcript.txt");
    let note = if inside(workspace, &saved) {
        let effect = Effect::Edit {
            path: display(workspace, &saved),
            diff: None,
        };
        match gate_call(emitter, state, step_id, TRANSCRIBE, &effect, mode, cancel).await? {
            Ok(()) => match std::fs::write(&saved, &text) {
                Ok(()) => format!(
                    "the whole transcript is saved as {}",
                    display(workspace, &saved)
                ),
                Err(e) => format!("the whole transcript couldn't be saved: {e}"),
            },
            Err((why, _, _)) => format!("the whole transcript wasn't saved: {why}"),
        }
    } else {
        "the whole transcript wasn't saved: the recording is outside the working folder".to_owned()
    };
    let head: String = text.chars().take(TRANSCRIPT_SHOWN_MAX_CHARS).collect();
    Ok(ToolOutput::ok(
        format!(
            "Transcript of {shown}: the first {TRANSCRIPT_SHOWN_MAX_CHARS} of {total} characters \
             ({note}):\n{head}"
        ),
        format!("{words} words"),
    ))
}

#[derive(Deserialize)]
struct VideoTranscriptArgs {
    url: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    folder: Option<String>,
}

/// A transcript saved in the working folder, and as much of it as fits for the model.
fn transcript_output(workspace: &Path, saved: &Path, text: &str, from: &str) -> ToolOutput {
    let note = match std::fs::write(saved, text) {
        Ok(()) => format!("saved as {}", display(workspace, saved)),
        Err(e) => format!("not saved ({e})"),
    };
    let words = text.split_whitespace().count();
    let total = text.chars().count();
    let head: String = text.chars().take(TRANSCRIPT_SHOWN_MAX_CHARS).collect();
    let cut = if total > TRANSCRIPT_SHOWN_MAX_CHARS {
        format!(
            " Only the first {TRANSCRIPT_SHOWN_MAX_CHARS} of {total} characters are below; read \
             the file for the rest."
        )
    } else {
        String::new()
    };
    ToolOutput::ok(
        format!("Transcript from {from}, {note}.{cut}\n\n{head}"),
        format!("From {from} · {words} words · {note}"),
    )
}

/// The `video_transcript` tool: the video's own captions when it has them; otherwise its sound,
/// downloaded and transcribed. `Ok(Err(..))` = not allowed, as for any step.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) async fn video_transcript_tool(
    emitter: &Emitter<'_>,
    state: &AppState,
    workspace: &Path,
    step_id: &str,
    args: &Value,
    effect: &Effect,
    mode: Mode,
    cancel: &Cancel,
) -> Result<Result<ToolOutput, (String, StepStatus, String)>, AgentError> {
    let a: VideoTranscriptArgs =
        match thrall_tools::tool::parse_args(VIDEO_TRANSCRIPT, args.clone()) {
            Ok(a) => a,
            Err(e) => return Ok(Ok(ToolOutput::error(e))),
        };
    if let Err(no) = gate_call(
        emitter,
        state,
        step_id,
        VIDEO_TRANSCRIPT,
        effect,
        mode,
        cancel,
    )
    .await?
    {
        return Ok(Err(no));
    }
    let language = a
        .language
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(CAPTIONS_DEFAULT_LANGUAGE);
    let folder = resolve(
        workspace,
        a.folder.as_deref().unwrap_or(VIDEO_FOLDER_DEFAULT),
    );
    if !inside(workspace, &folder) {
        return Ok(Ok(ToolOutput::error(
            "transcripts go inside the working folder",
        )));
    }
    let show = |text: String| Event::StepOutput {
        step: step_id.to_owned(),
        text,
    };
    emitter
        .emit(show("Looking for the video's own captions…\n".to_owned()))
        .await?;
    match thrall_tools::tools::fetch_captions(&a.url, &folder, language, cancel).await {
        Ok(Some(c)) => {
            let saved = folder.join(format!("{}.{}.transcript.txt", c.name, c.language));
            let from = format!("its own captions ({})", c.language);
            return Ok(Ok(transcript_output(workspace, &saved, &c.text, &from)));
        }
        Ok(None) => {
            emitter
                .emit(show(format!(
                    "It has no captions in {language}; downloading its sound to listen to.\n"
                )))
                .await?;
        }
        Err(e) if cancel.is_cancelled() => return Ok(Ok(ToolOutput::error(e))),
        Err(e) => {
            let first = e.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            emitter
                .emit(show(format!(
                    "Couldn't read its captions ({first}); downloading its sound to listen to.\n"
                )))
                .await?;
        }
    }
    let recording = match thrall_tools::tools::download(&a.url, &folder, true, cancel).await {
        Ok(saved) => saved.into_iter().find(|p| audio_format(p).is_some()),
        Err(e) => return Ok(Ok(ToolOutput::error(e))),
    };
    let Some(recording) = recording else {
        return Ok(Ok(ToolOutput::error("yt-dlp saved no recording")));
    };
    let (text, usage) = match listen_shown(emitter, state, &recording, step_id).await {
        Ok(heard) => heard,
        Err(e) => {
            return Ok(Ok(ToolOutput::error(format!(
                "{}: {e}",
                display(workspace, &recording)
            ))));
        }
    };
    emitter.record_side_usage(state, usage).await?;
    let saved = recording.with_extension("transcript.txt");
    let from = format!(
        "its sound, transcribed by {}",
        state.config.provider.transcribe_model
    );
    Ok(Ok(transcript_output(workspace, &saved, &text, &from)))
}

#[derive(Deserialize)]
struct SpeakArgs {
    text: String,
    #[serde(default)]
    voice: Option<String>,
    #[serde(default)]
    save_as: Option<String>,
}

/// The `speak` tool: a recording of the text, shown as a player on the step. Saving a copy in
/// the working folder (`save_as`) is a file change, so it asks like one. `Ok(Err(..))` = not
/// allowed, as for any step.
#[allow(clippy::type_complexity)]
pub(crate) async fn speak_tool(
    emitter: &Emitter<'_>,
    state: &AppState,
    workspace: &Path,
    step_id: &str,
    args: &Value,
    mode: Mode,
    cancel: &Cancel,
) -> Result<Result<ToolOutput, (String, StepStatus, String)>, AgentError> {
    let a: SpeakArgs = match thrall_tools::tool::parse_args("speak", args.clone()) {
        Ok(a) => a,
        Err(e) => return Ok(Ok(ToolOutput::error(e))),
    };
    if a.text.trim().is_empty() {
        return Ok(Ok(ToolOutput::error("say what to read aloud in text")));
    }
    if a.text.chars().count() > SPEAK_MAX_CHARS {
        return Ok(Ok(ToolOutput::error(format!(
            "that's too long to read in one go (up to {SPEAK_MAX_CHARS} characters); split it"
        ))));
    }
    let save_to = match a
        .save_as
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(name) => {
            let mut path = resolve(workspace, name);
            if path
                .extension()
                .is_none_or(|e| !e.eq_ignore_ascii_case("wav"))
            {
                path.set_extension("wav");
            }
            if !inside(workspace, &path) {
                return Ok(Ok(ToolOutput::error(format!(
                    "{name} is outside this session's folder"
                ))));
            }
            Some(path)
        }
        None => None,
    };
    if let Some(copy) = &save_to {
        let effect = Effect::Edit {
            path: display(workspace, copy),
            diff: None,
        };
        if let Err(no) = gate_call(emitter, state, step_id, "speak", &effect, mode, cancel).await? {
            return Ok(Err(no));
        }
    }
    let model = state.config.provider.speech_model.clone();
    let voice = a
        .voice
        .clone()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| state.config.provider.speech_voice.clone());
    let reply = match state
        .llm
        .speak(SpeechRequest {
            model: model.clone(),
            text: a.text.clone(),
            voice: voice.clone(),
        })
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Ok(Ok(ToolOutput::error(format!(
                "{model} couldn't speak it: {e}"
            ))));
        }
    };
    emitter
        .record_side_usage(
            state,
            Usage {
                input_tokens: reply.usage.input_tokens,
                output_tokens: reply.usage.output_tokens,
                cost_usd: reply.usage.cost_usd,
            },
        )
        .await?;
    let Some(uploads) = state.my_dir().map(|d| d.join(UPLOADS_DIR)) else {
        return Ok(Ok(ToolOutput::error(
            "there's no data folder to keep the recording in",
        )));
    };
    let name = save_to.as_ref().and_then(|p| p.file_name()).map_or_else(
        || "speech.wav".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let path = uploads.join(new_id(id_prefix::FILE)).join(&name);
    if let Err(e) = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, &reply.wav))
    {
        return Ok(Ok(ToolOutput::error(format!(
            "couldn't keep the recording: {e}"
        ))));
    }
    if let Some(copy) = &save_to {
        if let Some(parent) = copy.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(copy, &reply.wav) {
            return Ok(Ok(ToolOutput::error(format!(
                "couldn't save {}: {e}",
                display(workspace, copy)
            ))));
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let seconds = (reply.wav.len().saturating_sub(44)) as f64 / 48_000.0;
    let cost = reply
        .usage
        .cost_usd
        .map_or_else(|| "cost not reported".to_owned(), |c| format!("${c:.4}"));
    let mut content = format!(
        "Recorded {seconds:.0} s in the {voice} voice ({cost}); the user can play it in the chat."
    );
    if let Some(copy) = &save_to {
        content.push_str(&format!(" Saved as {}.", display(workspace, copy)));
    }
    let mut output = ToolOutput::ok(content, format!("{seconds:.0} s · {voice} · {cost}"));
    output.files = vec![crate::extras::file_ref(&path)];
    Ok(Ok(output))
}
