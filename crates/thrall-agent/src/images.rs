//! `make_image`: make a picture or change the user's, with the picture-making model. It costs
//! money, so it always asks first with the price; the result is saved like an upload (so every
//! client can show it) and, when asked, in the working folder.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::Deserialize;
use serde_json::Value;
use thrall_core::constants::{UPLOADS_DIR, id_prefix};
use thrall_core::ids::new_id;
use thrall_gates::{Effect, Mode};
use thrall_llm::{ImagePart, ImageRequest, ModelPrices};
use thrall_protocol::{Risk, StepStatus, Usage};
use thrall_tools::paths::{display, resolve};
use thrall_tools::{Cancel, ToolOutput};

use crate::constants::{
    IMAGE_INPUT_TOKENS_ESTIMATE, IMAGE_INPUTS_MAX, IMAGE_OUTPUT_TOKENS_ESTIMATE, PICTURE_MAX_BYTES,
    VISION_MAX_TOKENS, VISION_PROMPT,
};
use crate::error::AgentError;
use crate::state::AppState;
use crate::steps::{gate_call, save_in_system};
use crate::turn::Emitter;

#[derive(Deserialize)]
struct Args {
    prompt: String,
    #[serde(default)]
    images: Vec<String>,
    #[serde(default)]
    aspect_ratio: Option<String>,
    #[serde(default)]
    save_as: Option<String>,
}

/// What the user is told a picture will cost, from the model's published prices: "about $0.08",
/// or None when the provider doesn't say.
pub fn estimate(prices: Option<ModelPrices>, inputs: usize, prompt_chars: usize) -> Option<f64> {
    let p = prices?;
    let input_tokens = inputs * IMAGE_INPUT_TOKENS_ESTIMATE + prompt_chars / 4;
    #[allow(clippy::cast_precision_loss)]
    let cost = p.prompt * input_tokens as f64
        + p.image_output.unwrap_or(p.completion) * IMAGE_OUTPUT_TOKENS_ESTIMATE as f64;
    Some(cost)
}

fn money(usd: f64) -> String {
    if usd < 0.01 {
        format!("${usd:.4}")
    } else {
        format!("${usd:.2}")
    }
}

/// Run a `make_image` call. `Ok(Err(..))` = the user said no (or it wasn't allowed), as for any
/// step.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) async fn make_image(
    emitter: &Emitter<'_>,
    state: &AppState,
    workspace: &Path,
    step_id: &str,
    args: &Value,
    mode: Mode,
    cancel: &Cancel,
) -> Result<Result<ToolOutput, (String, StepStatus, String)>, AgentError> {
    let a: Args = match thrall_tools::tool::parse_args("make_image", args.clone()) {
        Ok(a) => a,
        Err(e) => return Ok(Ok(ToolOutput::error(e))),
    };
    if a.prompt.trim().is_empty() {
        return Ok(Ok(ToolOutput::error(
            "say what to make or change in prompt",
        )));
    }
    if mode == Mode::Plan {
        return Ok(Ok(ToolOutput::error(
            "plan mode is on: pictures can be made once the plan is approved",
        )));
    }
    let model = state.config.provider.image_model.trim().to_owned();
    if model.is_empty() {
        return Ok(Ok(ToolOutput::error(
            "no picture-making model is set ([provider] image_model)",
        )));
    }
    if a.images.len() > IMAGE_INPUTS_MAX {
        return Ok(Ok(ToolOutput::error(format!(
            "at most {IMAGE_INPUTS_MAX} pictures can be given at once"
        ))));
    }
    // The pictures to start from.
    let mut inputs = Vec::new();
    let mut names = Vec::new();
    for raw in &a.images {
        let path = resolve(workspace, raw);
        let shown = display(workspace, &path);
        let Some(mime) = crate::extras::image_mime(&path) else {
            return Ok(Ok(ToolOutput::error(format!(
                "{shown} isn't a PNG, JPEG, WebP or GIF picture"
            ))));
        };
        match std::fs::metadata(&path) {
            Ok(m) if m.len() > PICTURE_MAX_BYTES => {
                return Ok(Ok(ToolOutput::error(format!("{shown} is too big"))));
            }
            Ok(_) => {}
            Err(e) => return Ok(Ok(ToolOutput::error(format!("can't read {shown}: {e}")))),
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return Ok(Ok(ToolOutput::error(format!("can't read {shown}: {e}")))),
        };
        inputs.push(ImagePart {
            mime: mime.to_owned(),
            base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        names.push(
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(shown),
        );
    }
    // A copy can go in any folder; the system's folders make the card High risk.
    let mut system = false;
    let save_to = match a
        .save_as
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(name) => {
            let path = resolve(workspace, name);
            match save_in_system(state, workspace, &path) {
                Ok(s) => system = s,
                Err(why) => return Ok(Ok(ToolOutput::error(why))),
            }
            Some(path)
        }
        None => None,
    };

    // Ask first, with the price.
    let price = estimate(
        state.llm.prices(&model).await,
        inputs.len(),
        a.prompt.chars().count(),
    )
    .map_or_else(
        || "price unknown".to_owned(),
        |c| format!("about {}", money(c)),
    );
    let what = if names.is_empty() {
        "Make a picture".to_owned()
    } else {
        format!("Change {}", names.join(", "))
    };
    let saving = save_to
        .as_ref()
        .map(|p| format!(", saved as {}", display(workspace, p)))
        .unwrap_or_default();
    let effect = Effect::AlwaysAsk {
        summary: format!("{what} with {model} · {price}{saving}"),
        risk: if system { Risk::High } else { Risk::Low },
        diff: None,
    };
    if let Err(no) = gate_call(emitter, state, step_id, "make_image", &effect, mode, cancel).await?
    {
        return Ok(Err(no));
    }

    let request = ImageRequest {
        model: model.clone(),
        prompt: a.prompt.clone(),
        images: inputs,
        aspect_ratio: a.aspect_ratio.clone(),
    };
    let mut cancel = cancel.clone();
    let reply = tokio::select! {
        reply = state.llm.make_image(request) => reply,
        () = cancel.cancelled() => {
            return Ok(Ok(ToolOutput::error("stopped before the picture was made")));
        }
    };
    let reply = match reply {
        Ok(r) => r,
        Err(e) => {
            return Ok(Ok(ToolOutput::error(format!(
                "{model} couldn't make it: {e}"
            ))));
        }
    };
    let cost = reply.usage.cost_usd;
    emitter
        .record_side_usage(
            state,
            Usage {
                input_tokens: reply.usage.input_tokens,
                output_tokens: reply.usage.output_tokens,
                cost_usd: cost,
            },
        )
        .await?;
    if reply.images.is_empty() {
        let said = if reply.text.is_empty() {
            String::new()
        } else {
            format!(" It said: {}", reply.text)
        };
        return Ok(Ok(ToolOutput::error(format!(
            "{model} didn't return a picture.{said}"
        ))));
    }

    // Save each picture like an upload, so every client can show it.
    let Some(uploads) = state.my_dir().map(|d| d.join(UPLOADS_DIR)) else {
        return Ok(Ok(ToolOutput::error(
            "there's no data folder to keep pictures in",
        )));
    };
    let stem = save_to.as_ref().and_then(|p| p.file_stem()).map_or_else(
        || "picture".to_owned(),
        |s| s.to_string_lossy().into_owned(),
    );
    let mut saved: Vec<PathBuf> = Vec::new();
    let mut copied: Option<PathBuf> = None;
    for (n, image) in reply.images.iter().enumerate() {
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&image.base64) else {
            continue;
        };
        let ext = match image.mime.as_str() {
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            "image/gif" => "gif",
            _ => "png",
        };
        let name = if reply.images.len() == 1 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem}-{}.{ext}", n + 1)
        };
        let folder = uploads.join(new_id(id_prefix::FILE));
        let path = folder.join(&name);
        if std::fs::create_dir_all(&folder)
            .and_then(|()| std::fs::write(&path, &bytes))
            .is_err()
        {
            continue;
        }
        if n == 0
            && let Some(copy) = &save_to
        {
            if let Some(parent) = copy.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::write(copy, &bytes) {
                Ok(()) => copied = Some(copy.clone()),
                Err(e) => tracing::warn!(%e, "couldn't save the picture in the working folder"),
            }
        }
        saved.push(path);
    }
    if saved.is_empty() {
        return Ok(Ok(ToolOutput::error("the picture couldn't be saved")));
    }
    let cost_text = cost
        .map(money)
        .unwrap_or_else(|| "cost not reported".to_owned());
    let places: Vec<String> = saved.iter().map(|p| p.display().to_string()).collect();
    let mut content = format!(
        "Made {} with {model} ({cost_text}). The user can see it in the chat, so don't repeat where \
         it's saved unless they ask. Saved at {}.",
        if saved.len() == 1 {
            "1 picture".to_owned()
        } else {
            format!("{} pictures", saved.len())
        },
        places.join(", ")
    );
    if let Some(copy) = &save_to {
        content.push_str(&format!(
            " Copy in the working folder: {}.",
            display(workspace, copy)
        ));
    }
    if !reply.text.is_empty() {
        content.push_str(&format!(" The model said: {}", reply.text));
    }
    let mut output = ToolOutput::ok(content, format!("Made with {model} · {cost_text}"));
    output.files = saved.iter().map(|p| crate::extras::file_ref(p)).collect();
    output.saved.extend(copied);
    Ok(Ok(output))
}

#[derive(Deserialize)]
struct ViewArgs {
    path: String,
    #[serde(default)]
    question: Option<String>,
}

/// Run a `view_image` call: the picture reader describes a picture file (or answers a question
/// about it); the picture is shown on the step.
pub(crate) async fn view_image(
    emitter: &Emitter<'_>,
    state: &AppState,
    workspace: &Path,
    args: &Value,
) -> Result<ToolOutput, AgentError> {
    let a: ViewArgs = match thrall_tools::tool::parse_args("view_image", args.clone()) {
        Ok(a) => a,
        Err(e) => return Ok(ToolOutput::error(e)),
    };
    let path = resolve(workspace, &a.path);
    let shown = display(workspace, &path);
    let Some(mime) = crate::extras::image_mime(&path) else {
        return Ok(ToolOutput::error(format!(
            "{shown} isn't a PNG, JPEG, WebP or GIF picture"
        )));
    };
    match std::fs::metadata(&path) {
        Ok(m) if m.len() > PICTURE_MAX_BYTES => {
            return Ok(ToolOutput::error(format!("{shown} is too big to look at")));
        }
        Ok(_) => {}
        Err(e) => return Ok(ToolOutput::error(format!("can't read {shown}: {e}"))),
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => return Ok(ToolOutput::error(format!("can't read {shown}: {e}"))),
    };
    let readers = emitter.picture_readers();
    if readers.is_empty() {
        return Ok(ToolOutput::error(
            "no picture model is set ([provider] vision_model)",
        ));
    }
    let ask = a
        .question
        .as_deref()
        .filter(|q| !q.trim().is_empty())
        .map_or_else(
            || "Describe the picture.".to_owned(),
            |q| {
                format!("Answer this about the picture, then describe anything else relevant:\n{q}")
            },
        );
    let mut message = thrall_llm::ChatMessage::new(
        thrall_llm::ChatRole::User,
        format!("{VISION_PROMPT}\n\n{ask}"),
    );
    message.images.push(ImagePart {
        mime: mime.to_owned(),
        base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
    });
    let mut errors = Vec::new();
    let mut answer = None;
    for reader in &readers {
        match emitter
            .read_pictures(reader, message.clone(), VISION_MAX_TOKENS)
            .await?
        {
            Ok(text) => {
                answer = Some((reader.clone(), text));
                break;
            }
            Err(e) => errors.push(format!("{reader}: {e}")),
        }
    }
    let Some((reader, text)) = answer else {
        return Ok(ToolOutput::error(format!(
            "couldn't look at {shown}: {}",
            errors.join("; ")
        )));
    };
    let mut output = ToolOutput::ok(
        format!("{shown}, as seen by {reader}:\n{text}"),
        format!("Read by {reader}"),
    );
    // Show the picture on the step: a copy where clients can fetch it.
    if let Some(uploads) = state.my_dir().map(|d| d.join(UPLOADS_DIR)) {
        let name = path.file_name().map_or_else(
            || "picture.png".to_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        let copy = uploads.join(new_id(id_prefix::FILE)).join(&name);
        if copy
            .parent()
            .is_some_and(|p| std::fs::create_dir_all(p).is_ok())
            && std::fs::write(&copy, &bytes).is_ok()
        {
            output.files = vec![crate::extras::file_ref(&copy)];
        }
    }
    Ok(output)
}
