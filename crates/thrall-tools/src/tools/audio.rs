//! Audio and video: `speak` (text to speech), `transcribe` (speech to text) and
//! `video_transcript` (a video's captions, or its sound transcribed) — run by the agent, which
//! holds the AI provider — and `download_video` (yt-dlp). Also the yt-dlp and ffmpeg helpers
//! they share: fetching captions, and making long recordings small and cutting them into parts.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use thrall_gates::Effect;
use thrall_protocol::Risk;

use crate::constants::{
    CAPTION_PARAGRAPH_SECS, CAPTION_REPEAT_WINDOW, CAPTIONS_TEMP_PREFIX, CAPTIONS_TIMEOUT_SECS,
    DOWNLOAD_TIMEOUT_SECS, EXTRA_PROGRAM_DIRS, LISTEN_BITRATE, LISTEN_SAMPLE_RATE,
    RESULT_MAX_CHARS, SPLIT_TIMEOUT_SECS,
};
use crate::paths::{display, inside, resolve};
use crate::tool::{Cancel, Tool, ToolContext, ToolOutput, arg_str, cap, parse_args};

/// The `speak` tool's name.
pub const SPEAK: &str = "speak";
/// The `transcribe` tool's name.
pub const TRANSCRIBE: &str = "transcribe";
/// The `video_transcript` tool's name.
pub const VIDEO_TRANSCRIPT: &str = "video_transcript";

/// Read text aloud.
pub struct Speak;

impl Tool for Speak {
    fn name(&self) -> &'static str {
        SPEAK
    }
    fn description(&self) -> &'static str {
        "Read text aloud: makes a recording the user can play in the chat. voice (optional): \
         alloy, ash, ballad, coral, echo, fable, nova, onyx, sage, shimmer, verse. save_as also \
         puts the .wav in the working folder. Costs a fraction of a cent."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "What to say" },
                "voice": { "type": "string", "description": "Voice name (default from settings)" },
                "save_as": { "type": "string", "description": "Also save it in the working folder (a .wav name)" }
            },
            "required": ["text"]
        })
    }
    fn title(&self, args: &Value) -> String {
        let text: String = arg_str(args, "text").chars().take(50).collect();
        format!("Speaking: {text}")
    }
    fn run<'a>(&'a self, _ctx: &'a ToolContext, _args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async { ToolOutput::error("speaking isn't available here") })
    }
}

/// Turn a recording into text.
pub struct Transcribe;

impl Tool for Transcribe {
    fn name(&self) -> &'static str {
        TRANSCRIBE
    }
    fn description(&self) -> &'static str {
        "Turn a recording (mp3, wav, m4a, ogg, flac, aac, webm) into text — a voice note, a \
         meeting. Attached recordings are transcribed automatically. For a video online, use \
         video_transcript (it reads the video's own captions first)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The recording" }
            },
            "required": ["path"]
        })
    }
    fn title(&self, args: &Value) -> String {
        let path = std::path::Path::new(arg_str(args, "path"));
        let name = path.file_name().map_or_else(
            || arg_str(args, "path").to_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        format!("Listening to {name}")
    }
    fn run<'a>(&'a self, _ctx: &'a ToolContext, _args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async { ToolOutput::error("transcribing isn't available here") })
    }
}

/// Download a video (or its audio) with yt-dlp.
pub struct DownloadVideo;

#[derive(Deserialize)]
struct DownloadArgs {
    url: String,
    #[serde(default)]
    audio_only: bool,
    #[serde(default)]
    folder: Option<String>,
}

/// A program on the PATH, or where winget puts it: its links, its package folder, or a `bin`
/// folder one level inside that (winget's PATH entry only reaches programs started later).
fn find_program(stem: &str, winget_package: &str) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    // An app started from the Dock, a launcher or at log-in gets a short PATH: look where
    // Homebrew and per-person installs put programs too.
    if !cfg!(windows) {
        dirs.extend(EXTRA_PROGRAM_DIRS.iter().map(PathBuf::from));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".local/bin"));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let winget = PathBuf::from(local).join(r"Microsoft\WinGet");
        dirs.push(winget.join("Links"));
        if let Ok(packages) = std::fs::read_dir(winget.join("Packages")) {
            for package in packages
                .filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().starts_with(winget_package))
            {
                let package = package.path();
                if let Ok(inner) = std::fs::read_dir(&package) {
                    dirs.extend(inner.filter_map(Result::ok).map(|e| e.path().join("bin")));
                }
                dirs.push(package);
            }
        }
    }
    dirs.into_iter()
        .map(|d| d.join(&name))
        .find(|p| p.is_file())
}

/// yt-dlp on the PATH, or where winget puts it.
fn yt_dlp() -> Option<PathBuf> {
    find_program("yt-dlp", "yt-dlp.yt-dlp")
}

/// ffmpeg on the PATH, or where winget puts it.
pub fn ffmpeg() -> Option<PathBuf> {
    find_program("ffmpeg", "Gyan.FFmpeg")
}

/// A yt-dlp command for `url`, told where ffmpeg is when it isn't on the PATH.
fn yt_dlp_command(exe: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.arg("--no-playlist")
        .arg("--no-progress")
        .arg("--restrict-filenames");
    if let Some(ff) = ffmpeg().and_then(|f| f.parent().map(Path::to_path_buf)) {
        cmd.arg("--ffmpeg-location").arg(ff);
    }
    cmd.kill_on_drop(true);
    cmd
}

/// Run yt-dlp on `url` until it finishes, `secs` pass or the user stops it. The address goes
/// last, after `--`, so it can't be read as an option.
async fn run_yt_dlp(
    mut cmd: tokio::process::Command,
    url: &str,
    secs: u64,
    cancel: &Cancel,
) -> Result<std::process::Output, String> {
    cmd.arg("--").arg(url);
    thrall_core::process::no_window(cmd.as_std_mut());
    let run = tokio::time::timeout(Duration::from_secs(secs), cmd.output());
    let mut cancel = cancel.clone();
    let output = tokio::select! {
        out = run => out,
        () = cancel.cancelled() => return Err("stopped".to_owned()),
    };
    match output {
        Ok(Ok(o)) => Ok(o),
        Ok(Err(e)) => Err(format!("couldn't run yt-dlp: {e}")),
        Err(_) => Err(format!(
            "it took over {} minutes and was stopped",
            secs.div_ceil(60)
        )),
    }
}

/// Download `url` (or only its sound, as mp3) into `folder`; the files saved.
pub async fn download(
    url: &str,
    folder: &Path,
    audio_only: bool,
    cancel: &Cancel,
) -> Result<Vec<PathBuf>, String> {
    let exe = yt_dlp().ok_or("yt-dlp isn't installed (winget install yt-dlp.yt-dlp)")?;
    tokio::fs::create_dir_all(folder)
        .await
        .map_err(|e| format!("can't make the folder: {e}"))?;
    let mut cmd = yt_dlp_command(&exe);
    cmd.args(["--print", "after_move:filepath"])
        .arg("-o")
        .arg(folder.join("%(title).120B [%(id)s].%(ext)s"));
    if audio_only {
        cmd.args(["-x", "--audio-format", "mp3"]);
    }
    let output = run_yt_dlp(cmd, url, DOWNLOAD_TIMEOUT_SECS, cancel).await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let saved: Vec<PathBuf> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && Path::new(l).is_file())
        .map(PathBuf::from)
        .collect();
    if !output.status.success() || saved.is_empty() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "yt-dlp couldn't download it:\n{}",
            cap(err.into_owned(), RESULT_MAX_CHARS / 10)
        ));
    }
    Ok(saved)
}

/// A video's own captions, as text.
pub struct Captions {
    /// The video's file name without an extension: `Title [id]`.
    pub name: String,
    /// The captions' language code, as the site names it (`en`, `en-US`, `en-orig`).
    pub language: String,
    /// What is said, a paragraph with its time every half minute.
    pub text: String,
}

/// A video's own captions in `language` (the person's own first; the site's automatic ones when
/// there are none), fetched without downloading the video. `None` when it has none in that
/// language. Fetched into a temporary folder inside `folder`, which is removed again.
pub async fn fetch_captions(
    url: &str,
    folder: &Path,
    language: &str,
    cancel: &Cancel,
) -> Result<Option<Captions>, String> {
    let exe = yt_dlp().ok_or("yt-dlp isn't installed (winget install yt-dlp.yt-dlp)")?;
    let temp = folder.join(format!(
        "{CAPTIONS_TEMP_PREFIX}{}",
        thrall_core::ids::new_id("cap")
    ));
    tokio::fs::create_dir_all(&temp)
        .await
        .map_err(|e| format!("can't make the folder: {e}"))?;
    let result = fetch_captions_into(&exe, url, &temp, language, cancel).await;
    let _ = tokio::fs::remove_dir_all(&temp).await;
    result
}

async fn fetch_captions_into(
    exe: &Path,
    url: &str,
    temp: &Path,
    language: &str,
    cancel: &Cancel,
) -> Result<Option<Captions>, String> {
    let mut cmd = yt_dlp_command(exe);
    cmd.args(["--skip-download", "--write-subs", "--write-auto-subs"])
        .args(["--sub-format", "vtt/srt/best"])
        .arg("--sub-langs")
        .arg(format!("{language},{language}.*"))
        .arg("-o")
        .arg(temp.join("%(title).120B [%(id)s].%(ext)s"));
    let output = run_yt_dlp(cmd, url, CAPTIONS_TIMEOUT_SECS, cancel).await?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "yt-dlp couldn't read it:\n{}",
            cap(err.into_owned(), RESULT_MAX_CHARS / 10)
        ));
    }
    // Files are `Title [id].<language>.vtt`.
    let mut found: Vec<(String, String, PathBuf)> = std::fs::read_dir(temp)
        .map_err(|e| format!("can't read the captions: {e}"))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter_map(|p| {
            let ext = p.extension()?.to_string_lossy().to_lowercase();
            if ext != "vtt" && ext != "srt" {
                return None;
            }
            let stem = p.file_stem()?.to_string_lossy().into_owned();
            let (name, lang) = stem.rsplit_once('.')?;
            Some((name.to_owned(), lang.to_owned(), p))
        })
        .collect();
    // The language asked for exactly, then its original-language track, then any variant.
    let rank = |lang: &str| {
        if lang.eq_ignore_ascii_case(language) {
            0
        } else if lang.ends_with("-orig") {
            1
        } else {
            2
        }
    };
    found.sort_by(|a, b| rank(&a.1).cmp(&rank(&b.1)).then(a.1.cmp(&b.1)));
    for (name, lang, path) in found {
        let raw = std::fs::read_to_string(&path).map_err(|e| format!("can't read {lang}: {e}"))?;
        let text = captions_to_text(&raw);
        if !text.trim().is_empty() {
            return Ok(Some(Captions {
                name,
                language: lang,
                text,
            }));
        }
    }
    Ok(None)
}

/// A time in seconds as `m:ss`, or `h:mm:ss` from an hour.
pub fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `00:01:02.345` (or `01:02.345`, or SRT's `00:01:02,345`) in whole seconds.
fn caption_time(stamp: &str) -> Option<u64> {
    let whole = stamp.trim().split(['.', ',']).next()?;
    whole.split(':').try_fold(0u64, |acc, part| {
        Some(acc * 60 + part.trim().parse::<u64>().ok()?)
    })
}

/// A caption line without its markup (`<c>`, word timings), its entities decoded.
fn caption_line(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut in_tag = false;
    for c in line.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => plain.push(c),
            _ => {}
        }
    }
    let plain = plain
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A WebVTT or SRT caption file as plain text: a paragraph, starting with its time, every half
/// minute. Automatic captions repeat each line as the next one starts; the repeats are dropped.
pub fn captions_to_text(raw: &str) -> String {
    let mut out = String::new();
    let mut recent: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut start: u64 = 0;
    let mut paragraph: Option<u64> = None;
    let mut in_cue = false;
    for line in raw.lines() {
        if let Some((from, _)) = line.split_once("-->") {
            start = caption_time(from).unwrap_or(start);
            in_cue = true;
            continue;
        }
        if line.trim().is_empty() {
            in_cue = false;
            continue;
        }
        if !in_cue {
            continue;
        }
        let text = caption_line(line);
        if text.is_empty() || recent.contains(&text) {
            continue;
        }
        recent.push_back(text.clone());
        if recent.len() > CAPTION_REPEAT_WINDOW {
            recent.pop_front();
        }
        if paragraph.is_none_or(|p| start >= p + CAPTION_PARAGRAPH_SECS) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!("[{}] ", clock(start)));
            paragraph = Some(start);
        } else {
            out.push(' ');
        }
        out.push_str(&text);
    }
    out
}

/// A recording made small for listening (one channel, 16 kHz, a low bitrate: speech stays
/// clear) and cut into parts of `part_secs`, saved in `dir`; the parts in order.
pub async fn split_recording(
    ffmpeg: &Path,
    recording: &Path,
    dir: &Path,
    part_secs: u64,
) -> Result<Vec<PathBuf>, String> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("can't make a folder for the parts: {e}"))?;
    let mut cmd = tokio::process::Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
        .arg(recording)
        .args(["-vn", "-ac", "1", "-ar", LISTEN_SAMPLE_RATE])
        .args(["-c:a", "libmp3lame", "-b:a", LISTEN_BITRATE])
        .args(["-f", "segment", "-reset_timestamps", "1", "-segment_time"])
        .arg(part_secs.to_string())
        .arg(dir.join("part%03d.mp3"))
        .kill_on_drop(true);
    thrall_core::process::no_window(cmd.as_std_mut());
    let output = tokio::time::timeout(Duration::from_secs(SPLIT_TIMEOUT_SECS), cmd.output())
        .await
        .map_err(|_| {
            format!(
                "making it small took over {} minutes and was stopped",
                SPLIT_TIMEOUT_SECS / 60
            )
        })?
        .map_err(|e| format!("couldn't run ffmpeg: {e}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "ffmpeg couldn't read it:\n{}",
            cap(err.into_owned(), RESULT_MAX_CHARS / 10)
        ));
    }
    let mut parts: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("can't read the parts: {e}"))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "mp3"))
        .collect();
    parts.sort();
    Ok(parts)
}

/// The checks shared by `download_video` and `video_transcript`.
fn check_video(ctx: &ToolContext, args: &Value) -> Option<String> {
    let url = arg_str(args, "url");
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Some("give the video's web address (http or https)".to_owned());
    }
    let folder = resolve(
        &ctx.workspace,
        args.get("folder")
            .and_then(Value::as_str)
            .unwrap_or("downloads"),
    );
    if !inside(&ctx.workspace, &folder) {
        return Some("downloads go inside the working folder".to_owned());
    }
    if yt_dlp().is_none() {
        return Some(
            "yt-dlp isn't installed. The user can install it with: winget install yt-dlp.yt-dlp \
             (ffmpeg is needed too for best quality: winget install Gyan.FFmpeg)"
                .to_owned(),
        );
    }
    None
}

impl Tool for DownloadVideo {
    fn name(&self) -> &'static str {
        "download_video"
    }
    fn description(&self) -> &'static str {
        "Download a video (YouTube and most video sites) into the working folder with yt-dlp, \
         or just its audio (audio_only: true, as mp3). folder: where in the working folder \
         (default: downloads). Only for things the user may download. To know what a video \
         says, use video_transcript instead."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" },
                "audio_only": { "type": "boolean", "description": "Only the sound, as mp3" },
                "folder": { "type": "string", "description": "Folder inside the working folder (default: downloads)" }
            },
            "required": ["url"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Downloading {}", arg_str(args, "url"))
    }
    fn precheck(&self, ctx: &ToolContext, args: &Value) -> Option<String> {
        check_video(ctx, args)
    }
    fn effect(&self, _ctx: &ToolContext, args: &Value) -> Effect {
        Effect::Run {
            summary: format!(
                "Download {} from {}",
                if args.get("audio_only").and_then(Value::as_bool) == Some(true) {
                    "the audio"
                } else {
                    "the video"
                },
                arg_str(args, "url")
            ),
            risk: Risk::Low,
            grant: Some("download".to_owned()),
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: DownloadArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let folder = resolve(&ctx.workspace, a.folder.as_deref().unwrap_or("downloads"));
            match download(&a.url, &folder, a.audio_only, &ctx.cancel).await {
                Ok(saved) => {
                    let saved: Vec<String> =
                        saved.iter().map(|p| display(&ctx.workspace, p)).collect();
                    ToolOutput::ok(
                        format!("Downloaded: {}", saved.join(", ")),
                        format!("Saved {}", saved.join(", ")),
                    )
                }
                Err(e) => ToolOutput::error(e),
            }
        })
    }
}

/// What a video says: its own captions, or its sound transcribed. Run by the agent, which holds
/// the AI provider.
pub struct VideoTranscript;

impl Tool for VideoTranscript {
    fn name(&self) -> &'static str {
        VIDEO_TRANSCRIPT
    }
    fn description(&self) -> &'static str {
        "What a video says (YouTube and most video sites), as text. Uses the video's own captions \
         when it has them (free and instant); only when it has none does it download the sound \
         and transcribe it (long recordings are cut into parts). Saves the transcript in the \
         working folder. language: the captions' language code (default en). Use this rather \
         than download_video and transcribe."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" },
                "language": { "type": "string", "description": "Captions language code, e.g. en, de, ja (default en)" },
                "folder": { "type": "string", "description": "Folder inside the working folder for the transcript (default: downloads)" }
            },
            "required": ["url"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Getting the transcript of {}", arg_str(args, "url"))
    }
    fn precheck(&self, ctx: &ToolContext, args: &Value) -> Option<String> {
        check_video(ctx, args)
    }
    fn effect(&self, _ctx: &ToolContext, args: &Value) -> Effect {
        Effect::Run {
            summary: format!("Get the transcript of {}", arg_str(args, "url")),
            risk: Risk::Low,
            grant: Some("download".to_owned()),
        }
    }
    fn run<'a>(&'a self, _ctx: &'a ToolContext, _args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async { ToolOutput::error("transcripts aren't available here") })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_captions_lose_their_repeats_and_markup() {
        let vtt = "WEBVTT\nKind: captions\nLanguage: en\n\n\
            00:00:00.000 --> 00:00:02.000 align:start position:0%\n\
            hello<00:00:00.500><c> there</c>\n\n\
            00:00:02.000 --> 00:00:02.010 align:start position:0%\n\
            hello there\n\n\
            00:00:02.010 --> 00:00:04.000 align:start position:0%\n\
            hello there\n\
            how<00:00:02.500><c> are</c><c> you</c>\n\n\
            00:00:31.000 --> 00:00:33.000\n\
            fish &amp; chips\n";
        assert_eq!(
            captions_to_text(vtt),
            "[0:00] hello there how are you\n[0:31] fish & chips"
        );
    }

    #[test]
    fn srt_captions_read_too() {
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nFirst line\n\n\
                   2\n01:00:05,500 --> 01:00:07,000\nAn hour <i>later</i>\n";
        assert_eq!(
            captions_to_text(srt),
            "[0:01] First line\n[1:00:05] An hour later"
        );
    }

    #[test]
    fn clocks() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(600), "10:00");
        assert_eq!(clock(3725), "1:02:05");
    }
}
