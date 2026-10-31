//! Named constants for `thrall-bench`.

use std::time::Duration;

/// Port of the bench's page, on 127.0.0.1 only (the engine uses 7417, the phone link 7419).
pub const BENCH_PORT: u16 = 7425;

/// The bench's own folder inside Thrall's data folder (runs and presets; never the real database).
pub const BENCH_DIR: &str = "bench";
/// Saved runs, one folder each.
pub const RUNS_DIR: &str = "runs";
/// Saved lane setups.
pub const PRESETS_FILE: &str = "presets.json";
/// A finished run's results.
pub const RESULT_FILE: &str = "result.json";
/// A lane's copy of the task, where its turn works.
pub const WORK_DIR: &str = "work";
/// A lane's own throwaway database.
pub const LANE_DB: &str = "lane.db";

/// Test tasks, relative to where the bench is started (the repo root), unless `--tasks` says.
pub const DEFAULT_TASKS_DIR: &str = "bench/tasks";
/// Each task folder's description: `prompt`, optional `title` and `check`.
pub const TASK_FILE: &str = "task.toml";
/// A check prints this, then `passed/total`, to be scored by tests passed (e.g. `SCORE 7/9`).
pub const SCORE_PREFIX: &str = "SCORE ";
/// A task's folder of tests the lanes don't see: copied into the lane's folder only after its
/// turn, just before the check, so a model can't change the tests to pass them.
pub const HIDDEN_DIR: &str = "hidden";
/// The shell a check runs in, and its arguments before the command.
#[cfg(windows)]
pub const CHECK_SHELL: (&str, &[&str]) =
    ("powershell", &["-NoProfile", "-NonInteractive", "-Command"]);
/// The shell a check runs in, and its arguments before the command.
#[cfg(not(windows))]
pub const CHECK_SHELL: (&str, &[&str]) = ("sh", &["-c"]);

/// How often a running lane's numbers are refreshed (and its budget checked).
pub const REFRESH_INTERVAL: Duration = Duration::from_millis(500);
/// Most lanes in one run.
pub const MAX_LANES: usize = 8;
/// Budget per lane when none is given, in US dollars.
pub const DEFAULT_BUDGET_USD: f64 = 2.0;
/// Most lines kept in a lane's feed.
pub const FEED_MAX_LINES: usize = 300;
/// Most characters of one feed line.
pub const FEED_LINE_MAX_CHARS: usize = 240;
/// Events buffered between a lane's turn and its watcher.
pub const EVENT_BUFFER: usize = 1024;
/// Longest a task's check may run.
pub const CHECK_TIMEOUT_SECS: u64 = 600;
/// Most characters of a check's output kept (the end of it).
pub const CHECK_OUTPUT_MAX_CHARS: usize = 6000;
/// Folders not copied into a lane (build output and dependencies get rebuilt).
pub const SKIP_COPY: &[&str] = &["target", "node_modules", ".git", "__pycache__", ".venv"];
/// The OpenRouter environment variable or keychain entry is used, as Thrall itself does.
pub const KEY_HELP: &str = "No OpenRouter key: run `thrall key set openrouter` first (the bench \
uses the same key as Thrall).";
