//! Scheduled jobs: Thrall is asked something on a schedule. Each job gets its own session; runs
//! happen while Thrall is open, and never ask anything (nobody may be there to answer), so a run
//! only does what its session's mode allows without asking.

use std::str::FromStr;

use chrono::{DateTime, Local};
use croner::Cron;
use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};
use thrall_core::constants::id_prefix;
use thrall_core::ids::{new_id, now_ms};
use thrall_gates::Effect;
use thrall_protocol::Risk;
use thrall_store::{Job, NewSession};

use crate::constants::{MIN_INTERVAL_SECS, SCHEDULED_TITLE_PREFIX};
use crate::tool::{Tool, ToolContext, ToolOutput, arg_str, parse_args};

/// When a schedule next fires after `after_ms` (Unix ms). Accepts cron (`0 9 * * 1-5`) or
/// `every 30m` / `every 2h` / `every 1d`.
pub fn next_run(schedule: &str, after_ms: i64) -> Result<i64, String> {
    let schedule = schedule.trim();
    if let Some(interval) = schedule.strip_prefix("every ") {
        let interval = interval.trim();
        let (number, unit) = interval.split_at(interval.len().saturating_sub(1));
        let n: i64 = number.trim().parse().map_err(|_| {
            format!("can't read `{schedule}`; try `every 30m`, `every 2h` or `every 1d`")
        })?;
        let secs = match unit {
            "m" => n * 60,
            "h" => n * 3600,
            "d" => n * 86_400,
            _ => {
                return Err(format!(
                    "can't read `{schedule}`; use m, h or d, like `every 30m`"
                ));
            }
        };
        if secs < MIN_INTERVAL_SECS {
            return Err(format!(
                "the shortest interval is {} minutes",
                MIN_INTERVAL_SECS / 60
            ));
        }
        return Ok(after_ms + secs * 1000);
    }
    let cron = Cron::from_str(schedule).map_err(|e| {
        format!("`{schedule}` isn't a schedule I understand ({e}); try `0 9 * * *` (9:00 daily)")
    })?;
    let after: DateTime<Local> = DateTime::from_timestamp_millis(after_ms)
        .ok_or("bad time")?
        .with_timezone(&Local);
    let next = cron
        .find_next_occurrence(&after, false)
        .map_err(|e| format!("`{schedule}` never fires ({e})"))?;
    Ok(next.timestamp_millis())
}

/// Readable local time.
pub fn local_time(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms)
        .map(|t| t.with_timezone(&Local).format("%a %d %b %H:%M").to_string())
        .unwrap_or_default()
}

// ---------- schedule ----------

/// Create a scheduled job.
pub struct Schedule;

#[derive(Deserialize)]
struct ScheduleArgs {
    title: String,
    when: String,
    prompt: String,
}

impl Tool for Schedule {
    fn name(&self) -> &'static str {
        "schedule"
    }
    fn description(&self) -> &'static str {
        "Schedule a recurring job: Thrall is sent `prompt` on the schedule `when` — cron (\"0 9 * * \
         *\" is 9:00 every day, \"0 8 * * 1-5\" weekdays at 8) or \"every 30m\" / \"every 2h\" / \
         \"every 1d\". Runs happen while Thrall is open, in their own session, and never ask for \
         permission: they can read, search and remember, but not edit or run commands unless that \
         session's mode allows it. Write the prompt so it stands on its own."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Short name" },
                "when": { "type": "string", "description": "Cron expression or every Nm/Nh/Nd" },
                "prompt": { "type": "string", "description": "What Thrall is asked each run" }
            },
            "required": ["title", "when", "prompt"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Scheduling {}", arg_str(args, "title"))
    }
    fn precheck(&self, _ctx: &ToolContext, args: &Value) -> Option<String> {
        next_run(arg_str(args, "when"), now_ms()).err()
    }
    fn effect(&self, _ctx: &ToolContext, args: &Value) -> Effect {
        Effect::Run {
            summary: format!(
                "Schedule “{}” ({}): {}",
                arg_str(args, "title"),
                arg_str(args, "when"),
                arg_str(args, "prompt")
            ),
            risk: Risk::Medium,
            grant: None,
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let a: ScheduleArgs = match parse_args(self.name(), args) {
                Ok(a) => a,
                Err(e) => return ToolOutput::error(e),
            };
            let now = now_ms();
            let next = match next_run(&a.when, now) {
                Ok(n) => n,
                Err(e) => return ToolOutput::error(e),
            };
            let session = match ctx
                .store
                .create_session(NewSession {
                    id: new_id(id_prefix::SESSION),
                    user_id: ctx.user_id.clone(),
                    workspace: Some(ctx.workspace.to_string_lossy().into_owned()),
                    at: now,
                })
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    return ToolOutput::error(format!("couldn't create the job's session: {e}"));
                }
            };
            let _ = ctx
                .store
                .set_session_title(
                    &ctx.user_id,
                    &session.id,
                    &format!("{SCHEDULED_TITLE_PREFIX}{}", a.title),
                )
                .await;
            let job = Job {
                id: new_id(id_prefix::JOB),
                session_id: Some(session.id),
                title: a.title.clone(),
                schedule: a.when.clone(),
                prompt: a.prompt,
                enabled: true,
                last_run_at: None,
                next_run_at: next,
            };
            let id = job.id.clone();
            match ctx.store.add_job(&ctx.user_id, job).await {
                Ok(()) => ToolOutput::ok(
                    format!(
                        "Scheduled “{}” ({id}). First run: {}.",
                        a.title,
                        local_time(next)
                    ),
                    format!("First run {}", local_time(next)),
                ),
                Err(e) => ToolOutput::error(format!("couldn't save the job: {e}")),
            }
        })
    }
}

// ---------- schedule_list ----------

/// List scheduled jobs.
pub struct ScheduleList;

impl Tool for ScheduleList {
    fn name(&self) -> &'static str {
        "schedule_list"
    }
    fn description(&self) -> &'static str {
        "List scheduled jobs: id, title, schedule, next run, and what each is asked."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn title(&self, _args: &Value) -> String {
        "Listing scheduled jobs".to_owned()
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, _args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            match ctx.store.list_jobs(&ctx.user_id).await {
                Ok(jobs) if jobs.is_empty() => ToolOutput::ok("No scheduled jobs.", "None"),
                Ok(jobs) => {
                    let out = jobs
                        .iter()
                        .map(|j| {
                            format!(
                                "- {} · {} · `{}` · next {} · {}",
                                j.id,
                                j.title,
                                j.schedule,
                                local_time(j.next_run_at),
                                j.prompt
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    ToolOutput::ok(out, format!("{} jobs", jobs.len()))
                }
                Err(e) => ToolOutput::error(format!("couldn't list jobs: {e}")),
            }
        })
    }
}

// ---------- schedule_remove ----------

/// Delete a scheduled job.
pub struct ScheduleRemove;

impl Tool for ScheduleRemove {
    fn name(&self) -> &'static str {
        "schedule_remove"
    }
    fn description(&self) -> &'static str {
        "Delete a scheduled job by id (see schedule_list)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"]
        })
    }
    fn title(&self, args: &Value) -> String {
        format!("Removing scheduled job {}", arg_str(args, "id"))
    }
    fn effect(&self, _ctx: &ToolContext, args: &Value) -> Effect {
        Effect::Run {
            summary: format!("Delete scheduled job {}", arg_str(args, "id")),
            risk: Risk::Medium,
            grant: None,
        }
    }
    fn run<'a>(&'a self, ctx: &'a ToolContext, args: Value) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let id = arg_str(&args, "id").to_owned();
            match ctx.store.delete_job(&ctx.user_id, &id).await {
                Ok(true) => ToolOutput::ok(format!("Deleted {id}."), "Deleted"),
                Ok(false) => ToolOutput::error(format!("there's no job {id}")),
                Err(e) => ToolOutput::error(format!("couldn't delete it: {e}")),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_and_cron_parse() {
        let now = 1_700_000_000_000;
        assert_eq!(next_run("every 30m", now).unwrap(), now + 30 * 60 * 1000);
        assert_eq!(next_run("every 2h", now).unwrap(), now + 2 * 3600 * 1000);
        assert!(next_run("every 1m", now).is_err(), "too often");
        assert!(next_run("every x", now).is_err());
        let next = next_run("0 9 * * *", now).unwrap();
        assert!(next > now && next <= now + 86_400_000);
        assert!(next_run("not a schedule", now).is_err());
    }
}
