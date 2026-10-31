//! The `archive` tool against real zip files in a temporary folder, including hostile ones.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;
use thrall_store::SqliteStore;
use thrall_tools::tools::Archive;
use thrall_tools::{Cancel, Effect, Tool, ToolContext};
use tokio::sync::mpsc;

async fn ctx(name: &str) -> ToolContext {
    let workspace: PathBuf =
        std::env::temp_dir().join(format!("thrall-zip-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&workspace);
    std::fs::create_dir_all(&workspace).unwrap();
    let (tx, _rx) = mpsc::unbounded_channel();
    ToolContext {
        workspace,
        store: Arc::new(SqliteStore::open_in_memory().await.unwrap()),
        user_id: "u".into(),
        session_id: "s".into(),
        http: reqwest::Client::new(),
        output: tx,
        reads: Default::default(),
        cancel: Cancel::never(),
        data_dir: None,
        mcp: None,
        memory: None,
        browser: Default::default(),
        disabled_skills: Vec::new(),
    }
}

/// A zip with `files`, and a link called `link` pointing at `target` when given.
fn zip_of(path: &Path, files: &[(&str, &str)], link: Option<(&str, &str)>) {
    let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (name, body) in files {
        w.start_file(*name, options).unwrap();
        w.write_all(body.as_bytes()).unwrap();
    }
    if let Some((name, target)) = link {
        w.add_symlink(name, target, options).unwrap();
    }
    w.finish().unwrap();
}

#[tokio::test]
async fn a_hostile_zip_cant_write_outside_the_folder() {
    let ctx = ctx("hostile").await;
    let zip = ctx.workspace.join("in/evil.zip");
    std::fs::create_dir_all(zip.parent().unwrap()).unwrap();
    zip_of(
        &zip,
        &[
            ("../../escaped.txt", "nope"),
            ("/absolute.txt", "nope"),
            ("notes.txt:hidden", "nope"),
            ("good/ok.txt", "fine"),
        ],
        Some(("link.txt", "../../../somewhere")),
    );
    let out = Archive
        .run(&ctx, json!({"action": "unpack", "path": "in/evil.zip"}))
        .await;
    assert!(out.ok, "{}", out.content);
    assert!(out.content.contains("Unpacked 1 files"), "{}", out.content);
    assert!(out.content.contains("Skipped 4"), "{}", out.content);
    assert_eq!(
        std::fs::read_to_string(ctx.workspace.join("in/evil/good/ok.txt")).unwrap(),
        "fine"
    );
    assert!(!ctx.workspace.join("escaped.txt").exists());
    assert!(!ctx.workspace.parent().unwrap().join("escaped.txt").exists());
    assert!(
        !ctx.workspace.join("in/evil/link.txt").exists(),
        "links aren't made"
    );
    let _ = std::fs::remove_dir_all(&ctx.workspace);
}

#[tokio::test]
async fn zip_list_and_unpack_round_trip() {
    let ctx = ctx("round").await;
    let ws = &ctx.workspace;
    std::fs::create_dir_all(ws.join("project/src")).unwrap();
    std::fs::write(ws.join("project/readme.md"), "# Hi").unwrap();
    std::fs::write(ws.join("project/src/main.rs"), "fn main() {}").unwrap();
    std::fs::write(ws.join("notes.txt"), "remember").unwrap();

    let args =
        json!({"action": "zip", "path": "out/bundle.zip", "files": ["project", "notes.txt"]});
    assert_eq!(Archive.precheck(&ctx, &args), None);
    match Archive.effect(&ctx, &args) {
        Effect::Edit { path, diff } => {
            assert_eq!(path, "out/bundle.zip");
            let diff = diff.unwrap().unified;
            assert!(diff.contains("project/src/main.rs"), "{diff}");
        }
        other => panic!("{other:?}"),
    }
    std::fs::create_dir_all(ws.join("out")).unwrap();
    let made = Archive.run(&ctx, args.clone()).await;
    assert!(made.ok, "{}", made.content);
    assert!(made.content.contains("3 files"), "{}", made.content);
    assert!(
        Archive
            .precheck(&ctx, &args)
            .unwrap()
            .contains("already exists"),
        "no overwrite unless asked"
    );

    let listed = Archive
        .run(&ctx, json!({"action": "list", "path": "out/bundle.zip"}))
        .await;
    assert!(
        listed.content.contains("project/readme.md"),
        "{}",
        listed.content
    );
    assert_eq!(listed.summary, "3 files, 24 bytes unpacked");
    assert!(matches!(
        Archive.effect(&ctx, &json!({"action": "list", "path": "out/bundle.zip"})),
        Effect::Read
    ));

    let unpack = json!({"action": "unpack", "path": "out/bundle.zip", "to": "copy"});
    let done = Archive.run(&ctx, unpack.clone()).await;
    assert!(done.ok, "{}", done.content);
    assert_eq!(
        std::fs::read_to_string(ws.join("copy/project/src/main.rs")).unwrap(),
        "fn main() {}"
    );
    // Unpacking again would replace files: refused unless overwrite is set.
    let again = Archive.precheck(&ctx, &unpack).unwrap();
    assert!(again.contains("3 files are already in copy"), "{again}");
    std::fs::write(ws.join("copy/notes.txt"), "changed").unwrap();
    let replaced = Archive
        .run(
            &ctx,
            json!({"action": "unpack", "path": "out/bundle.zip", "to": "copy", "overwrite": true}),
        )
        .await;
    assert!(replaced.ok, "{}", replaced.content);
    assert_eq!(
        std::fs::read_to_string(ws.join("copy/notes.txt")).unwrap(),
        "remember"
    );
    let _ = std::fs::remove_dir_all(ws);
}

#[tokio::test]
async fn it_only_writes_inside_the_session_folder() {
    let ctx = ctx("inside").await;
    std::fs::write(ctx.workspace.join("a.txt"), "a").unwrap();
    let outside = std::env::temp_dir().join("thrall-zip-outside.zip");
    let e = Archive
        .precheck(
            &ctx,
            &json!({"action": "zip", "path": outside.to_string_lossy(), "files": ["a.txt"]}),
        )
        .unwrap();
    assert!(e.contains("outside this session's folder"), "{e}");
    zip_of(&ctx.workspace.join("x.zip"), &[("a.txt", "a")], None);
    let e = Archive
        .precheck(
            &ctx,
            &json!({"action": "unpack", "path": "x.zip", "to": "../elsewhere"}),
        )
        .unwrap();
    assert!(e.contains("outside this session's folder"), "{e}");
    let e = Archive
        .precheck(
            &ctx,
            &json!({"action": "zip", "path": "x.tar", "files": ["a.txt"]}),
        )
        .unwrap();
    assert!(e.contains(".zip"), "{e}");
    let e = Archive
        .precheck(&ctx, &json!({"action": "explode", "path": "x.zip"}))
        .unwrap();
    assert!(e.contains("list, unpack or zip"), "{e}");
    let _ = std::fs::remove_dir_all(&ctx.workspace);
}
