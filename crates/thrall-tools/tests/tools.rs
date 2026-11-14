//! The core tools against a real temporary folder.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use thrall_protocol::{DiffKind, Risk};
use thrall_store::{NewSession, SqliteStore, Store};
use thrall_tools::tools::{
    EditFile, Glob, Grep, ListDir, McpCall, ReadFile, Shell, TodoWrite, WriteFile,
};
use thrall_tools::{Cancel, Effect, Tool, ToolContext};
use tokio::sync::mpsc;

async fn ctx(name: &str) -> (ToolContext, mpsc::UnboundedReceiver<String>) {
    let workspace: PathBuf =
        std::env::temp_dir().join(format!("thrall-tools-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&workspace);
    std::fs::create_dir_all(&workspace).unwrap();
    let store = SqliteStore::open_in_memory().await.unwrap();
    store.ensure_user("u", "U", 1).await.unwrap();
    store
        .create_session(NewSession {
            id: "s".into(),
            user_id: "u".into(),
            workspace: None,
            at: 1,
        })
        .await
        .unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    (
        ToolContext {
            workspace,
            store: Arc::new(store),
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
            pc: None,
        },
        rx,
    )
}

#[tokio::test]
async fn write_edit_read_round_trip() {
    let (ctx, _rx) = ctx("files").await;
    let w = WriteFile
        .run(
            &ctx,
            json!({"path": "notes/a.txt", "content": "one\ntwo\n"}),
        )
        .await;
    assert!(w.ok, "{}", w.content);
    assert_eq!(w.diff.as_ref().unwrap().kind, DiffKind::Create);

    let e = EditFile
        .run(
            &ctx,
            json!({"path": "notes/a.txt", "old_string": "two", "new_string": "TWO"}),
        )
        .await;
    assert!(e.ok, "{}", e.content);
    assert_eq!(e.diff.as_ref().unwrap().removed, 1);

    let r = ReadFile.run(&ctx, json!({"path": "notes/a.txt"})).await;
    assert!(r.content.contains("     2\tTWO"), "{}", r.content);

    let missing = EditFile
        .run(
            &ctx,
            json!({"path": "notes/a.txt", "old_string": "nope", "new_string": "x"}),
        )
        .await;
    assert!(!missing.ok);

    let listed = ListDir.run(&ctx, json!({})).await;
    assert!(listed.content.contains("notes/"));
}

#[tokio::test]
async fn files_outside_the_folder_can_be_changed_once_read() {
    let (ctx, _rx) = ctx("outside").await;
    let elsewhere = ctx.workspace.with_extension("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let file = elsewhere.join("notes.txt");
    let _ = std::fs::remove_file(&file);

    // A new file outside the folder is written.
    let path = file.to_string_lossy().into_owned();
    let out = WriteFile
        .run(&ctx, json!({"path": path, "content": "one"}))
        .await;
    assert!(out.ok, "{}", out.content);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "one");

    // One that's there but unread isn't changed until it's read.
    std::fs::write(elsewhere.join("old.txt"), "old").unwrap();
    let old = elsewhere.join("old.txt").to_string_lossy().into_owned();
    let out = WriteFile
        .run(&ctx, json!({"path": old, "content": "new"}))
        .await;
    assert!(!out.ok);
    assert!(out.content.contains("read"), "{}", out.content);
    ReadFile.run(&ctx, json!({"path": old})).await;
    let out = EditFile
        .run(
            &ctx,
            json!({"path": old, "old_string": "old", "new_string": "new"}),
        )
        .await;
    assert!(out.ok, "{}", out.content);
    assert_eq!(
        std::fs::read_to_string(elsewhere.join("old.txt")).unwrap(),
        "new"
    );
    let _ = std::fs::remove_dir_all(elsewhere);
}

#[tokio::test]
async fn ambiguous_edits_need_replace_all() {
    let (ctx, _rx) = ctx("ambiguous").await;
    WriteFile
        .run(&ctx, json!({"path": "b.txt", "content": "x x x"}))
        .await;
    let out = EditFile
        .run(
            &ctx,
            json!({"path": "b.txt", "old_string": "x", "new_string": "y"}),
        )
        .await;
    assert!(!out.ok);
    let all = EditFile
        .run(
            &ctx,
            json!({"path": "b.txt", "old_string": "x", "new_string": "y", "replace_all": true}),
        )
        .await;
    assert!(all.ok);
    assert_eq!(
        std::fs::read_to_string(ctx.workspace.join("b.txt")).unwrap(),
        "y y y"
    );
}

#[tokio::test]
async fn several_edits_go_in_one_call_or_not_at_all() {
    let (ctx, _rx) = ctx("multi").await;
    let path = ctx.workspace.join("c.py");
    WriteFile
        .run(
            &ctx,
            json!({"path": "c.py", "content": "a = 1\nb = 2\nc = 3\n"}),
        )
        .await;

    // Several changes in order, the second matching what the first left.
    let out = EditFile
        .run(
            &ctx,
            json!({"path": "c.py", "edits": [
                {"old_string": "a = 1", "new_string": "a = 10"},
                {"old_string": "a = 10\nb", "new_string": "a = 10\nbb"},
                {"old_string": "c = 3", "new_string": "c = 30"}
            ]}),
        )
        .await;
    assert!(out.ok, "{}", out.content);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "a = 10\nbb = 2\nc = 30\n"
    );
    // The result shows the lines as they now read, numbered.
    assert!(out.content.contains("     1\ta = 10"), "{}", out.content);
    assert!(out.content.contains("     3\tc = 30"), "{}", out.content);

    // One change that can't be made stops them all.
    let bad = EditFile
        .run(
            &ctx,
            json!({"path": "c.py", "edits": [
                {"old_string": "a = 10", "new_string": "a = 99"},
                {"old_string": "missing", "new_string": "x"}
            ]}),
        )
        .await;
    assert!(!bad.ok);
    assert!(bad.content.contains("edits[1]"), "{}", bad.content);
    assert!(
        bad.content.contains("Nothing was changed"),
        "{}",
        bad.content
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "a = 10\nbb = 2\nc = 30\n"
    );

    // A list and a single change at once is refused, as is neither.
    let both = json!({"path": "c.py", "old_string": "a", "new_string": "b",
        "edits": [{"old_string": "c", "new_string": "d"}]});
    assert!(EditFile.precheck(&ctx, &both).is_some());
    assert!(!EditFile.run(&ctx, both).await.ok);
    assert!(!EditFile.run(&ctx, json!({"path": "c.py"})).await.ok);
}

#[tokio::test]
async fn glob_and_grep_find_things() {
    let (ctx, _rx) = ctx("search").await;
    WriteFile
        .run(
            &ctx,
            json!({"path": "src/main.rs", "content": "fn main() {\n    hello();\n}\n"}),
        )
        .await;
    WriteFile
        .run(
            &ctx,
            json!({"path": "README.md", "content": "hello world\n"}),
        )
        .await;
    let g = Glob.run(&ctx, json!({"pattern": "**/*.rs"})).await;
    assert_eq!(g.content.trim(), "src/main.rs");
    let found = Grep
        .run(&ctx, json!({"pattern": "hello", "glob": "*.rs"}))
        .await;
    assert!(
        found.content.contains("src/main.rs:2:"),
        "{}",
        found.content
    );
    assert!(!found.content.contains("README"));
}

#[tokio::test]
async fn mcp_call_without_a_real_server_is_refused_before_asking() {
    let (ctx, _rx) = ctx("mcp-precheck").await;
    // Server and tool dropped, only the arguments sent (seen from a model on a long run).
    let blank = json!({"arguments": {"blueprint": "Buff_TownBellAlert"}});
    assert!(
        McpCall
            .precheck(&ctx, &blank)
            .unwrap()
            .contains("needs \"server\" and \"tool\"")
    );
    // A server that isn't set up (a model once invented "realty").
    let made_up = json!({"server": "realty", "tool": "domains", "arguments": {}});
    assert!(
        McpCall
            .precheck(&ctx, &made_up)
            .unwrap()
            .contains("no MCP server called realty")
    );
}

#[tokio::test]
async fn shell_asks_and_runs() {
    let (ctx, mut rx) = ctx("shell").await;
    let args = json!({"command": "echo thrall-ok"});
    assert!(matches!(
        Shell.effect(&ctx, &args),
        Effect::Run {
            risk: Risk::Low,
            everyday: true,
            ..
        }
    ));
    let out = Shell.run(&ctx, args).await;
    assert!(out.ok, "{}", out.content);
    assert!(out.content.contains("thrall-ok"));
    assert!(rx.recv().await.unwrap().contains("thrall-ok"));
}

#[tokio::test]
async fn shell_timeout_kills_the_command() {
    let (ctx, _rx) = ctx("timeout").await;
    let command = if cfg!(windows) {
        "Start-Sleep -Seconds 30"
    } else {
        "sleep 30"
    };
    let started = std::time::Instant::now();
    let out = Shell
        .run(&ctx, json!({"command": command, "timeout_secs": 2}))
        .await;
    assert!(!out.ok);
    assert!(out.content.contains("stopped after 2s"), "{}", out.content);
    assert!(started.elapsed().as_secs() < 20);
}

#[tokio::test]
async fn todo_write_saves_the_list() {
    let (ctx, _rx) = ctx("todo").await;
    let out = TodoWrite
        .run(
            &ctx,
            json!({"items": [
                {"text": "Read the code", "status": "done"},
                {"text": "Fix the bug", "status": "in_progress"}
            ]}),
        )
        .await;
    assert!(out.ok, "{}", out.content);
    assert_eq!(out.tasks.unwrap().len(), 2);
    let stored = ctx.store.tasks("u", "s").await.unwrap();
    assert_eq!(stored[1].status, "in_progress");
}

#[tokio::test]
async fn the_edit_guard_wants_a_fresh_read() {
    let (ctx, _rx) = ctx("guard").await;
    std::fs::write(ctx.workspace.join("old.txt"), "alpha\n").unwrap();
    let edit = json!({"path": "old.txt", "old_string": "alpha", "new_string": "beta"});

    // Never read: refused before anyone is asked.
    assert!(
        EditFile
            .precheck(&ctx, &edit)
            .unwrap()
            .contains("read old.txt")
    );
    assert!(!EditFile.run(&ctx, edit.clone()).await.ok);

    // Read, then changed behind Thrall's back: refused again.
    ReadFile.run(&ctx, json!({"path": "old.txt"})).await;
    assert!(EditFile.precheck(&ctx, &edit).is_none());
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(ctx.workspace.join("old.txt"), "alpha changed\n").unwrap();
    assert!(
        EditFile
            .precheck(&ctx, &edit)
            .unwrap()
            .contains("changed since")
    );

    // Read again: allowed, and the effect carries the diff for the approval card.
    ReadFile.run(&ctx, json!({"path": "old.txt"})).await;
    match EditFile.effect(&ctx, &edit) {
        Effect::Edit { diff: Some(d), .. } => assert!(d.unified.contains("+beta changed")),
        other => panic!("expected an edit with a diff, got {other:?}"),
    }
    assert!(EditFile.run(&ctx, edit).await.ok);

    // New files need no read.
    assert!(
        WriteFile
            .precheck(&ctx, &json!({"path": "new.txt", "content": "x"}))
            .is_none()
    );
}

#[tokio::test]
async fn interrupting_kills_a_running_command() {
    let (mut ctx, _rx) = ctx("cancel").await;
    let (interrupter, cancel) = Cancel::pair();
    ctx.cancel = cancel;
    let command = if cfg!(windows) {
        "Start-Sleep -Seconds 30"
    } else {
        "sleep 30"
    };
    let started = std::time::Instant::now();
    let run = Shell.run(&ctx, json!({"command": command}));
    let stop = async {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        interrupter.interrupt();
    };
    let (out, ()) = tokio::join!(run, stop);
    assert!(!out.ok);
    assert!(out.content.contains("interrupted"), "{}", out.content);
    assert!(started.elapsed().as_secs() < 15);
}

#[tokio::test]
async fn memory_refuses_secrets() {
    let (ctx, _rx) = ctx("memory").await;
    let out = thrall_tools::tools::MemorySave
        .run(
            &ctx,
            json!({"topic": "keys", "content": "openrouter key sk-or-v1-abcdefghijklmnopqrstuvwxyz0123456789"}),
        )
        .await;
    assert!(!out.ok);
    assert!(
        ctx.store
            .search_knowledge("u", "keys", 5)
            .await
            .unwrap()
            .is_empty()
    );
}
