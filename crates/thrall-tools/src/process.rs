//! Running programs: output streamed live, and on timeout the whole process tree is killed so no
//! child is left holding pipes (a Python Thrall lesson).

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::constants::{
    LEFT_BEHIND_MARGIN_MS, LEFT_BEHIND_QUERY, LIVE_OUTPUT_MAX_CHARS, OUTPUT_DRAIN_MS,
    PYTHON_UNBUFFERED,
};
use crate::tool::Cancel;

/// How a program run ended.
pub struct Finished {
    /// Exit code (`None` if killed).
    pub code: Option<i32>,
    /// Stdout and stderr, interleaved by line as they arrived.
    pub output: String,
    /// Whether it was killed for taking too long.
    pub timed_out: bool,
    /// Whether it was killed because the user interrupted.
    pub cancelled: bool,
    /// What it left running when it ended, if something it started kept its output open (a
    /// windowed app, a server): no longer under the command, so stopping the command won't end it.
    pub left: Option<Left>,
}

/// Programs a command left running when it ended, and the rest of their output.
pub struct Left {
    /// Each one's process ID and name (empty if they couldn't be found).
    pub programs: Vec<(u32, String)>,
    /// What to stop to end them: their process IDs (Windows), or the command's process group.
    pub stop: Vec<u32>,
    /// Their output from here on.
    pub lines: mpsc::UnboundedReceiver<String>,
}

/// Run `program args…` in `cwd`, sending output lines to `live` as they come. Killed with its
/// whole process tree on timeout or when `cancel` fires.
pub async fn run(
    program: &str,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
    live: &mpsc::UnboundedSender<String>,
    mut cancel: Cancel,
) -> std::io::Result<Finished> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        // Python holds back what it prints when it isn't printing to a screen, so a server's
        // "listening" line would never arrive.
        .env(PYTHON_UNBUFFERED, "1");
    #[cfg(unix)]
    command.process_group(0);
    thrall_core::process::no_window(command.as_std_mut());
    let since = SystemTime::now();
    let mut child = command.spawn()?;
    let pid = child.id();

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    if let Some(out) = child.stdout.take() {
        tokio::spawn(pump(out, tx.clone()));
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(pump(err, tx.clone()));
    }
    drop(tx);

    let mut output = String::new();
    let mut live_sent = 0usize;
    let collect = async {
        while let Some(line) = rx.recv().await {
            if live_sent < LIVE_OUTPUT_MAX_CHARS {
                live_sent += line.len();
                // The client may have gone; the run still finishes.
                let _ = live.send(line.clone());
            }
            output.push_str(&line);
        }
    };

    enum End {
        Exited(std::io::Result<std::process::ExitStatus>, bool),
        TimedOut,
        Cancelled,
    }
    let end = {
        let finish = tokio::time::timeout(timeout, async {
            tokio::pin!(collect);
            tokio::select! {
                () = &mut collect => (child.wait().await, false),
                status = child.wait() => {
                    // It ended. Its output is done too, unless something it started inherited
                    // the output and is still running: don't wait for that to end.
                    let drain = Duration::from_millis(OUTPUT_DRAIN_MS);
                    let left = tokio::time::timeout(drain, &mut collect).await.is_err();
                    (status, left)
                }
            }
        });
        tokio::select! {
            result = finish => match result {
                Ok((status, left)) => End::Exited(status, left),
                Err(_) => End::TimedOut,
            },
            () = cancel.cancelled() => End::Cancelled,
        }
    };

    match end {
        End::Exited(status, left_running) => {
            let left = if left_running {
                Some(left_behind(pid, since, rx).await)
            } else {
                None
            };
            Ok(Finished {
                code: status?.code(),
                output,
                timed_out: false,
                cancelled: false,
                left,
            })
        }
        stopped => {
            kill_tree(pid).await;
            let _ = child.kill().await;
            Ok(Finished {
                code: None,
                output,
                timed_out: matches!(stopped, End::TimedOut),
                cancelled: matches!(stopped, End::Cancelled),
                left: None,
            })
        }
    }
}

/// How a run that watches for a line ended.
pub enum Watched {
    /// It ended (or was stopped) before the line appeared.
    Ended(Finished),
    /// The line appeared and the program was stopped there.
    Seen(String),
    /// The line appeared and the program keeps running in the background, with this id.
    Kept {
        /// Its output up to the line.
        output: String,
        /// The `background` id.
        id: String,
    },
}

/// Like [`run`], but hand back as soon as a line matches `wait_for`: then the program is
/// stopped, or with `keep_running` left running in the background as `user_id`'s. With no
/// `wait_for`, a program kept running is handed back once `timeout` passes and it's still going.
#[allow(clippy::too_many_arguments)]
pub async fn run_until(
    program: &str,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
    live: &mpsc::UnboundedSender<String>,
    mut cancel: Cancel,
    wait_for: Option<&regex::Regex>,
    keep_running: Option<(&str, &str)>,
) -> std::io::Result<Watched> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        // Python holds back what it prints when it isn't printing to a screen, so a server's
        // "listening" line would never arrive.
        .env(PYTHON_UNBUFFERED, "1");
    #[cfg(unix)]
    command.process_group(0);
    thrall_core::process::no_window(command.as_std_mut());
    let since = SystemTime::now();
    let mut child = command.spawn()?;
    let pid = child.id();

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    if let Some(out) = child.stdout.take() {
        tokio::spawn(pump(out, tx.clone()));
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(pump(err, tx.clone()));
    }
    drop(tx);

    enum End {
        Seen,
        Closed,
        LeftBehind,
        TimedOut,
        Cancelled,
    }
    let mut output = String::new();
    let mut live_sent = 0usize;
    let deadline = tokio::time::Instant::now() + timeout;
    // The command's own end, and how long its output then gets to close.
    let mut exited = None;
    let mut drained = deadline;
    let end = loop {
        tokio::select! {
            line = rx.recv() => match line {
                Some(line) => {
                    if live_sent < LIVE_OUTPUT_MAX_CHARS {
                        live_sent += line.len();
                        let _ = live.send(line.clone());
                    }
                    let seen = wait_for.is_some_and(|w| w.is_match(&line));
                    output.push_str(&line);
                    if seen {
                        break End::Seen;
                    }
                }
                None => break End::Closed,
            },
            // It ended. Its output is done too, unless something it started inherited the output
            // and is still running (a windowed app prints nothing, so its line never comes).
            status = child.wait(), if exited.is_none() => {
                exited = Some(status);
                let drain = tokio::time::Instant::now() + Duration::from_millis(OUTPUT_DRAIN_MS);
                drained = drain.min(deadline);
            }
            () = tokio::time::sleep_until(drained), if exited.is_some() => break End::LeftBehind,
            () = tokio::time::sleep_until(deadline) => break End::TimedOut,
            () = cancel.cancelled() => break End::Cancelled,
        }
    };
    let end = match end {
        End::TimedOut if exited.is_some() => End::LeftBehind,
        end => end,
    };
    match end {
        End::Seen => match keep_running {
            // The command's gone: keep track of what it left instead.
            Some((user_id, shown)) if exited.is_some() => {
                let left = left_behind(pid, since, rx).await;
                let id =
                    crate::background::adopt(user_id, shown, left.stop, left.lines, output.clone());
                Ok(Watched::Kept { output, id })
            }
            Some((user_id, shown)) => {
                let id = crate::background::keep(user_id, shown, child, rx, output.clone());
                Ok(Watched::Kept { output, id })
            }
            None => {
                stop(pid, since, exited.is_some()).await;
                let _ = child.kill().await;
                Ok(Watched::Seen(output))
            }
        },
        // Every pipe closed: it's ending (or handed its output elsewhere); wait for it, within
        // the time left.
        End::Closed => {
            let status = match exited {
                Some(status) => Ok(status),
                None => tokio::time::timeout_at(deadline, child.wait()).await,
            };
            match status {
                Ok(status) => Ok(Watched::Ended(Finished {
                    code: status?.code(),
                    output,
                    timed_out: false,
                    cancelled: false,
                    left: None,
                })),
                Err(_) => {
                    kill_tree(pid).await;
                    let _ = child.kill().await;
                    Ok(Watched::Ended(Finished {
                        code: None,
                        output,
                        timed_out: true,
                        cancelled: false,
                        left: None,
                    }))
                }
            }
        }
        // It ended and left something running: hand that back rather than waiting on a line it
        // may never print, or stopping the command and leaving it behind.
        End::LeftBehind => {
            let code = match exited {
                Some(status) => status?.code(),
                None => None,
            };
            Ok(Watched::Ended(Finished {
                code,
                output,
                timed_out: false,
                cancelled: false,
                left: Some(left_behind(pid, since, rx).await),
            }))
        }
        End::TimedOut if wait_for.is_none() && keep_running.is_some() => {
            let (user_id, shown) = keep_running.unwrap_or_default();
            let id = crate::background::keep(user_id, shown, child, rx, output.clone());
            Ok(Watched::Kept { output, id })
        }
        stopped => {
            stop(pid, since, exited.is_some()).await;
            let _ = child.kill().await;
            Ok(Watched::Ended(Finished {
                code: None,
                output,
                timed_out: matches!(stopped, End::TimedOut),
                cancelled: matches!(stopped, End::Cancelled),
                left: None,
            }))
        }
    }
}

/// Stop a command and everything it started, including what it left running if it already
/// ended (no longer under it).
async fn stop(pid: Option<u32>, since: SystemTime, ended: bool) {
    kill_tree(pid).await;
    #[cfg(windows)]
    if ended && let Some(pid) = pid {
        for (left, _) in list_left(pid, since).await {
            kill_tree(Some(left)).await;
        }
    }
    // Elsewhere they're in the command's process group, stopped with it.
    #[cfg(not(windows))]
    let _ = (since, ended);
}

/// What a command that ended left running, with the rest of its output.
async fn left_behind(
    pid: Option<u32>,
    since: SystemTime,
    lines: mpsc::UnboundedReceiver<String>,
) -> Left {
    let programs = match pid {
        Some(pid) => list_left(pid, since).await,
        None => Vec::new(),
    };
    #[cfg(windows)]
    let stop = programs.iter().map(|(id, _)| *id).collect();
    #[cfg(not(windows))]
    let stop = match pid {
        Some(group) if !programs.is_empty() => vec![group],
        _ => Vec::new(),
    };
    Left {
        programs,
        stop,
        lines,
    }
}

/// The programs command `pid` started since `since` that are still running after it ended: on
/// Windows those whose parent it was, elsewhere those in its process group.
async fn list_left(pid: u32, since: SystemTime) -> Vec<(u32, String)> {
    let ms = since
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
        .saturating_sub(LEFT_BEHIND_MARGIN_MS);
    #[cfg(windows)]
    let mut command = {
        let query = LEFT_BEHIND_QUERY
            .replace("{pid}", &pid.to_string())
            .replace("{ms}", &ms.to_string());
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &query]);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let _ = (ms, LEFT_BEHIND_QUERY);
        let mut command = Command::new("pgrep");
        command.args(["-l", "-g", &pid.to_string()]);
        command
    };
    command.stdin(Stdio::null()).stderr(Stdio::null());
    thrall_core::process::no_window(command.as_std_mut());
    let Ok(out) = command.output().await else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let (id, name) = line.trim().split_once(' ')?;
            Some((id.parse().ok()?, name.trim().to_owned()))
        })
        .collect()
}

async fn pump(stream: impl AsyncRead + Unpin, tx: mpsc::UnboundedSender<String>) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if tx.send(String::from_utf8_lossy(&buf).into_owned()).is_err() {
                    break;
                }
            }
        }
    }
}

/// Kill a process and everything it started.
pub(crate) async fn kill_tree(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    let pid = pid.to_string();
    #[cfg(windows)]
    let result = {
        let mut taskkill = Command::new("taskkill");
        taskkill
            .args(["/PID", &pid, "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        thrall_core::process::no_window(taskkill.as_std_mut());
        taskkill.status().await
    };
    #[cfg(not(windows))]
    let result = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    if let Err(e) = result {
        tracing::warn!(%e, pid, "couldn't kill process tree");
    }
}
