//! What runs beside the conversation while Thrall is open: background helpers, and the scheduler
//! that runs due jobs. Clients start these once (`start`); a restart stops the old engine's.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use thrall_core::constants::id_prefix;
use thrall_core::ids::{new_id, now_ms};
use thrall_protocol::{Event, EventFrame, NoticeLevel};
use thrall_store::{Job, JobRun, NewEvent};
use thrall_tools::tools::schedule::next_run;
use tokio::sync::mpsc;

use crate::constants::{BROKEN_JOB_RETRY_MS, EVENT_CHANNEL_CAPACITY, SCHEDULER_TICK};
use crate::state::AppState;

/// Record an event that happened outside a turn and tell whoever's listening.
pub async fn announce(state: &AppState, session_id: &str, event: Event) {
    let at = now_ms();
    let payload = match serde_json::to_string(&event) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(%e, "couldn't encode an event");
            return;
        }
    };
    let seq = state
        .store
        .append_event(NewEvent {
            user_id: state.user_id.clone(),
            session_id: session_id.to_owned(),
            turn_id: None,
            kind: event.type_name().to_owned(),
            payload,
            at,
        })
        .await
        .ok();
    // Nobody listening is fine.
    let _ = state
        .notices
        .send(EventFrame::new(session_id, seq, at, event));
}

/// Start the background-helper runner and the scheduler (once per engine).
pub fn start(state: Arc<AppState>) {
    if state.runners_started.swap(true, Ordering::SeqCst) {
        return;
    }
    let queue = state.background_rx.lock().ok().and_then(|mut q| q.take());
    if let Some(mut queue) = queue {
        let st = state.clone();
        tokio::spawn(async move {
            while let Some(task) = queue.recv().await {
                if st.stopping.load(Ordering::SeqCst) {
                    break;
                }
                // A helper works as the person whose session started it.
                let st = st.for_user(&task.user_id);
                tokio::spawn(async move { crate::workers::run_background(&st, task).await });
            }
        });
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SCHEDULER_TICK);
        loop {
            tick.tick().await;
            if state.stopping.load(Ordering::SeqCst) {
                break;
            }
            run_due_jobs(&state).await;
            crate::heartbeat::maybe_beat(&state);
        }
    });
}

/// Run every job that's due, everyone's (each in its own task, as the job's person).
pub async fn run_due_jobs(state: &Arc<AppState>) {
    let people = match state.store.list_people().await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(%e, "couldn't check scheduled jobs");
            return;
        }
    };
    for person in people {
        run_due_jobs_of(&Arc::new(state.for_user(&person.id))).await;
    }
}

/// Run state's person's due jobs.
async fn run_due_jobs_of(state: &Arc<AppState>) {
    let now = now_ms();
    let jobs = match state.store.due_jobs(&state.user_id, now).await {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!(%e, "couldn't check scheduled jobs");
            return;
        }
    };
    for job in jobs {
        // Book the next run first, so a slow run can't start twice. A one-off is booked for
        // never, and removed once it has run.
        let once = thrall_tools::tools::schedule::is_once(&job.schedule);
        let next = if once {
            i64::MAX
        } else {
            next_run(&job.schedule, now).unwrap_or(now + BROKEN_JOB_RETRY_MS)
        };
        if let Err(e) = state
            .store
            .job_ran(&state.user_id, &job.id, now, next)
            .await
        {
            tracing::warn!(%e, job = job.id, "couldn't book a job's next run");
            continue;
        }
        let st = state.clone();
        tokio::spawn(async move {
            let id = job.id.clone();
            run_job(&st, job).await;
            if once && let Err(e) = st.store.delete_job(&st.user_id, &id).await {
                tracing::warn!(%e, job = id, "couldn't remove a one-off job that ran");
            }
        });
    }
}

/// One scheduled run: the job's prompt as a turn in its own session, with nobody to ask.
pub async fn run_job(state: &AppState, job: Job) {
    let Some(session_id) = job.session_id.clone() else {
        return;
    };
    let started = now_ms();
    announce(
        state,
        &session_id,
        Event::AgentStarted {
            agent: job.id.clone(),
            title: format!("Scheduled: {}", job.title),
        },
    )
    .await;
    // The run's own events go to listeners (a client watching that session shows them live).
    let (tx, mut rx) = mpsc::channel::<EventFrame>(EVENT_CHANNEL_CAPACITY);
    let notices = state.notices.clone();
    let forward = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            let _ = notices.send(frame);
        }
    });
    state.set_unattended(&session_id, true);
    let result = crate::turn::run(state, &session_id, &job.prompt, &tx).await;
    state.set_unattended(&session_id, false);
    drop(tx);
    let _ = forward.await;

    let (status, summary) = match &result {
        Ok(s) => (
            format!("{:?}", s.status).to_lowercase(),
            s.reply
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("(no reply)")
                .chars()
                .take(160)
                .collect::<String>(),
        ),
        Err(e) => ("failed".to_owned(), e.to_string()),
    };
    if let Err(e) = state
        .store
        .add_job_run(
            &state.user_id,
            JobRun {
                id: new_id(id_prefix::JOB_RUN),
                job_id: job.id.clone(),
                started_at: started,
                ended_at: now_ms(),
                status: status.clone(),
                summary: summary.clone(),
            },
        )
        .await
    {
        tracing::warn!(%e, "couldn't record a job run");
    }
    announce(
        state,
        &session_id,
        Event::AgentCompleted {
            agent: job.id.clone(),
            title: format!("Scheduled: {}", job.title),
            status: if status == "done" {
                "done".into()
            } else {
                status
            },
            summary,
        },
    )
    .await;
    if result.is_err() {
        announce(
            state,
            &session_id,
            Event::Notice {
                level: NoticeLevel::Warning,
                text: format!("Scheduled job “{}” didn't run properly.", job.title),
            },
        )
        .await;
    }
}
