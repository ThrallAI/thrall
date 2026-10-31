//! `POST /v1/files`: a file someone attaches to a message. The body is the file; the name comes in
//! `X-File-Name` (URL-encoded). It's saved under the data folder's `uploads/<id>/` and the id is
//! returned, for `turn.send {files}`. Same rules as the socket: the access token, and a browser
//! request must come from Thrall's own pages.

use std::path::PathBuf;

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use thrall_core::constants::{UPLOADS_DIR, id_prefix};
use thrall_core::ids::new_id;

use crate::constants::{FALLBACK_FILE_NAME, FILE_NAME_HEADER, FILE_NAME_MAX_CHARS};

/// Decode `%XX` escapes (what a browser's `encodeURIComponent` produces).
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A safe file name: letters, digits, `.`, `-`, `_` and spaces only, no leading dots, capped.
pub fn clean_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .take(FILE_NAME_MAX_CHARS)
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim().to_owned();
    if cleaned.is_empty() {
        FALLBACK_FILE_NAME.to_owned()
    } else {
        cleaned
    }
}

/// A refusal, as JSON `{error}`.
pub fn refuse(status: StatusCode, message: &str) -> Response {
    (status, axum::Json(json!({ "error": message }))).into_response()
}

/// Save an upload (the caller checked who sent it); `uploads` is the person's uploads folder
/// (None when the engine has none).
pub fn save(headers: &HeaderMap, body: &Bytes, uploads: Option<PathBuf>) -> Response {
    let Some(uploads) = uploads else {
        return refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "This engine has no data folder to keep files in.",
        );
    };
    let name = clean_name(&percent_decode(
        headers
            .get(FILE_NAME_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default(),
    ));
    let id = new_id(id_prefix::FILE);
    let folder = uploads.join(&id);
    let saved =
        std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(folder.join(&name), body));
    if let Err(e) = saved {
        tracing::warn!(%e, "couldn't save an upload");
        return refuse(StatusCode::INTERNAL_SERVER_ERROR, "Couldn't save the file.");
    }
    (
        StatusCode::CREATED,
        axum::Json(json!({ "file": id, "name": name, "size": body.len() })),
    )
        .into_response()
}

/// The content type a file is served with. Only pictures, PDFs and recordings show in the page; everything
/// else downloads, so an uploaded page or script never runs as part of Thrall.
fn content_type(path: &std::path::Path) -> (&'static str, bool) {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => ("image/png", true),
        "jpg" | "jpeg" => ("image/jpeg", true),
        "webp" => ("image/webp", true),
        "gif" => ("image/gif", true),
        "pdf" => ("application/pdf", true),
        // Recordings and videos play in the page.
        "mp4" | "m4v" => ("video/mp4", true),
        "mov" => ("video/quicktime", true),
        "wav" => ("audio/wav", true),
        "mp3" => ("audio/mpeg", true),
        "ogg" | "oga" | "opus" => ("audio/ogg", true),
        "webm" | "weba" => ("audio/webm", true),
        "m4a" => ("audio/mp4", true),
        "flac" => ("audio/flac", true),
        _ => ("application/octet-stream", false),
    }
}

/// Send an uploaded file back (the caller checked who asked; `path` is None when there's no
/// such upload, or it isn't theirs).
pub async fn send(path: Option<PathBuf>) -> Response {
    let Some(path) = path else {
        return refuse(StatusCode::NOT_FOUND, "There's no such file.");
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(_) => return refuse(StatusCode::NOT_FOUND, "There's no such file."),
    };
    let (mime, inline) = content_type(&path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let disposition = format!(
        "{}; filename=\"{}\"",
        if inline { "inline" } else { "attachment" },
        clean_name(&name)
    );
    (
        [
            (header::CONTENT_TYPE, mime.to_owned()),
            (header::CONTENT_DISPOSITION, disposition),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
            (header::CACHE_CONTROL, "private, max-age=86400".to_owned()),
        ],
        bytes,
    )
        .into_response()
}

/// The uploads folder inside a data folder.
pub fn folder(data_dir: Option<&std::path::Path>) -> Option<PathBuf> {
    data_dir.map(|d| d.join(UPLOADS_DIR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("notes.txt"), "notes.txt");
        assert_eq!(clean_name("..\\..\\Windows\\win.ini"), "win.ini");
        assert_eq!(clean_name("../../etc/passwd"), "passwd");
        assert_eq!(clean_name("..."), FALLBACK_FILE_NAME);
        assert_eq!(clean_name("a<b>:c?.txt"), "a_b__c_.txt");
        assert_eq!(clean_name(""), FALLBACK_FILE_NAME);
    }

    #[test]
    fn percent_escapes_decode() {
        assert_eq!(percent_decode("my%20notes%2Bv2.txt"), "my notes+v2.txt");
        assert_eq!(percent_decode("caf%C3%A9.md"), "café.md");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
