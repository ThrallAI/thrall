//! A command that starts something on its own and ends: Thrall hands back straight away and says
//! something is still running, rather than waiting for it until the timeout.

use std::time::{Duration, Instant};

use thrall_tools::process::{Watched, run, run_until};
use thrall_tools::tools::shell_command;
use tokio::sync::mpsc;

/// Starts a program that keeps the output open for a while, prints its process ID, and ends.
fn starts_and_leaves() -> String {
    if cfg!(windows) {
        let log = std::env::temp_dir().join("thrall-left-running.log");
        format!(
            "(Start-Process -FilePath powershell -ArgumentList '-NoProfile','-Command',\
             'Start-Sleep 20' -RedirectStandardOutput '{}' -WindowStyle Hidden -PassThru).Id",
            log.display()
        )
    } else {
        "sleep 20 & echo $!".to_owned()
    }
}

fn stop(pid: &str) {
    let pid = pid.trim();
    if cfg!(windows) {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", pid, "/T", "/F"])
            .output();
    } else {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", pid])
            .output();
    }
}

#[tokio::test]
async fn a_program_left_running_is_reported_not_waited_for() {
    let (program, args) = shell_command(&starts_and_leaves());
    let (live, _rx) = mpsc::unbounded_channel();
    let started = Instant::now();
    let finished = run(
        program,
        &args,
        &std::env::temp_dir(),
        Duration::from_secs(40),
        &live,
        thrall_tools::Cancel::never(),
    )
    .await
    .unwrap();
    stop(&finished.output);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "waited for the program it left running"
    );
    assert!(!finished.timed_out);
    assert_eq!(finished.code, Some(0));
    let left = finished.left.expect("reported as left running");
    // And it's found, so it can be kept track of and stopped.
    let pid: u32 = finished.output.trim().parse().unwrap();
    assert!(
        left.programs.iter().any(|(id, _)| *id == pid),
        "{pid} not in {:?}",
        left.programs
    );
}

#[tokio::test]
async fn waiting_for_a_line_hands_back_what_was_left_running() {
    let (program, args) = shell_command(&starts_and_leaves());
    let (live, _rx) = mpsc::unbounded_channel();
    let started = Instant::now();
    let watched = run_until(
        program,
        &args,
        &std::env::temp_dir(),
        Duration::from_secs(40),
        &live,
        thrall_tools::Cancel::never(),
        Some(&regex::Regex::new("never printed").unwrap()),
        None,
    )
    .await
    .unwrap();
    let Watched::Ended(finished) = watched else {
        panic!("expected it to end");
    };
    stop(&finished.output);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "waited for a line from the program it left running"
    );
    assert!(!finished.timed_out);
    let pid: u32 = finished.output.trim().parse().unwrap();
    let left = finished.left.expect("reported as left running");
    assert!(left.programs.iter().any(|(id, _)| *id == pid));
}

#[tokio::test]
async fn a_quick_command_isnt_reported_as_leaving_anything() {
    let (program, args) = shell_command("echo hello");
    let (live, _rx) = mpsc::unbounded_channel();
    let finished = run(
        program,
        &args,
        &std::env::temp_dir(),
        Duration::from_secs(40),
        &live,
        thrall_tools::Cancel::never(),
    )
    .await
    .unwrap();
    assert!(finished.output.contains("hello"));
    assert!(finished.left.is_none());
}
