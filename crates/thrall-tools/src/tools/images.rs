//! `make_image`: make a picture, or change pictures the user gave. It needs the AI provider and
//! asks first with the price, so Thrall's agent runs it; this declares it for the model.

use futures::future::BoxFuture;
use serde_json::{Value, json};
use thrall_gates::Effect;
use thrall_protocol::Risk;

use crate::tool::{Tool, ToolContext, ToolOutput, arg_str};

/// The tool's name.
pub const MAKE_IMAGE: &str = "make_image";

/// Make or edit a picture.
pub struct MakeImage;

impl Tool for MakeImage {
    fn name(&self) -> &'static str {
        MAKE_IMAGE
    }
    fn description(&self) -> &'static str {
        "Make a picture from a description, or change pictures the user gave (\"make me look \
         younger\", \"put a hat on the dog\", \"turn this into a watercolour\"). Pass the pictures \
         to change in images (paths, e.g. where an attached picture is saved). Each picture costs \
         money: the user sees the price and approves it first. The result is shown to the user; \
         save_as also puts a copy in the working folder."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "What to make, or exactly how to change the pictures. Be specific."
                },
                "images": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Pictures to change or use as a reference (paths). Leave out to make a new picture."
                },
                "aspect_ratio": {
                    "type": "string",
                    "description": "Shape, only when the user asked: 1:1, 16:9, 9:16, 4:3, 3:4, 3:2 or 2:3"
                },
                "save_as": {
                    "type": "string",
                    "description": "Also save it in the working folder under this name (e.g. cat-hat.png)"
                }
            },
            "required": ["prompt"]
        })
    }
    fn title(&self, args: &Value) -> String {
        if args
            .get("images")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
        {
            "Changing a picture".to_owned()
        } else {
            let prompt: String = arg_str(args, "prompt").chars().take(60).collect();
            format!("Making a picture: {prompt}")
        }
    }
    fn effect(&self, _ctx: &ToolContext, _args: &Value) -> Effect {
        Effect::AlwaysAsk {
            summary: "Make a picture (costs money)".to_owned(),
            risk: Risk::Low,
            diff: None,
        }
    }
    fn run<'a>(&'a self, _ctx: &'a ToolContext, _args: Value) -> BoxFuture<'a, ToolOutput> {
        // Run by the agent, which holds the AI provider; reaching here means it wasn't.
        Box::pin(async { ToolOutput::error("making pictures isn't available here") })
    }
}

/// The `view_image` tool's name.
pub const VIEW_IMAGE: &str = "view_image";

/// Look at a picture file with the picture reader.
pub struct ViewImage;

impl Tool for ViewImage {
    fn name(&self) -> &'static str {
        VIEW_IMAGE
    }
    fn description(&self) -> &'static str {
        "Look at a picture file (PNG, JPEG, WebP, GIF) — a screenshot, a photo in the folder, a \
         picture from the clipboard or a website — and get a detailed description, with any text \
         in it copied out. Ask a question to focus on something."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The picture file" },
                "question": { "type": "string", "description": "What to look for or answer, if anything specific" }
            },
            "required": ["path"]
        })
    }
    fn title(&self, args: &Value) -> String {
        // Just the name: saved pictures live at long internal paths.
        let path = std::path::Path::new(arg_str(args, "path"));
        let name = path.file_name().map_or_else(
            || arg_str(args, "path").to_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        format!("Looking at {name}")
    }
    fn run<'a>(&'a self, _ctx: &'a ToolContext, _args: Value) -> BoxFuture<'a, ToolOutput> {
        // Run by the agent, which holds the picture reader.
        Box::pin(async { ToolOutput::error("looking at pictures isn't available here") })
    }
}

/// The `make_video` tool's name.
pub const MAKE_VIDEO: &str = "make_video";

/// Make a short video (Seedance 2.0 on fal.ai).
pub struct MakeVideo;

impl Tool for MakeVideo {
    fn name(&self) -> &'static str {
        MAKE_VIDEO
    }
    fn description(&self) -> &'static str {
        "Make a short video with Seedance 2.0 (fal.ai). From a description alone, or from \
         pictures: mode start_frame uses images[0] as the first frame (and images[1], if given, as \
         the last); mode references uses up to 9 pictures as references (mention them in the prompt \
         as @Image1, @Image2…). resolution 480p/720p/1080p/4k (default 720p), duration 4-15 s \
         (default 5), aspect_ratio auto/21:9/16:9/4:3/1:1/3:4/9:16 (default 16:9), audio (default \
         true), fast (cheaper, a little lower quality). Videos cost real money (about $1.50 for 5 s \
         at 720p): the user sees the exact price and approves it first. It takes a few minutes."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "The scene, action, camera and style" },
                "images": { "type": "array", "items": { "type": "string" }, "description": "Pictures (paths) for start_frame or references" },
                "mode": { "type": "string", "enum": ["text", "start_frame", "references"] },
                "resolution": { "type": "string", "enum": ["480p", "720p", "1080p", "4k"] },
                "duration": { "type": "integer", "minimum": 4, "maximum": 15 },
                "aspect_ratio": { "type": "string", "enum": ["auto", "21:9", "16:9", "4:3", "1:1", "3:4", "9:16"] },
                "audio": { "type": "boolean", "description": "Generate sound (default true)" },
                "fast": { "type": "boolean", "description": "The cheaper fast variant" },
                "save_as": { "type": "string", "description": "Also save it in the working folder (an .mp4 name)" }
            },
            "required": ["prompt"]
        })
    }
    fn title(&self, args: &Value) -> String {
        let prompt: String = arg_str(args, "prompt").chars().take(60).collect();
        format!("Making a video: {prompt}")
    }
    fn effect(&self, _ctx: &ToolContext, _args: &Value) -> Effect {
        Effect::AlwaysAsk {
            summary: "Make a video (costs money)".to_owned(),
            risk: Risk::Low,
            diff: None,
        }
    }
    fn run<'a>(&'a self, _ctx: &'a ToolContext, _args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async { ToolOutput::error("making videos isn't available here") })
    }
}
