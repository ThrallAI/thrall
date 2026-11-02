//! `shell` with `wait_for`: hand back as soon as a line appears, and keep the program running in
//! the background when asked.

use std::time::{Duration, Instant};

use thrall_tools::process::{Watched, run_until};
use thrall_tools::tools::shell_command;
use tokio::sync::mpsc;

/// Prints a line, then keeps running for a long time (well past the test's patience).
fn slow_server() -> String {
    if cfg!(windows) {
        "[Console]::Out.WriteLine('starting'); [Console]::Out.WriteLine('Listening on 3000'); \
         [Console]::Out.Flush(); Start-Sleep 60"
            .to_owned()
    } else {
        "echo starting; echo 'Listening on 3000'; sleep 60".to_owned()
    }
}

fn pattern(text: &str) -> regex::Regex {
    regex::RegexBuilder::new(text)
        .case_insensitive(true)
        .build()
        .unwrap()
}

#[tokio::test]
async fn hands_back_when_the_line_appears() {
    let (program, args) = shell_command(&slow_server());
    let (live, _rx) = mpsc::unbounded_channel();
    let started = Instant::now();
    let out = run_until(
        program,
        &args,
        &std::env::temp_dir(),
        Duration::from_secs(40),
        &live,
        thrall_tools::Cancel::never(),
        &pattern("listening on"),
        None,
    )
    .await
    .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "didn't wait for the program to end"
    );
    match out {
        Watched::Seen(output) => assert!(output.contains("Listening on 3000"), "{output}"),
        _ => panic!("expected the line to be seen"),
    }
}

#[tokio::test]
async fn keeps_running_in_the_background_until_stopped() {
    let (program, args) = shell_command(&slow_server());
    let (live, _rx) = mpsc::unbounded_channel();
    let out = run_until(
        program,
        &args,
        &std::env::temp_dir(),
        Duration::from_secs(40),
        &live,
        thrall_tools::Cancel::never(),
        &pattern("Listening"),
        Some(("u_test", "the server")),
    )
    .await
    .unwrap();
    let Watched::Kept { id, .. } = out else {
        panic!("expected it to keep running");
    };
    let (command, output, ended) = thrall_tools::background::output("u_test", &id, 1_000).unwrap();
    assert_eq!(command, "the server");
    assert!(output.contains("Listening on 3000"));
    assert_eq!(ended, None, "still running");
    assert!(
        thrall_tools::background::output("u_other", &id, 1_000).is_none(),
        "someone else can't read it"
    );
    assert!(!thrall_tools::background::stop("u_other", &id).await);
    assert!(thrall_tools::background::stop("u_test", &id).await);
    assert!(thrall_tools::background::output("u_test", &id, 1_000).is_none());
}

#[tokio::test]
async fn a_line_that_never_comes_times_out() {
    let quiet = if cfg!(windows) {
        "Start-Sleep 60"
    } else {
        "sleep 60"
    };
    let (program, args) = shell_command(quiet);
    let (live, _rx) = mpsc::unbounded_channel();
    let out = run_until(
        program,
        &args,
        &std::env::temp_dir(),
        Duration::from_secs(3),
        &live,
        thrall_tools::Cancel::never(),
        &pattern("ready"),
        None,
    )
    .await
    .unwrap();
    assert!(matches!(out, Watched::Ended(done) if done.timed_out));
}
