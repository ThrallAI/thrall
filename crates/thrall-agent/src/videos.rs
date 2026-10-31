//! `make_video`: Seedance 2.0 on fal.ai. Always asks first with the exact price (fal's published
//! token formula), follows the job with live progress on the step, and shows the video.

use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};
use thrall_core::constants::{UPLOADS_DIR, id_prefix};
use thrall_core::ids::new_id;
use thrall_gates::{Effect, Mode};
use thrall_llm::fal::{self, Progress};
use thrall_protocol::{Event, Risk, StepStatus, Usage};
use thrall_tools::paths::{display, inside, resolve};
use thrall_tools::{Cancel, ToolOutput};

use crate::constants::{
    FAL_KEY_NAME, FAL_MAX_WAIT_SECS, FAL_POLL_SECS, PICTURE_MAX_BYTES, SEEDANCE_FAST_MULTIPLIER,
    SEEDANCE_FPS_FACTOR, SEEDANCE_MAX_IMAGES, SEEDANCE_MAX_SECONDS, SEEDANCE_MIN_SECONDS,
    SEEDANCE_PER_1K_TOKENS, SEEDANCE_PER_1K_TOKENS_4K, SEEDANCE_TOKEN_DIVISOR,
};
use crate::error::AgentError;
use crate::state::AppState;
use crate::steps::gate_call;
use crate::turn::Emitter;

const ASPECTS: &[&str] = &["auto", "21:9", "16:9", "4:3", "1:1", "3:4", "9:16"];
const RESOLUTIONS: &[&str] = &["480p", "720p", "1080p", "4k"];

#[derive(Deserialize)]
struct Args {
    prompt: String,
    #[serde(default)]
    images: Vec<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    resolution: Option<String>,
    #[serde(default)]
    duration: Option<u32>,
    #[serde(default)]
    aspect_ratio: Option<String>,
    #[serde(default)]
    audio: Option<bool>,
    #[serde(default)]
    fast: bool,
    #[serde(default)]
    save_as: Option<String>,
}

/// The frame Seedance renders at, for the price formula. `auto` is priced at the widest frame,
/// so the price shown never undershoots.
pub fn seedance_frame(resolution: &str, aspect: &str) -> (u32, u32) {
    let aspect = if aspect == "auto" { "21:9" } else { aspect };
    let (a, b) = aspect
        .split_once(':')
        .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
        .unwrap_or((16, 9));
    let short = match resolution {
        "480p" => 480,
        "1080p" => 1080,
        "4k" => 2160,
        _ => 720,
    };
    let long = (f64::from(short) * f64::from(a.max(b)) / f64::from(a.min(b).max(1))).round();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let long = long as u32;
    if a >= b { (long, short) } else { (short, long) }
}

/// fal's price for a Seedance video, in US dollars: tokens = width × height × seconds × 24 / 1024,
/// at the published rate per thousand tokens.
pub fn seedance_price(resolution: &str, aspect: &str, seconds: u32, fast: bool) -> f64 {
    let (w, h) = seedance_frame(resolution, aspect);
    let tokens = f64::from(w) * f64::from(h) * f64::from(seconds) * SEEDANCE_FPS_FACTOR
        / SEEDANCE_TOKEN_DIVISOR;
    let mut rate = if resolution == "4k" {
        SEEDANCE_PER_1K_TOKENS_4K
    } else {
        SEEDANCE_PER_1K_TOKENS
    };
    if fast {
        rate *= SEEDANCE_FAST_MULTIPLIER;
    }
    tokens / 1000.0 * rate
}

/// Run a `make_video` call. `Ok(Err(..))` = the user said no, as for any step.
#[allow(clippy::type_complexity, clippy::too_many_lines)]
pub(crate) async fn make_video(
    emitter: &Emitter<'_>,
    state: &AppState,
    workspace: &Path,
    step_id: &str,
    args: &Value,
    mode: Mode,
    cancel: &Cancel,
) -> Result<Result<ToolOutput, (String, StepStatus, String)>, AgentError> {
    let fail = |m: String| Ok(Ok(ToolOutput::error(m)));
    let a: Args = match thrall_tools::tool::parse_args("make_video", args.clone()) {
        Ok(a) => a,
        Err(e) => return fail(e),
    };
    if a.prompt.trim().is_empty() {
        return fail("describe the video in prompt".to_owned());
    }
    if mode == Mode::Plan {
        return fail("plan mode is on: videos can be made once the plan is approved".to_owned());
    }
    let keys = state.keys.clone();
    let key = tokio::task::spawn_blocking(move || keys.get(FAL_KEY_NAME))
        .await
        .ok()
        .flatten();
    let Some(key) = key else {
        return fail(format!(
            "there's no fal.ai key yet. The user can make one at fal.ai (Dashboard → Keys) and \
             add it with: thrall key set {FAL_KEY_NAME}"
        ));
    };
    let resolution = a.resolution.clone().unwrap_or_else(|| "720p".to_owned());
    if !RESOLUTIONS.contains(&resolution.as_str()) {
        return fail(format!(
            "resolution must be one of {}",
            RESOLUTIONS.join(", ")
        ));
    }
    let aspect = a.aspect_ratio.clone().unwrap_or_else(|| "16:9".to_owned());
    if !ASPECTS.contains(&aspect.as_str()) {
        return fail(format!(
            "aspect_ratio must be one of {}",
            ASPECTS.join(", ")
        ));
    }
    let seconds = a
        .duration
        .unwrap_or(5)
        .clamp(SEEDANCE_MIN_SECONDS, SEEDANCE_MAX_SECONDS);
    let audio = a.audio.unwrap_or(true);
    let video_mode = match (a.mode.as_deref(), a.images.len()) {
        (_, 0) => "text",
        (Some("references"), _) => "references",
        (Some("start_frame") | None, 1..=2) => "start_frame",
        _ => "references",
    };
    if a.images.len() > SEEDANCE_MAX_IMAGES {
        return fail(format!("at most {SEEDANCE_MAX_IMAGES} pictures"));
    }
    // Pictures go to fal as data URIs.
    let mut refs = Vec::new();
    for raw in &a.images {
        let path = resolve(workspace, raw);
        let shown = display(workspace, &path);
        let Some(mime) = crate::extras::image_mime(&path) else {
            return fail(format!("{shown} isn't a PNG, JPEG, WebP or GIF picture"));
        };
        match std::fs::read(&path) {
            Ok(b) if b.len() as u64 <= PICTURE_MAX_BYTES => refs.push(format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(b)
            )),
            Ok(_) => return fail(format!("{shown} is too big")),
            Err(e) => return fail(format!("can't read {shown}: {e}")),
        }
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
                .is_none_or(|e| !e.eq_ignore_ascii_case("mp4"))
            {
                path.set_extension("mp4");
            }
            if !inside(workspace, &path) {
                return fail(format!("{name} is outside this session's folder"));
            }
            Some(path)
        }
        None => None,
    };

    // Ask, with the exact price.
    let price = seedance_price(&resolution, &aspect, seconds, a.fast);
    let what = match video_mode {
        "start_frame" => " from the given frame",
        "references" => " from the reference pictures",
        _ => "",
    };
    let summary = format!(
        "Make a {seconds} s {resolution} {aspect} video{what}{} with Seedance 2.0{} · ${price:.2}{}",
        if audio { " with sound" } else { "" },
        if a.fast { " fast" } else { "" },
        save_to
            .as_ref()
            .map(|p| format!(", saved as {}", display(workspace, p)))
            .unwrap_or_default()
    );
    let effect = Effect::AlwaysAsk {
        summary,
        risk: Risk::Low,
        diff: None,
    };
    if let Err(no) = gate_call(emitter, state, step_id, "make_video", &effect, mode, cancel).await?
    {
        return Ok(Err(no));
    }

    // Submit and follow.
    let endpoint = format!(
        "bytedance/seedance-2.0/{}{}",
        if a.fast { "fast/" } else { "" },
        match video_mode {
            "start_frame" => "image-to-video",
            "references" => "reference-to-video",
            _ => "text-to-video",
        }
    );
    let mut body = json!({
        "prompt": a.prompt,
        "resolution": resolution,
        "duration": seconds.to_string(),
        "aspect_ratio": aspect,
        "generate_audio": audio,
    });
    match video_mode {
        "start_frame" => {
            body["image_url"] = json!(refs[0]);
            if let Some(end) = refs.get(1) {
                body["end_image_url"] = json!(end);
            }
        }
        "references" => body["image_urls"] = json!(refs),
        _ => {}
    }
    let base = state.config.provider.fal_base_url.clone();
    let progress = |text: String| Event::StepOutput {
        step: step_id.to_owned(),
        text,
    };
    let job = match fal::submit(&state.http, &base, &key, &endpoint, &body).await {
        Ok(j) => j,
        Err(e) => return fail(format!("fal.ai didn't take the job: {e}")),
    };
    emitter
        .emit(progress("Sent to fal.ai.\n".to_owned()))
        .await?;
    let started = tokio::time::Instant::now();
    let mut last = String::new();
    let mut cancel = cancel.clone();
    loop {
        if started.elapsed() > Duration::from_secs(FAL_MAX_WAIT_SECS) {
            return fail(format!(
                "fal.ai hasn't finished after {} minutes; the job ({}) may still finish on fal's side",
                FAL_MAX_WAIT_SECS / 60,
                job.request_id
            ));
        }
        let now = match fal::status(&state.http, &key, &job).await {
            Ok(p) => p,
            Err(e) => return fail(format!("fal.ai: {e}")),
        };
        let line = match now {
            Progress::Queued(Some(n)) => format!("Queued (position {n})"),
            Progress::Queued(None) => "Queued".to_owned(),
            Progress::Working => "Generating…".to_owned(),
            Progress::Done => break,
        };
        if line != last {
            emitter.emit(progress(format!("{line}\n"))).await?;
            last = line;
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(FAL_POLL_SECS)) => {}
            () = cancel.cancelled() => {
                return fail(format!(
                    "stopped waiting. The job ({}) was already sent, so fal.ai may still finish and charge for it",
                    job.request_id
                ));
            }
        }
    }
    emitter.emit(progress("Downloading…\n".to_owned())).await?;
    let video = match fal::video_url(&state.http, &key, &job).await {
        Ok((url, _)) => match fal::download(&state.http, &url).await {
            Ok(b) => b,
            Err(e) => return fail(format!("couldn't download the video: {e}")),
        },
        Err(e) => return fail(format!("fal.ai: {e}")),
    };
    // fal doesn't report cost; its published formula is exact.
    emitter
        .record_side_usage(
            state,
            Usage {
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: Some(price),
            },
        )
        .await?;
    let Some(uploads) = state.my_dir().map(|d| d.join(UPLOADS_DIR)) else {
        return fail("there's no data folder to keep the video in".to_owned());
    };
    let name = save_to.as_ref().and_then(|p| p.file_name()).map_or_else(
        || "video.mp4".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let path = uploads.join(new_id(id_prefix::FILE)).join(&name);
    if let Err(e) = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, &video))
    {
        return fail(format!("couldn't keep the video: {e}"));
    }
    let mut content = format!(
        "Made a {seconds} s {resolution} video with Seedance 2.0 (${price:.2}). The user can watch it in the chat."
    );
    if let Some(copy) = &save_to {
        if let Some(parent) = copy.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(copy, &video) {
            Ok(()) => content.push_str(&format!(" Saved as {}.", display(workspace, copy))),
            Err(e) => content.push_str(&format!(" Couldn't save the copy in the folder: {e}.")),
        }
    }
    let mut output = ToolOutput::ok(content, format!("{seconds} s {resolution} · ${price:.2}"));
    output.files = vec![crate::extras::file_ref(&path)];
    Ok(Ok(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_follow_the_aspect() {
        assert_eq!(seedance_frame("720p", "16:9"), (1280, 720));
        assert_eq!(seedance_frame("720p", "9:16"), (720, 1280));
        assert_eq!(seedance_frame("1080p", "1:1"), (1080, 1080));
        // auto is priced at the widest frame.
        assert_eq!(seedance_frame("480p", "auto"), (1120, 480));
    }

    #[test]
    fn prices_follow_fals_formula() {
        // 1280 × 720 × 5 × 24 / 1024 = 108,000 tokens at $0.014 per 1,000.
        assert!((seedance_price("720p", "16:9", 5, false) - 1.512).abs() < 1e-9);
        assert!((seedance_price("720p", "16:9", 5, true) - 1.512 * 0.797).abs() < 1e-9);
        // 4k uses its own rate.
        let (w, h) = seedance_frame("4k", "16:9");
        let expected = f64::from(w) * f64::from(h) * 5.0 * 24.0 / 1024.0 / 1000.0 * 0.008;
        assert!((seedance_price("4k", "16:9", 5, false) - expected).abs() < 1e-9);
    }
}
