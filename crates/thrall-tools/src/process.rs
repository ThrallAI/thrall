//! Running programs: output streamed live, and on timeout the whole process tree is killed so no
//! child is left holding pipes (a Python Thrall lesson).

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::constants::LIVE_OUTPUT_MAX_CHARS;
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
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    thrall_core::process::no_window(command.as_std_mut());
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
        Exited(std::io::Result<std::process::ExitStatus>),
        TimedOut,
        Cancelled,
    }
    let end = {
        let finish = tokio::time::timeout(timeout, async {
            collect.await;
            child.wait().await
        });
        tokio::select! {
            result = finish => match result {
                Ok(status) => End::Exited(status),
                Err(_) => End::TimedOut,
            },
            () = cancel.cancelled() => End::Cancelled,
        }
    };

    match end {
        End::Exited(status) => Ok(Finished {
            code: status?.code(),
            output,
            timed_out: false,
            cancelled: false,
        }),
        stopped => {
            kill_tree(pid).await;
            let _ = child.kill().await;
            Ok(Finished {
                code: None,
                output,
                timed_out: matches!(stopped, End::TimedOut),
                cancelled: matches!(stopped, End::Cancelled),
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
/// stopped, or with `keep_running` left running in the background as `user_id`'s.
#[allow(clippy::too_many_arguments)]
pub async fn run_until(
    program: &str,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
    live: &mpsc::UnboundedSender<String>,
    mut cancel: Cancel,
    wait_for: &regex::Regex,
    keep_running: Option<(&str, &str)>,
) -> std::io::Result<Watched> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    thrall_core::process::no_window(command.as_std_mut());
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
        TimedOut,
        Cancelled,
    }
    let mut output = String::new();
    let mut live_sent = 0usize;
    let deadline = tokio::time::Instant::now() + timeout;
    let end = loop {
        tokio::select! {
            line = rx.recv() => match line {
                Some(line) => {
                    if live_sent < LIVE_OUTPUT_MAX_CHARS {
                        live_sent += line.len();
                        let _ = live.send(line.clone());
                    }
                    let seen = wait_for.is_match(&line);
                    output.push_str(&line);
                    if seen {
                        break End::Seen;
                    }
                }
                None => break End::Closed,
            },
            () = tokio::time::sleep_until(deadline) => break End::TimedOut,
            () = cancel.cancelled() => break End::Cancelled,
        }
    };
    match end {
        End::Seen => match keep_running {
            Some((user_id, shown)) => {
                let id = crate::background::keep(user_id, shown, child, rx, output.clone());
                Ok(Watched::Kept { output, id })
            }
            None => {
                kill_tree(pid).await;
                let _ = child.kill().await;
                Ok(Watched::Seen(output))
            }
        },
        // Every pipe closed: it's ending (or handed its output elsewhere); wait for it, within
        // the time left.
        End::Closed => match tokio::time::timeout_at(deadline, child.wait()).await {
            Ok(status) => Ok(Watched::Ended(Finished {
                code: status?.code(),
                output,
                timed_out: false,
                cancelled: false,
            })),
            Err(_) => {
                kill_tree(pid).await;
                let _ = child.kill().await;
                Ok(Watched::Ended(Finished {
                    code: None,
                    output,
                    timed_out: true,
                    cancelled: false,
                }))
            }
        },
        stopped => {
            kill_tree(pid).await;
            let _ = child.kill().await;
            Ok(Watched::Ended(Finished {
                code: None,
                output,
                timed_out: matches!(stopped, End::TimedOut),
                cancelled: matches!(stopped, End::Cancelled),
            }))
        }
    }
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
