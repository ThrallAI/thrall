//! Files a tool makes for the user to see (a clipboard picture, a screenshot, audio): saved like
//! uploads (`uploads/<id>/<name>` in the data folder) so every client can show them.

use std::path::PathBuf;

use thrall_core::constants::{UPLOADS_DIR, id_prefix};
use thrall_core::ids::new_id;
use thrall_protocol::FileRef;

use crate::tool::ToolContext;

/// How clients show a file, from its name.
pub fn kind(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or_default().to_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" => "image",
        "mp3" | "wav" | "ogg" | "m4a" | "webm" | "flac" => "audio",
        "mp4" | "mov" | "m4v" => "video",
        "pdf" | "docx" | "pptx" | "xlsx" | "xls" | "ods" => "document",
        "txt" | "md" | "csv" | "json" => "text",
        _ => "other",
    }
}

/// Save `bytes` as `name` for the user to see; where it is and how to refer to it.
pub fn save(ctx: &ToolContext, name: &str, bytes: &[u8]) -> Result<(PathBuf, FileRef), String> {
    let uploads = ctx
        .data_dir
        .as_ref()
        .map(|d| d.join(UPLOADS_DIR))
        .ok_or_else(|| "there's no data folder to keep files in".to_owned())?;
    let id = new_id(id_prefix::FILE);
    let folder = uploads.join(&id);
    let path = folder.join(name);
    std::fs::create_dir_all(&folder)
        .and_then(|()| std::fs::write(&path, bytes))
        .map_err(|e| format!("couldn't save {name}: {e}"))?;
    let file = FileRef {
        file: id,
        name: name.to_owned(),
        kind: kind(name).to_owned(),
        size: bytes.len() as u64,
    };
    Ok((path, file))
}

/// RGBA pixels as a PNG.
pub fn png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// A PNG's RGBA pixels (width, height, bytes), for putting a picture on the clipboard.
pub fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| "the picture is too big".to_owned())?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::GrayscaleAlpha => buf
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        other => return Err(format!("unsupported PNG colour type {other:?}")),
    };
    Ok((info.width, info.height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trips() {
        let pixels = [255, 0, 0, 255, 0, 255, 0, 128];
        let encoded = png(2, 1, &pixels).unwrap();
        let (w, h, back) = decode_png(&encoded).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(back, pixels);
    }

    #[test]
    fn kinds_come_from_names() {
        assert_eq!(kind("shot.PNG"), "image");
        assert_eq!(kind("hello.mp3"), "audio");
        assert_eq!(kind("notes.txt"), "text");
        assert_eq!(kind("thing"), "other");
    }
}
