//! `archive`: list, unpack and make `.zip` files with Thrall's own code (no outside program, the
//! same on every system). Unpacking keeps every file inside the folder it unpacks to: names that
//! climb out (`../`), absolute names and links are skipped, Unix permission bits aren't applied,
//! and the total size and number of files are capped.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use thrall_gates::Effect;

use crate::constants::{
    ARCHIVE_LIST_ENTRIES, ARCHIVE_MAX_ADDED, ARCHIVE_MAX_FILES, ARCHIVE_MAX_UNPACKED_BYTES,
};
use crate::diff;
use crate::paths::{display, inside, resolve};
use crate::tool::{Cancel, Tool, ToolContext, ToolOutput, arg_str, parse_args};

/// The `archive` tool.
pub struct Archive;

/// The arguments (`action` is read on its own, by [`action`]).
#[derive(Deserialize)]
struct ArchiveArgs {
    path: String,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    List,
    Unpack,
    Zip,
}

fn action(args: &Value) -> Option<Action> {
    match arg_str(args, "action").trim().to_lowercase().as_str() {
        "list" => Some(Action::List),
        "unpack" | "unzip" | "extract" => Some(Action::Unpack),
        "zip" | "create" | "pack" => Some(Action::Zip),
        _ => None,
    }
}

/// One entry in a zip, as listed.
struct Entry {
    name: String,
    size: u64,
    dir: bool,
    link: bool,
}

/// `bytes` for people: "1.4 MB".
fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

fn open(zip: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let file = std::fs::File::open(zip).map_err(|e| format!("couldn't open it: {e}"))?;
    zip::ZipArchive::new(file).map_err(|e| format!("it isn't a zip file Thrall can read: {e}"))
}

/// What's in a zip (read from its index; nothing is unpacked).
fn entries(zip: &Path) -> Result<Vec<Entry>, String> {
    let mut archive = open(zip)?;
    let mut out = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|e| format!("couldn't read entry {}: {e}", i + 1))?;
        out.push(Entry {
            name: entry
                .name()
                .map(|n| n.into_owned())
                .unwrap_or_else(|_| format!("(entry {} with an unreadable name)", i + 1)),
            size: entry.size(),
            dir: entry.is_dir(),
            link: entry.is_symlink(),
        });
    }
    Ok(out)
}

/// Lines for `entries` (capped), then the totals.
fn listing(entries: &[Entry]) -> String {
    let mut out = String::new();
    for e in entries.iter().take(ARCHIVE_LIST_ENTRIES) {
        let line = if e.dir {
            e.name.clone()
        } else if e.link {
            format!("{}  (a link: not unpacked)", e.name)
        } else {
            format!("{}  ({})", e.name, human(e.size))
        };
        out.push_str(&line);
        out.push('\n');
    }
    if entries.len() > ARCHIVE_LIST_ENTRIES {
        out.push_str(&format!(
            "… and {} more\n",
            entries.len() - ARCHIVE_LIST_ENTRIES
        ));
    }
    let files = entries.iter().filter(|e| !e.dir && !e.link).count();
    let total: u64 = entries.iter().filter(|e| !e.dir).map(|e| e.size).sum();
    out.push_str(&format!("{files} files, {} unpacked", human(total)));
    out
}

/// Where an entry called `name` goes inside the unpack folder, if it's safe: relative, with no
/// `..`, no drive or root, and nothing Windows would read as a hidden stream (`:`).
fn safe_relative(name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    if name.contains(':') || name.chars().any(char::is_control) {
        return None;
    }
    let mut out = PathBuf::new();
    for part in Path::new(&name).components() {
        match part {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// The folder a zip unpacks to: `to`, or a folder named after the zip, next to it.
fn unpack_folder(ctx: &ToolContext, a: &ArchiveArgs) -> PathBuf {
    match a.to.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(to) => resolve(&ctx.workspace, to),
        None => resolve(&ctx.workspace, &a.path).with_extension(""),
    }
}

/// Files already in `folder` that unpacking `entries` would replace.
fn conflicts(folder: &Path, entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .filter(|e| !e.dir && !e.link)
        .filter_map(|e| safe_relative(&e.name).map(|rel| (e, folder.join(rel))))
        .filter(|(_, target)| target.is_file())
        .map(|(e, _)| e.name.clone())
        .collect()
}

/// What unpacking did.
#[derive(Debug, Default)]
struct Unpacked {
    files: usize,
    bytes: u64,
    skipped: Vec<String>,
}

/// How much one unpack may write.
#[derive(Debug, Clone, Copy)]
struct Limits {
    bytes: u64,
    files: usize,
}

const LIMITS: Limits = Limits {
    bytes: ARCHIVE_MAX_UNPACKED_BYTES,
    files: ARCHIVE_MAX_FILES,
};

/// Unpack `zip` into `folder`, keeping every file inside it.
fn unpack(
    zip: &Path,
    folder: &Path,
    overwrite: bool,
    cancel: &Cancel,
    limits: Limits,
) -> Result<Unpacked, String> {
    let mut archive = open(zip)?;
    std::fs::create_dir_all(folder).map_err(|e| format!("couldn't make the folder: {e}"))?;
    let mut done = Unpacked::default();
    for i in 0..archive.len() {
        if cancel.is_cancelled() {
            return Err(format!(
                "stopped after {} files (the rest weren't unpacked)",
                done.files
            ));
        }
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            // Password-protected or an unsupported kind of compression.
            Err(e) => {
                done.skipped.push(format!("entry {}: {e}", i + 1));
                continue;
            }
        };
        let name = entry
            .name()
            .map(|n| n.into_owned())
            .unwrap_or_else(|_| format!("entry {}", i + 1));
        if entry.is_symlink() {
            done.skipped.push(format!("{name} (a link)"));
            continue;
        }
        let Some(rel) = safe_relative(&name) else {
            done.skipped
                .push(format!("{name} (its name would put it outside the folder)"));
            continue;
        };
        let target = folder.join(&rel);
        // A link already in the folder mustn't lead the file somewhere else.
        if !inside(folder, &target) {
            done.skipped
                .push(format!("{name} (it would land outside the folder)"));
            continue;
        }
        if entry.is_dir() {
            std::fs::create_dir_all(&target)
                .map_err(|e| format!("couldn't make the folder {name}: {e}"))?;
            continue;
        }
        if target.exists() && !overwrite {
            done.skipped.push(format!("{name} (already there)"));
            continue;
        }
        if done.files >= limits.files {
            return Err(format!(
                "it has more than {} files; stopped after {} (the rest weren't unpacked)",
                limits.files, done.files
            ));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("couldn't make the folder for {name}: {e}"))?;
            if !inside(folder, parent) {
                done.skipped
                    .push(format!("{name} (it would land outside the folder)"));
                continue;
            }
        }
        let room = limits.bytes - done.bytes;
        let mut out =
            std::fs::File::create(&target).map_err(|e| format!("couldn't write {name}: {e}"))?;
        // Counted as it's written: the sizes a zip claims can be false.
        let written = std::io::copy(&mut (&mut entry).take(room + 1), &mut out)
            .map_err(|e| format!("couldn't unpack {name}: {e}"))?;
        if written > room {
            drop(out);
            let _ = std::fs::remove_file(&target);
            return Err(format!(
                "it unpacks to more than {}, so Thrall stopped after {} files (it may be a zip \
                 bomb: a small file built to fill the disk)",
                human(limits.bytes),
                done.files
            ));
        }
        done.files += 1;
        done.bytes += written;
    }
    Ok(done)
}

/// What goes into a new zip: (name in the zip, file on disk; `None` for a folder).
fn gather(sources: &[PathBuf], skip: &Path) -> Result<Vec<(String, Option<PathBuf>)>, String> {
    fn walk(
        dir: &Path,
        prefix: &str,
        skip: &Path,
        out: &mut Vec<(String, Option<PathBuf>)>,
    ) -> Result<(), String> {
        out.push((format!("{prefix}/"), None));
        let mut items: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| format!("couldn't read {}: {e}", dir.display()))?
            .filter_map(Result::ok)
            .collect();
        items.sort_by_key(std::fs::DirEntry::file_name);
        for item in items {
            if out.len() > ARCHIVE_MAX_ADDED {
                return Err(format!(
                    "that's more than {ARCHIVE_MAX_ADDED} files and folders"
                ));
            }
            let path = item.path();
            // Links aren't followed (they could lead anywhere, or round in circles).
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() || path == skip {
                continue;
            }
            let name = format!("{prefix}/{}", item.file_name().to_string_lossy());
            if meta.is_dir() {
                walk(&path, &name, skip, out)?;
            } else {
                out.push((name, Some(path)));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for source in sources {
        let meta = std::fs::symlink_metadata(source)
            .map_err(|_| format!("{} doesn't exist", source.display()))?;
        let name = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{} can't go in a zip", source.display()))?;
        if meta.is_dir() {
            walk(source, &name, skip, &mut out)?;
        } else if !meta.file_type().is_symlink() {
            out.push((name, Some(source.clone())));
        }
    }
    Ok(out)
}

/// Write `items` into a new zip at `zip` (through a temporary file, so a failure leaves nothing
/// half-made). Returns how many files went in and their size.
fn make_zip(
    zip: &Path,
    items: &[(String, Option<PathBuf>)],
    cancel: &Cancel,
) -> Result<(usize, u64), String> {
    let temp = zip.with_extension("zip.thrall-tmp");
    let result = (|| {
        let out = std::fs::File::create(&temp).map_err(|e| format!("couldn't write: {e}"))?;
        let mut writer = zip::ZipWriter::new(std::io::BufWriter::new(out));
        let (mut files, mut bytes) = (0usize, 0u64);
        for (name, source) in items {
            if cancel.is_cancelled() {
                return Err("stopped before the zip was finished".to_owned());
            }
            let Some(source) = source else {
                writer
                    .add_directory(name.as_str(), zip::write::SimpleFileOptions::default())
                    .map_err(|e| format!("couldn't add {name}: {e}"))?;
                continue;
            };
            let size = std::fs::metadata(source).map(|m| m.len()).unwrap_or(0);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .large_file(size > u64::from(u32::MAX));
            writer
                .start_file(name.as_str(), options)
                .map_err(|e| format!("couldn't add {name}: {e}"))?;
            let mut input =
                std::fs::File::open(source).map_err(|e| format!("couldn't read {name}: {e}"))?;
            bytes += std::io::copy(&mut input, &mut writer)
                .map_err(|e| format!("couldn't add {name}: {e}"))?;
            files += 1;
        }
        let mut out = writer
            .finish()
            .map_err(|e| format!("couldn't finish the zip: {e}"))?;
        out.flush().map_err(|e| format!("couldn't write: {e}"))?;
        Ok((files, bytes))
    })();
    match result {
        Ok(done) => {
            std::fs::rename(&temp, zip).map_err(|e| {
                let _ = std::fs::remove_file(&temp);
                format!("couldn't save it: {e}")
            })?;
            Ok(done)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

impl Archive {
    /// Why this call can't work, if so.
    fn problem(ctx: &ToolContext, args: &Value) -> Option<String> {
        let Some(action) = action(args) else {
            return Some("action must be list, unpack or zip".to_owned());
        };
        let a: ArchiveArgs = match parse_args("archive", args.clone()) {
            Ok(a) => a,
            Err(e) => return Some(e),
        };
        let path = resolve(&ctx.workspace, &a.path);
        let shown = display(&ctx.workspace, &path);
        match action {
            Action::List | Action::Unpack => {
                if !path.is_file() {
                    return Some(format!("{shown} doesn't exist"));
                }
                let found = match entries(&path) {
                    Ok(f) => f,
                    Err(e) => return Some(format!("{shown}: {e}")),
                };
                if action == Action::List {
                    return None;
                }
                let folder = unpack_folder(ctx, &a);
                let folder_shown = display(&ctx.workspace, &folder);
                if folder.is_file() {
                    return Some(format!("{folder_shown} is a file, not a folder"));
                }
                let clash = conflicts(&folder, &found);
                if !a.overwrite && !clash.is_empty() {
                    let mut names: Vec<&str> = clash.iter().map(String::as_str).take(10).collect();
                    if clash.len() > 10 {
                        names.push("…");
                    }
                    return Some(format!(
                        "{} files are already in {folder_shown} ({}). Set overwrite to replace \
                         them, or unpack somewhere else with `to`.",
                        clash.len(),
                        names.join(", ")
                    ));
                }
                None
            }
            Action::Zip => {
                if path
                    .extension()
                    .is_none_or(|e| !e.eq_ignore_ascii_case("zip"))
                {
                    return Some(format!("{shown} must end in .zip"));
                }
                if path.exists() && !a.overwrite {
                    return Some(format!(
                        "{shown} already exists. Set overwrite to replace it, or pick another name."
                    ));
                }
                if a.files.is_empty() {
                    return Some("say which files or folders go in (files)".to_owned());
                }
                a.files
                    .iter()
                    .map(|f| resolve(&ctx.workspace, f))
                    .find(|f| std::fs::symlink_metadata(f).is_err())
                    .map(|missing| format!("{} doesn't exist", display(&ctx.workspace, &missing)))
            }
        }
    }
}

impl Tool for Archive {
    fn name(&self) -> &'static str {
        "archive"
    }
    fn description(&self) -> &'static str {
        "Zip files: `list` what's in a .zip, `unpack` it (into `to`, or a folder named after it \
         next to it), or `zip` files and folders (`files`) into a new .zip at `path`. Use this \
         rather than a shell command: unpacking keeps everything inside the folder and skips \
         anything unsafe. Existing files are only replaced with overwrite."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "unpack", "zip"] },
                "path": { "type": "string", "description": "The .zip to list or unpack, or the new .zip to make" },
                "to": { "type": "string", "description": "unpack: the folder to unpack into (default: a folder named after the zip, next to it)" },
                "files": { "type": "array", "items": { "type": "string" }, "description": "zip: files and folders to put in it" },
                "overwrite": { "type": "boolean", "description": "Replace files (unpack) or the zip (zip) that already exist" }
            },
            "required": ["action", "path"]
        })
    }
    fn title(&self, args: &Value) -> String {
        let path = arg_str(args, "path");
        match action(args) {
            Some(Action::List) => format!("Looking inside {path}"),
            Some(Action::Unpack) => format!("Unpacking {path}"),
            Some(Action::Zip) => format!("Zipping into {path}"),
            None => format!("Zip file {path}"),
        }
    }
    fn precheck(&self, ctx: &ToolContext, args: &Value) -> Option<String> {
        Self::problem(ctx, args)
    }
    fn effect(&self, ctx: &ToolContext, args: &Value) -> Effect {
        let Ok(a) = parse_args::<ArchiveArgs>("archive", args.clone()) else {
            return Effect::Read;
        };
        let path = resolve(&ctx.workspace, &a.path);
        match action(args) {
            Some(Action::Unpack) => {
                let folder = unpack_folder(ctx, &a);
                let shown = format!("{}/", display(&ctx.workspace, &folder));
                Effect::Edit {
                    diff: entries(&path)
                        .ok()
                        .map(|found| diff::make(&shown, None, &format!("{}\n", listing(&found)))),
                    path: shown,
                }
            }
            Some(Action::Zip) => {
                let shown = display(&ctx.workspace, &path);
                let sources: Vec<PathBuf> =
                    a.files.iter().map(|f| resolve(&ctx.workspace, f)).collect();
                let preview = gather(&sources, &path).ok().map(|items| {
                    let mut text: String = items
                        .iter()
                        .take(ARCHIVE_LIST_ENTRIES)
                        .map(|(name, _)| format!("{name}\n"))
                        .collect();
                    if items.len() > ARCHIVE_LIST_ENTRIES {
                        text.push_str(&format!(
                            "… and {} more\n",
                            items.len() - ARCHIVE_LIST_ENTRIES
                        ));
                    }
                    diff::make(&shown, None, &text)
                });
                Effect::Edit {
                    diff: preview,
                    path: shown,
                }
            }
            _ => Effect::Read,
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            if let Some(problem) = Self::problem(ctx, &args) {
                return ToolOutput::error(problem);
            }
            let Some(action) = action(&args) else {
                return ToolOutput::error("action must be list, unpack or zip");
            };
            let a: ArchiveArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let path = resolve(&ctx.workspace, &a.path);
            let shown = display(&ctx.workspace, &path);
            let cancel = ctx.cancel.clone();
            match action {
                Action::List => match entries(&path) {
                    Ok(found) => {
                        let text = listing(&found);
                        let summary = text.lines().last().unwrap_or_default().to_owned();
                        ToolOutput::ok(format!("{shown}:\n{text}"), summary)
                    }
                    Err(e) => ToolOutput::error(format!("{shown}: {e}")),
                },
                Action::Unpack => {
                    let folder = unpack_folder(ctx, &a);
                    let folder_shown = display(&ctx.workspace, &folder);
                    let (zip, to, overwrite) = (path.clone(), folder.clone(), a.overwrite);
                    let done = tokio::task::spawn_blocking(move || {
                        unpack(&zip, &to, overwrite, &cancel, LIMITS)
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    match done {
                        Ok(done) => {
                            let mut text = format!(
                                "Unpacked {} files ({}) from {shown} into {folder_shown}/.",
                                done.files,
                                human(done.bytes)
                            );
                            if !done.skipped.is_empty() {
                                text.push_str(&format!(
                                    "\nSkipped {}:\n- {}",
                                    done.skipped.len(),
                                    done.skipped
                                        .iter()
                                        .take(ARCHIVE_LIST_ENTRIES)
                                        .cloned()
                                        .collect::<Vec<_>>()
                                        .join("\n- ")
                                ));
                            }
                            let summary = format!(
                                "{} files into {folder_shown}/{}",
                                done.files,
                                if done.skipped.is_empty() {
                                    String::new()
                                } else {
                                    format!(", {} skipped", done.skipped.len())
                                }
                            );
                            ToolOutput::ok(text, summary)
                        }
                        Err(e) => ToolOutput::error(format!("unpacking {shown}: {e}")),
                    }
                }
                Action::Zip => {
                    let sources: Vec<PathBuf> =
                        a.files.iter().map(|f| resolve(&ctx.workspace, f)).collect();
                    let out = path.clone();
                    let done = tokio::task::spawn_blocking(move || {
                        let items = gather(&sources, &out)?;
                        make_zip(&out, &items, &cancel)
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    match done {
                        Ok((files, bytes)) => {
                            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                            ToolOutput::ok(
                                format!(
                                    "Made {shown}: {files} files ({} before, {} zipped).",
                                    human(bytes),
                                    human(size)
                                ),
                                format!("{files} files, {}", human(size)),
                            )
                        }
                        Err(e) => ToolOutput::error(format!("making {shown}: {e}")),
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_names_that_stay_inside_are_used() {
        assert_eq!(
            safe_relative("a/b.txt"),
            Some(PathBuf::from("a").join("b.txt"))
        );
        assert_eq!(safe_relative("./a.txt"), Some(PathBuf::from("a.txt")));
        assert_eq!(
            safe_relative("a\\b.txt"),
            Some(PathBuf::from("a").join("b.txt"))
        );
        for bad in [
            "../evil.txt",
            "a/../../evil.txt",
            "/etc/passwd",
            "\\windows\\evil.dll",
            "C:/evil.txt",
            "C:evil.txt",
            "notes.txt:hidden",
            "",
            "./",
        ] {
            assert_eq!(safe_relative(bad), None, "{bad}");
        }
    }

    fn zip_of(path: &Path, files: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, body) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("thrall-archive-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn unpacking_stops_at_the_size_and_file_limits() {
        let dir = scratch("limits");
        let zip = dir.join("big.zip");
        let big = vec![b'x'; 10_000];
        zip_of(&zip, &[("a.bin", &big), ("b.bin", &big), ("c.bin", &big)]);
        let small = Limits {
            bytes: 15_000,
            files: 100,
        };
        let e = unpack(&zip, &dir.join("out1"), false, &Cancel::never(), small).unwrap_err();
        assert!(e.contains("zip bomb") && e.contains("after 1 files"), "{e}");
        assert!(
            !dir.join("out1/b.bin").exists(),
            "the file that went over is removed"
        );
        let few = Limits {
            bytes: u64::MAX / 2,
            files: 2,
        };
        let e = unpack(&zip, &dir.join("out2"), false, &Cancel::never(), few).unwrap_err();
        assert!(e.contains("more than 2 files"), "{e}");
        let done = unpack(&zip, &dir.join("out3"), false, &Cancel::never(), LIMITS).unwrap();
        assert_eq!((done.files, done.bytes), (3, 30_000));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizes_read_well() {
        assert_eq!(human(512), "512 bytes");
        assert_eq!(human(1536), "1.5 KB");
        assert_eq!(human(3 * 1024 * 1024), "3.0 MB");
    }
}
