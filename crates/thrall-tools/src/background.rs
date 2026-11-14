//! Programs left running after `shell` with `keep_running` (a dev server, a watcher): their output
//! keeps being collected so `shell_output` can show the latest, and `shell_stop` ends them with
//! everything they started. Each belongs to the person whose session started it, and they all
//! stop when the engine does.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use tokio::process::Child;
use tokio::sync::mpsc;

use crate::constants::BACKGROUND_OUTPUT_MAX_CHARS;

/// A program still running (or finished) in the background.
struct Entry {
    user_id: String,
    command: String,
    /// What to stop to end it (each with everything it started).
    pids: Vec<u32>,
    output: Arc<Mutex<String>>,
    /// `Some(code)` once it ended (`None` inside: killed).
    ended: Arc<Mutex<Option<Option<i32>>>>,
}

static RUNNING: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);
static NEXT: AtomicU64 = AtomicU64::new(1);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Keep only the newest output.
fn keep_tail(buf: &mut String) {
    if buf.len() > BACKGROUND_OUTPUT_MAX_CHARS {
        let mut cut = buf.len() - BACKGROUND_OUTPUT_MAX_CHARS;
        while !buf.is_char_boundary(cut) {
            cut += 1;
        }
        buf.drain(..cut);
    }
}

/// How many of `user_id`'s programs are still running.
pub fn running_for(user_id: &str) -> usize {
    lock(&RUNNING)
        .values()
        .filter(|e| e.user_id == user_id && lock(&e.ended).is_none())
        .count()
}

/// Hand a running program over: its output (`lines`, already started) keeps being collected.
/// Returns its id.
pub fn keep(
    user_id: &str,
    command: &str,
    child: Child,
    lines: mpsc::UnboundedReceiver<String>,
    so_far: String,
) -> String {
    let pids = child.id().into_iter().collect();
    track(user_id, command, pids, Some(child), lines, so_far)
}

/// Take over programs a command left running when it ended (`stop` ends them): their output
/// (`lines`) keeps being collected. Returns their id.
pub fn adopt(
    user_id: &str,
    command: &str,
    stop: Vec<u32>,
    lines: mpsc::UnboundedReceiver<String>,
    so_far: String,
) -> String {
    track(user_id, command, stop, None, lines, so_far)
}

fn track(
    user_id: &str,
    command: &str,
    pids: Vec<u32>,
    child: Option<Child>,
    mut lines: mpsc::UnboundedReceiver<String>,
    so_far: String,
) -> String {
    let id = format!("bg_{}", NEXT.fetch_add(1, Ordering::SeqCst));
    let output = Arc::new(Mutex::new(so_far));
    let ended = Arc::new(Mutex::new(None));
    {
        let output = output.clone();
        let ended = ended.clone();
        tokio::spawn(async move {
            while let Some(line) = lines.recv().await {
                let mut buf = lock(&output);
                buf.push_str(&line);
                keep_tail(&mut buf);
            }
            // Its output closed: it ended. Programs taken over can't be waited on for a code.
            let code = match child {
                Some(mut child) => child.wait().await.ok().and_then(|s| s.code()),
                None => None,
            };
            *lock(&ended) = Some(code);
        });
    }
    lock(&RUNNING).insert(
        id.clone(),
        Entry {
            user_id: user_id.to_owned(),
            command: command.to_owned(),
            pids,
            output,
            ended,
        },
    );
    id
}

/// The latest output of `id` (at most `max_chars`) and whether it's still running, if it's
/// `user_id`'s.
pub fn output(
    user_id: &str,
    id: &str,
    max_chars: usize,
) -> Option<(String, String, Option<Option<i32>>)> {
    let running = lock(&RUNNING);
    let e = running.get(id).filter(|e| e.user_id == user_id)?;
    let out = lock(&e.output);
    let tail: String = {
        let chars = out.chars().count();
        out.chars().skip(chars.saturating_sub(max_chars)).collect()
    };
    Some((e.command.clone(), tail, *lock(&e.ended)))
}

/// Stop `id` and everything it started, if it's `user_id`'s. False if there's no such program.
pub async fn stop(user_id: &str, id: &str) -> bool {
    let pids = {
        let mut running = lock(&RUNNING);
        match running.get(id) {
            Some(e) if e.user_id == user_id => running.remove(id).map(|e| e.pids),
            _ => return false,
        }
    };
    for pid in pids.unwrap_or_default() {
        crate::process::kill_tree(Some(pid)).await;
    }
    true
}

/// Stop every background program (the engine is stopping).
pub async fn stop_all() {
    let pids: Vec<u32> = lock(&RUNNING).drain().flat_map(|(_, e)| e.pids).collect();
    for pid in pids {
        crate::process::kill_tree(Some(pid)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_newest_output_is_kept() {
        let mut s = "é".repeat(BACKGROUND_OUTPUT_MAX_CHARS);
        keep_tail(&mut s);
        assert!(s.len() <= BACKGROUND_OUTPUT_MAX_CHARS);
        assert!(s.chars().all(|c| c == 'é'));
    }
}
