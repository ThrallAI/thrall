//! `thrall sessions`: your sessions, most recent first.

use std::process::ExitCode;

use thrall_core::Paths;
use thrall_core::constants::{LOCAL_USER_ID, LOCAL_USER_NAME};
use thrall_core::ids::now_ms;
use thrall_store::{SqliteStore, Store};

/// List sessions.
pub async fn list(paths: &Paths, limit: u32) -> anyhow::Result<ExitCode> {
    let store = SqliteStore::open(&paths.db_file).await?;
    store
        .ensure_user(LOCAL_USER_ID, LOCAL_USER_NAME, now_ms())
        .await?;
    let sessions = store.list_sessions(LOCAL_USER_ID, limit).await?;
    if sessions.is_empty() {
        println!("No sessions yet. Start one with: thrall run \"hello\"");
        return Ok(ExitCode::SUCCESS);
    }
    let now = now_ms();
    for s in sessions {
        let title = if s.title.is_empty() {
            "(untitled)"
        } else {
            &s.title
        };
        println!("{}  {:>10}  {title}", s.id, ago(now - s.updated_at));
    }
    println!("\nContinue one with: thrall run -s <id> \"…\"  (or -c for the most recent)");
    Ok(ExitCode::SUCCESS)
}

/// A rough "how long ago" for a gap in milliseconds.
fn ago(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    match secs {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_naturally() {
        assert_eq!(ago(5_000), "just now");
        assert_eq!(ago(5 * 60_000), "5 min ago");
        assert_eq!(ago(3 * 3_600_000), "3 h ago");
        assert_eq!(ago(2 * 86_400_000), "2 d ago");
        assert_eq!(ago(-10), "just now");
    }
}
