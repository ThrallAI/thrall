//! SQLite storage for local mode.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow};
use sqlx::{Row, SqlitePool};

use crate::constants::{
    DEFAULT_SESSION_MODE, FORGOTTEN, MAX_EVENTS_PER_PAGE, MAX_SESSIONS_PER_PAGE, MICROS_PER_DOLLAR,
    SQLITE_BUSY_TIMEOUT_MS, SQLITE_MAX_CONNECTIONS,
};
use crate::error::StoreError;
use crate::models::{
    AgentRow, AuditEntry, Checkpoint, Device, Episode, Job, JobRun, Knowledge, MemoryKind,
    NewEvent, NewMessage, NewSession, NewStep, Person, PersonTerms, Session, SessionGroup,
    StoredEvent, StoredMessage, StoredVector, TaskRow, TextHit, TurnEnd, UsageRecord, UsageTotals,
};
use crate::store::{Store, StoreFuture};

/// Storage in one SQLite file (or in memory, for tests).
#[derive(Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    /// Open (creating if needed) the database at `path` and bring it up to date.
    pub async fn open(path: &Path) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_millis(SQLITE_BUSY_TIMEOUT_MS));
        let pool = SqlitePoolOptions::new()
            .max_connections(SQLITE_MAX_CONNECTIONS)
            .connect_with(options)
            .await?;
        Self::migrated(pool).await
    }

    /// A fresh in-memory database, for tests.
    pub async fn open_in_memory() -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
        // One connection: every in-memory connection would otherwise be its own empty database.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        Self::migrated(pool).await
    }

    async fn migrated(pool: SqlitePool) -> Result<Self, StoreError> {
        sqlx::migrate!("./migrations/sqlite").run(&pool).await?;
        Ok(Self { pool })
    }
}

fn to_i64(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Corrupt(format!("{value} is too large to store")))
}

fn to_u64(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| StoreError::Corrupt(format!("negative counter {value}")))
}

fn dollars_to_micros(cost: Option<f64>) -> Option<i64> {
    // Saturating float-to-int cast; costs are tiny, this never clips in practice.
    cost.map(|usd| (usd * MICROS_PER_DOLLAR).round() as i64)
}

/// The columns `session_from_row` reads, as a literal so queries stay compile-time strings.
macro_rules! session_columns {
    () => {
        "id, user_id, title, workspace, mode, state, last_seq, created_at, updated_at, group_id, pinned_at, home_day"
    };
}

fn person_from_row(row: &SqliteRow) -> Result<Person, StoreError> {
    Ok(Person {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        role: row.try_get("role")?,
        plan: row.try_get("plan")?,
        created_at: row.try_get("created_at")?,
        removed_at: row.try_get("removed_at")?,
        own_key: row.try_get("own_key")?,
        model: row.try_get("model")?,
        monthly_limit_micros: row.try_get("monthly_limit_micros")?,
    })
}

fn device_from_row(row: &SqliteRow) -> Result<Device, StoreError> {
    Ok(Device {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        name: row.try_get("name")?,
        created_at: row.try_get("created_at")?,
        last_seen_at: row.try_get("last_seen_at")?,
    })
}

fn session_from_row(row: &SqliteRow) -> Result<Session, StoreError> {
    Ok(Session {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        title: row.try_get("title")?,
        workspace: row.try_get("workspace")?,
        mode: row.try_get("mode")?,
        state: row.try_get("state")?,
        last_seq: to_u64(row.try_get("last_seq")?)?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        group_id: row.try_get("group_id")?,
        pinned_at: row.try_get("pinned_at")?,
        home_day: row.try_get("home_day")?,
    })
}

/// A user's search text as an FTS5 query: each word quoted (so nothing they type is syntax), any
/// word may match, best matches first. `None` when there are no words.
fn fts_query(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 1)
        .map(|w| format!("\"{}\"", w.to_lowercase()))
        .collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

fn job_from_row(row: &SqliteRow) -> Result<Job, StoreError> {
    Ok(Job {
        id: row.try_get("id")?,
        session_id: row.try_get("session_id")?,
        title: row.try_get("title")?,
        schedule: row.try_get("schedule")?,
        prompt: row.try_get("prompt")?,
        enabled: row.try_get("enabled")?,
        last_run_at: row.try_get("last_run_at")?,
        next_run_at: row.try_get("next_run_at")?,
    })
}

impl Store for SqliteStore {
    fn ensure_user<'a>(&'a self, user_id: &'a str, name: &'a str, at: i64) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query("INSERT OR IGNORE INTO users (id, name, created_at) VALUES (?, ?, ?)")
                .bind(user_id)
                .bind(name)
                .bind(at)
                .execute(&self.pool)
                .await?;
            Ok(())
        })
    }

    fn get_person<'a>(&'a self, user_id: &'a str) -> StoreFuture<'a, Option<Person>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, name, role, plan, created_at, removed_at, own_key, model, \n                 monthly_limit_micros FROM users WHERE id = ?",
            )
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
            row.as_ref().map(person_from_row).transpose()
        })
    }

    fn list_people(&self) -> StoreFuture<'_, Vec<Person>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, name, role, plan, created_at, removed_at, own_key, model, \n                 monthly_limit_micros FROM users \
                 WHERE removed_at IS NULL ORDER BY created_at, id",
            )
            .fetch_all(&self.pool)
            .await?;
            rows.iter().map(person_from_row).collect()
        })
    }

    fn set_role<'a>(&'a self, user_id: &'a str, role: &'a str) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query("UPDATE users SET role = ? WHERE id = ?")
                .bind(role)
                .bind(user_id)
                .execute(&self.pool)
                .await?;
            Ok(())
        })
    }

    fn rename_person<'a>(&'a self, user_id: &'a str, name: &'a str) -> StoreFuture<'a, bool> {
        Box::pin(async move {
            let done = sqlx::query("UPDATE users SET name = ? WHERE id = ? AND removed_at IS NULL")
                .bind(name)
                .bind(user_id)
                .execute(&self.pool)
                .await?;
            Ok(done.rows_affected() > 0)
        })
    }

    fn set_person_terms<'a>(
        &'a self,
        user_id: &'a str,
        terms: &'a PersonTerms,
    ) -> StoreFuture<'a, bool> {
        Box::pin(async move {
            let done = sqlx::query(
                "UPDATE users SET own_key = ?, model = ?, monthly_limit_micros = ?                  WHERE id = ? AND removed_at IS NULL",
            )
            .bind(terms.own_key)
            .bind(&terms.model)
            .bind(terms.monthly_limit_micros)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
            Ok(done.rows_affected() > 0)
        })
    }

    fn remove_person<'a>(&'a self, user_id: &'a str, at: i64) -> StoreFuture<'a, bool> {
        Box::pin(async move {
            let done =
                sqlx::query("UPDATE users SET removed_at = ? WHERE id = ? AND removed_at IS NULL")
                    .bind(at)
                    .bind(user_id)
                    .execute(&self.pool)
                    .await?;
            Ok(done.rows_affected() > 0)
        })
    }

    fn add_device<'a>(&'a self, device: &'a Device, token_hash: &'a str) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO devices (id, user_id, name, token_hash, created_at)                  VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&device.id)
            .bind(&device.user_id)
            .bind(&device.name)
            .bind(token_hash)
            .bind(device.created_at)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn use_device<'a>(&'a self, token_hash: &'a str, at: i64) -> StoreFuture<'a, Option<Device>> {
        Box::pin(async move {
            let row = sqlx::query(
                "UPDATE devices SET last_seen_at = ?                  WHERE token_hash = ? AND removed_at IS NULL                  RETURNING id, user_id, name, created_at, last_seen_at",
            )
            .bind(at)
            .bind(token_hash)
            .fetch_optional(&self.pool)
            .await?;
            row.as_ref().map(device_from_row).transpose()
        })
    }

    fn list_devices<'a>(&'a self, user_id: &'a str) -> StoreFuture<'a, Vec<Device>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, user_id, name, created_at, last_seen_at FROM devices                  WHERE user_id = ? AND removed_at IS NULL ORDER BY created_at DESC, id DESC",
            )
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
            rows.iter().map(device_from_row).collect()
        })
    }

    fn device_signed_in<'a>(&'a self, device_id: &'a str) -> StoreFuture<'a, bool> {
        Box::pin(async move {
            let row = sqlx::query("SELECT 1 FROM devices WHERE id = ? AND removed_at IS NULL")
                .bind(device_id)
                .fetch_optional(&self.pool)
                .await?;
            Ok(row.is_some())
        })
    }

    fn remove_device<'a>(
        &'a self,
        user_id: &'a str,
        device_id: &'a str,
        at: i64,
    ) -> StoreFuture<'a, bool> {
        Box::pin(async move {
            let done = sqlx::query(
                "UPDATE devices SET removed_at = ?                  WHERE id = ? AND user_id = ? AND removed_at IS NULL",
            )
            .bind(at)
            .bind(device_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
            Ok(done.rows_affected() > 0)
        })
    }

    fn remove_devices_of<'a>(&'a self, user_id: &'a str, at: i64) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE devices SET removed_at = ? WHERE user_id = ? AND removed_at IS NULL",
            )
            .bind(at)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn create_session(&self, new: NewSession) -> StoreFuture<'_, Session> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO sessions (id, user_id, workspace, mode, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&new.id)
            .bind(&new.user_id)
            .bind(&new.workspace)
            .bind(DEFAULT_SESSION_MODE)
            .bind(new.at)
            .bind(new.at)
            .execute(&self.pool)
            .await?;
            self.get_session(&new.user_id, &new.id)
                .await?
                .ok_or(StoreError::NotFound("session"))
        })
    }

    fn get_session<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
    ) -> StoreFuture<'a, Option<Session>> {
        Box::pin(async move {
            let row = sqlx::query(concat!(
                "SELECT ",
                session_columns!(),
                " FROM sessions WHERE id = ? AND user_id = ?"
            ))
            .bind(session_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
            row.as_ref().map(session_from_row).transpose()
        })
    }

    fn list_sessions<'a>(&'a self, user_id: &'a str, limit: u32) -> StoreFuture<'a, Vec<Session>> {
        Box::pin(async move {
            // Pinned sessions first (most recently pinned on top), then the rest by activity.
            let rows = sqlx::query(concat!(
                "SELECT ",
                session_columns!(),
                " FROM sessions WHERE user_id = ? AND archived_at IS NULL AND home_day IS NULL \
                 ORDER BY pinned_at IS NULL, pinned_at DESC, updated_at DESC, id DESC LIMIT ?"
            ))
            .bind(user_id)
            .bind(limit.min(MAX_SESSIONS_PER_PAGE))
            .fetch_all(&self.pool)
            .await?;
            rows.iter().map(session_from_row).collect()
        })
    }

    fn set_session_title<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        title: &'a str,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let done = sqlx::query("UPDATE sessions SET title = ? WHERE id = ? AND user_id = ?")
                .bind(title)
                .bind(session_id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;
            if done.rows_affected() == 0 {
                return Err(StoreError::NotFound("session"));
            }
            Ok(())
        })
    }

    fn set_session_workspace<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        workspace: &'a str,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let done =
                sqlx::query("UPDATE sessions SET workspace = ? WHERE id = ? AND user_id = ?")
                    .bind(workspace)
                    .bind(session_id)
                    .bind(user_id)
                    .execute(&self.pool)
                    .await?;
            if done.rows_affected() == 0 {
                return Err(StoreError::NotFound("session"));
            }
            Ok(())
        })
    }

    fn set_session_archived<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        archived_at: Option<i64>,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let done =
                sqlx::query("UPDATE sessions SET archived_at = ? WHERE id = ? AND user_id = ?")
                    .bind(archived_at)
                    .bind(session_id)
                    .bind(user_id)
                    .execute(&self.pool)
                    .await?;
            if done.rows_affected() == 0 {
                return Err(StoreError::NotFound("session"));
            }
            Ok(())
        })
    }

    fn list_archived_sessions<'a>(
        &'a self,
        user_id: &'a str,
        limit: u32,
    ) -> StoreFuture<'a, Vec<Session>> {
        Box::pin(async move {
            let rows = sqlx::query(concat!(
                "SELECT ",
                session_columns!(),
                " FROM sessions WHERE user_id = ? AND archived_at IS NOT NULL \
                 AND home_day IS NULL ORDER BY archived_at DESC, id DESC LIMIT ?"
            ))
            .bind(user_id)
            .bind(limit.min(MAX_SESSIONS_PER_PAGE))
            .fetch_all(&self.pool)
            .await?;
            rows.iter().map(session_from_row).collect()
        })
    }

    fn set_session_pinned<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        pinned_at: Option<i64>,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let done =
                sqlx::query("UPDATE sessions SET pinned_at = ? WHERE id = ? AND user_id = ?")
                    .bind(pinned_at)
                    .bind(session_id)
                    .bind(user_id)
                    .execute(&self.pool)
                    .await?;
            if done.rows_affected() == 0 {
                return Err(StoreError::NotFound("session"));
            }
            Ok(())
        })
    }

    fn set_session_group<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        group_id: Option<&'a str>,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            if let Some(group) = group_id {
                let mine = sqlx::query("SELECT 1 FROM session_groups WHERE id = ? AND user_id = ?")
                    .bind(group)
                    .bind(user_id)
                    .fetch_optional(&self.pool)
                    .await?;
                if mine.is_none() {
                    return Err(StoreError::NotFound("group"));
                }
            }
            let done = sqlx::query("UPDATE sessions SET group_id = ? WHERE id = ? AND user_id = ?")
                .bind(group_id)
                .bind(session_id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;
            if done.rows_affected() == 0 {
                return Err(StoreError::NotFound("session"));
            }
            Ok(())
        })
    }

    fn list_groups<'a>(&'a self, user_id: &'a str) -> StoreFuture<'a, Vec<SessionGroup>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, name, position FROM session_groups WHERE user_id = ? \
                 ORDER BY position, created_at",
            )
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
            rows.iter()
                .map(|row| {
                    Ok(SessionGroup {
                        id: row.try_get("id")?,
                        name: row.try_get("name")?,
                        position: row.try_get("position")?,
                    })
                })
                .collect()
        })
    }

    fn create_group<'a>(
        &'a self,
        user_id: &'a str,
        group_id: &'a str,
        name: &'a str,
        at: i64,
    ) -> StoreFuture<'a, SessionGroup> {
        Box::pin(async move {
            let row = sqlx::query(
                "INSERT INTO session_groups (id, user_id, name, position, created_at) \
                 VALUES (?, ?, ?, (SELECT COALESCE(MAX(position), -1) + 1 FROM session_groups \
                 WHERE user_id = ?), ?) RETURNING position",
            )
            .bind(group_id)
            .bind(user_id)
            .bind(name)
            .bind(user_id)
            .bind(at)
            .fetch_one(&self.pool)
            .await?;
            Ok(SessionGroup {
                id: group_id.to_owned(),
                name: name.to_owned(),
                position: row.try_get("position")?,
            })
        })
    }

    fn rename_group<'a>(
        &'a self,
        user_id: &'a str,
        group_id: &'a str,
        name: &'a str,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let done =
                sqlx::query("UPDATE session_groups SET name = ? WHERE id = ? AND user_id = ?")
                    .bind(name)
                    .bind(group_id)
                    .bind(user_id)
                    .execute(&self.pool)
                    .await?;
            if done.rows_affected() == 0 {
                return Err(StoreError::NotFound("group"));
            }
            Ok(())
        })
    }

    fn delete_group<'a>(&'a self, user_id: &'a str, group_id: &'a str) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await?;
            sqlx::query("UPDATE sessions SET group_id = NULL WHERE group_id = ? AND user_id = ?")
                .bind(group_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            let done = sqlx::query("DELETE FROM session_groups WHERE id = ? AND user_id = ?")
                .bind(group_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            if done.rows_affected() == 0 {
                return Err(StoreError::NotFound("group"));
            }
            tx.commit().await?;
            Ok(())
        })
    }

    fn reorder_groups<'a>(
        &'a self,
        user_id: &'a str,
        group_ids: &'a [String],
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await?;
            for (position, id) in group_ids.iter().enumerate() {
                sqlx::query("UPDATE session_groups SET position = ? WHERE id = ? AND user_id = ?")
                    .bind(i64::try_from(position).unwrap_or(i64::MAX))
                    .bind(id)
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
            Ok(())
        })
    }

    fn set_session_home<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        day: &'a str,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let done = sqlx::query("UPDATE sessions SET home_day = ? WHERE id = ? AND user_id = ?")
                .bind(day)
                .bind(session_id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;
            if done.rows_affected() == 0 {
                return Err(StoreError::NotFound("session"));
            }
            Ok(())
        })
    }

    fn home_sessions<'a>(&'a self, user_id: &'a str) -> StoreFuture<'a, Vec<Session>> {
        Box::pin(async move {
            let rows = sqlx::query(concat!(
                "SELECT ",
                session_columns!(),
                " FROM sessions WHERE user_id = ? AND home_day IS NOT NULL \
                 ORDER BY created_at DESC, id DESC"
            ))
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
            rows.iter().map(session_from_row).collect()
        })
    }

    fn delete_session<'a>(&'a self, user_id: &'a str, session_id: &'a str) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let done = sqlx::query("DELETE FROM sessions WHERE id = ? AND user_id = ?")
                .bind(session_id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;
            if done.rows_affected() == 0 {
                return Err(StoreError::NotFound("session"));
            }
            Ok(())
        })
    }

    fn append_event(&self, event: NewEvent) -> StoreFuture<'_, u64> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await?;
            let row = sqlx::query(
                "UPDATE sessions SET last_seq = last_seq + 1, updated_at = ? \
                 WHERE id = ? AND user_id = ? RETURNING last_seq",
            )
            .bind(event.at)
            .bind(&event.session_id)
            .bind(&event.user_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(StoreError::NotFound("session"))?;
            let seq: i64 = row.try_get("last_seq")?;
            sqlx::query(
                "INSERT INTO events (session_id, seq, user_id, turn_id, type, payload, at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&event.session_id)
            .bind(seq)
            .bind(&event.user_id)
            .bind(&event.turn_id)
            .bind(&event.kind)
            .bind(&event.payload)
            .bind(event.at)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            to_u64(seq)
        })
    }

    fn events_after<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        after_seq: u64,
        limit: u32,
    ) -> StoreFuture<'a, Vec<StoredEvent>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT session_id, seq, turn_id, type, payload, at FROM events \
                 WHERE session_id = ? AND user_id = ? AND seq > ? ORDER BY seq LIMIT ?",
            )
            .bind(session_id)
            .bind(user_id)
            .bind(to_i64(after_seq)?)
            .bind(limit.min(MAX_EVENTS_PER_PAGE))
            .fetch_all(&self.pool)
            .await?;
            rows.iter()
                .map(|row| {
                    Ok(StoredEvent {
                        session_id: row.try_get("session_id")?,
                        seq: to_u64(row.try_get("seq")?)?,
                        turn_id: row.try_get("turn_id")?,
                        kind: row.try_get("type")?,
                        payload: row.try_get("payload")?,
                        at: row.try_get("at")?,
                    })
                })
                .collect()
        })
    }

    fn start_turn<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        turn_id: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO turns (id, session_id, user_id, status, started_at) \
                 VALUES (?, ?, ?, 'running', ?)",
            )
            .bind(turn_id)
            .bind(session_id)
            .bind(user_id)
            .bind(at)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn finish_turn(&self, end: TurnEnd) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let done = sqlx::query(
                "UPDATE turns SET status = ?, ended_at = ?, input_tokens = ?, output_tokens = ?, \
                 cost_micros = ? WHERE id = ? AND user_id = ?",
            )
            .bind(&end.status)
            .bind(end.at)
            .bind(to_i64(end.input_tokens)?)
            .bind(to_i64(end.output_tokens)?)
            .bind(dollars_to_micros(end.cost_usd))
            .bind(&end.turn_id)
            .bind(&end.user_id)
            .execute(&self.pool)
            .await?;
            if done.rows_affected() == 0 {
                return Err(StoreError::NotFound("turn"));
            }
            Ok(())
        })
    }

    fn add_message(&self, message: NewMessage) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO messages (id, session_id, user_id, turn_id, role, content, tokens, \
                 created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&message.id)
            .bind(&message.session_id)
            .bind(&message.user_id)
            .bind(&message.turn_id)
            .bind(&message.role)
            .bind(&message.content)
            .bind(to_i64(message.tokens)?)
            .bind(message.at)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn last_turn<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
    ) -> StoreFuture<'a, Option<(String, Vec<String>)>> {
        Box::pin(async move {
            let Some(row) = sqlx::query(
                "SELECT turn_id FROM events WHERE session_id = ? AND user_id = ? \
                 AND type = 'turn.started' ORDER BY seq DESC LIMIT 1",
            )
            .bind(session_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?
            else {
                return Ok(None);
            };
            let turn: Option<String> = row.try_get("turn_id")?;
            let Some(turn) = turn else { return Ok(None) };
            let texts = self.turn_messages(user_id, session_id, &turn).await?;
            Ok((!texts.is_empty()).then_some((turn, texts)))
        })
    }

    fn turn_messages<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        turn: &'a str,
    ) -> StoreFuture<'a, Vec<String>> {
        Box::pin(async move {
            // The user's messages: completed messages whose start says role "user".
            let rows = sqlx::query(
                "SELECT done.payload FROM events done JOIN events started \
                 ON started.session_id = done.session_id AND started.turn_id = done.turn_id \
                 AND started.type = 'message.started' \
                 AND json_extract(started.payload, '$.role') = 'user' \
                 AND json_extract(started.payload, '$.message') = json_extract(done.payload, '$.message') \
                 WHERE done.session_id = ? AND done.user_id = ? AND done.turn_id = ? \
                 AND done.type = 'message.completed' ORDER BY done.seq",
            )
            .bind(session_id)
            .bind(user_id)
            .bind(turn)
            .fetch_all(&self.pool)
            .await?;
            let mut texts = Vec::new();
            for row in rows {
                let payload: String = row.try_get("payload")?;
                if let Some(text) = serde_json::from_str::<serde_json::Value>(&payload)
                    .ok()
                    .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_owned))
                {
                    texts.push(text);
                }
            }
            Ok(texts)
        })
    }

    fn turns_from<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        turn_id: &'a str,
    ) -> StoreFuture<'a, Vec<String>> {
        Box::pin(async move {
            // Turns are numbered by when they were added (rowid), which is the order they ran.
            let rows = sqlx::query(
                "SELECT id FROM turns WHERE session_id = ? AND user_id = ? AND rowid >= \
                 (SELECT rowid FROM turns WHERE id = ? AND session_id = ? AND user_id = ?) \
                 ORDER BY rowid",
            )
            .bind(session_id)
            .bind(user_id)
            .bind(turn_id)
            .bind(session_id)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
            rows.iter().map(|row| Ok(row.try_get("id")?)).collect()
        })
    }

    fn save_checkpoint(&self, cp: Checkpoint) -> StoreFuture<'_, bool> {
        Box::pin(async move {
            let done = sqlx::query(
                "INSERT OR IGNORE INTO checkpoints (session_id, turn_id, user_id, path, existed, \
                 content, too_big, after_hash, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&cp.session_id)
            .bind(&cp.turn_id)
            .bind(&cp.user_id)
            .bind(&cp.path)
            .bind(cp.existed)
            .bind(&cp.content)
            .bind(cp.too_big)
            .bind(&cp.after_hash)
            .bind(cp.at)
            .execute(&self.pool)
            .await?;
            Ok(done.rows_affected() > 0)
        })
    }

    fn set_checkpoint_after<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        turn_id: &'a str,
        path: &'a str,
        after_hash: &'a str,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE checkpoints SET after_hash = ? \
                 WHERE session_id = ? AND user_id = ? AND turn_id = ? AND path = ?",
            )
            .bind(after_hash)
            .bind(session_id)
            .bind(user_id)
            .bind(turn_id)
            .bind(path)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn checkpoints_from<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        turn_id: &'a str,
    ) -> StoreFuture<'a, Vec<Checkpoint>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT c.session_id, c.turn_id, c.user_id, c.path, c.existed, c.content, \
                 c.too_big, c.after_hash, c.at FROM checkpoints c \
                 JOIN turns t ON t.id = c.turn_id \
                 WHERE c.session_id = ? AND c.user_id = ? AND t.rowid >= \
                 (SELECT rowid FROM turns WHERE id = ? AND session_id = ? AND user_id = ?) \
                 ORDER BY t.rowid, c.at, c.path",
            )
            .bind(session_id)
            .bind(user_id)
            .bind(turn_id)
            .bind(session_id)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
            rows.iter()
                .map(|row| {
                    Ok(Checkpoint {
                        session_id: row.try_get("session_id")?,
                        turn_id: row.try_get("turn_id")?,
                        user_id: row.try_get("user_id")?,
                        path: row.try_get("path")?,
                        existed: row.try_get("existed")?,
                        content: row.try_get("content")?,
                        too_big: row.try_get("too_big")?,
                        after_hash: row.try_get("after_hash")?,
                        at: row.try_get("at")?,
                    })
                })
                .collect()
        })
    }

    fn retract_turn<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        turn_id: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE messages SET retracted_at = ? \
                 WHERE session_id = ? AND user_id = ? AND turn_id = ? AND retracted_at IS NULL",
            )
            .bind(at)
            .bind(session_id)
            .bind(user_id)
            .bind(turn_id)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn context_messages<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
    ) -> StoreFuture<'a, Vec<StoredMessage>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, session_id, turn_id, role, content, tokens, created_at FROM messages \
                 WHERE session_id = ? AND user_id = ? AND compacted = 0 AND retracted_at IS NULL \
                 ORDER BY created_at, id",
            )
            .bind(session_id)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
            rows.iter()
                .map(|row| {
                    Ok(StoredMessage {
                        id: row.try_get("id")?,
                        session_id: row.try_get("session_id")?,
                        turn_id: row.try_get("turn_id")?,
                        role: row.try_get("role")?,
                        content: row.try_get("content")?,
                        tokens: to_u64(row.try_get("tokens")?)?,
                        created_at: row.try_get("created_at")?,
                    })
                })
                .collect()
        })
    }

    fn record_usage(&self, usage: UsageRecord) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO usage (id, user_id, session_id, turn_id, provider, model, role, \
                 input_tokens, cached_tokens, output_tokens, cost_micros, at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&usage.id)
            .bind(&usage.user_id)
            .bind(&usage.session_id)
            .bind(&usage.turn_id)
            .bind(&usage.provider)
            .bind(&usage.model)
            .bind(&usage.role)
            .bind(to_i64(usage.input_tokens)?)
            .bind(to_i64(usage.cached_tokens)?)
            .bind(to_i64(usage.output_tokens)?)
            .bind(dollars_to_micros(usage.cost_usd))
            .bind(usage.at)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn spent_since<'a>(&'a self, user_id: &'a str, since: i64) -> StoreFuture<'a, f64> {
        Box::pin(async move {
            let micros: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(cost_micros), 0) FROM usage WHERE user_id = ? AND at >= ?",
            )
            .bind(user_id)
            .bind(since)
            .fetch_one(&self.pool)
            .await?;
            Ok(micros as f64 / MICROS_PER_DOLLAR)
        })
    }

    fn usage_totals<'a>(
        &'a self,
        user_id: &'a str,
        session_id: Option<&'a str>,
    ) -> StoreFuture<'a, UsageTotals> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT COALESCE(SUM(input_tokens), 0) AS input, \
                 COALESCE(SUM(output_tokens), 0) AS output, \
                 COALESCE(SUM(cost_micros), 0) AS micros \
                 FROM usage WHERE user_id = ? AND (? IS NULL OR session_id = ?)",
            )
            .bind(user_id)
            .bind(session_id)
            .bind(session_id)
            .fetch_one(&self.pool)
            .await?;
            let micros: i64 = row.try_get("micros")?;
            Ok(UsageTotals {
                input_tokens: to_u64(row.try_get("input")?)?,
                output_tokens: to_u64(row.try_get("output")?)?,
                cost_usd: micros as f64 / MICROS_PER_DOLLAR,
            })
        })
    }

    fn turn_usage<'a>(
        &'a self,
        user_id: &'a str,
        turn_id: &'a str,
    ) -> StoreFuture<'a, Vec<UsageRecord>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, user_id, session_id, turn_id, provider, model, role, input_tokens, \
                 cached_tokens, output_tokens, cost_micros, at FROM usage \
                 WHERE user_id = ? AND turn_id = ? ORDER BY at, rowid",
            )
            .bind(user_id)
            .bind(turn_id)
            .fetch_all(&self.pool)
            .await?;
            rows.iter()
                .map(|row| {
                    let micros: Option<i64> = row.try_get("cost_micros")?;
                    Ok(UsageRecord {
                        id: row.try_get("id")?,
                        user_id: row.try_get("user_id")?,
                        session_id: row.try_get("session_id")?,
                        turn_id: row.try_get("turn_id")?,
                        provider: row.try_get("provider")?,
                        model: row.try_get("model")?,
                        role: row.try_get("role")?,
                        input_tokens: to_u64(row.try_get("input_tokens")?)?,
                        cached_tokens: to_u64(row.try_get("cached_tokens")?)?,
                        output_tokens: to_u64(row.try_get("output_tokens")?)?,
                        cost_usd: micros.map(|m| m as f64 / MICROS_PER_DOLLAR),
                        at: row.try_get("at")?,
                    })
                })
                .collect()
        })
    }

    fn start_step(&self, step: NewStep) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO steps (id, turn_id, user_id, tool, title, args, status, started_at) \
                 VALUES (?, ?, ?, ?, ?, ?, 'running', ?)",
            )
            .bind(&step.id)
            .bind(&step.turn_id)
            .bind(&step.user_id)
            .bind(&step.tool)
            .bind(&step.title)
            .bind(&step.args)
            .bind(step.at)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn finish_step<'a>(
        &'a self,
        user_id: &'a str,
        step_id: &'a str,
        status: &'a str,
        summary: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let done = sqlx::query(
                "UPDATE steps SET status = ?, summary = ?, ended_at = ? WHERE id = ? AND user_id = ?",
            )
            .bind(status)
            .bind(summary)
            .bind(at)
            .bind(step_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
            if done.rows_affected() == 0 {
                return Err(StoreError::NotFound("step"));
            }
            Ok(())
        })
    }

    fn set_tasks<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        tasks: &'a [TaskRow],
        at: i64,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await?;
            sqlx::query("DELETE FROM tasks WHERE session_id = ? AND user_id = ?")
                .bind(session_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            for (position, task) in tasks.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO tasks (id, session_id, user_id, text, status, position, \
                     updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&task.id)
                .bind(session_id)
                .bind(user_id)
                .bind(&task.text)
                .bind(&task.status)
                .bind(to_i64(position as u64)?)
                .bind(at)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            Ok(())
        })
    }

    fn tasks<'a>(&'a self, user_id: &'a str, session_id: &'a str) -> StoreFuture<'a, Vec<TaskRow>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, text, status FROM tasks WHERE session_id = ? AND user_id = ? \
                 ORDER BY position",
            )
            .bind(session_id)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
            rows.iter()
                .map(|row| {
                    Ok(TaskRow {
                        id: row.try_get("id")?,
                        text: row.try_get("text")?,
                        status: row.try_get("status")?,
                    })
                })
                .collect()
        })
    }

    fn save_knowledge<'a>(
        &'a self,
        user_id: &'a str,
        id: &'a str,
        topic: &'a str,
        content: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO knowledge (id, user_id, topic, content, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(user_id)
            .bind(topic)
            .bind(content)
            .bind(at)
            .bind(at)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn set_session_mode<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        mode: &'a str,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let done = sqlx::query("UPDATE sessions SET mode = ? WHERE id = ? AND user_id = ?")
                .bind(mode)
                .bind(session_id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;
            if done.rows_affected() == 0 {
                return Err(StoreError::NotFound("session"));
            }
            Ok(())
        })
    }

    fn add_grant<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        id: &'a str,
        tool: &'a str,
        scope: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO grants (id, user_id, session_id, tool, scope, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(user_id)
            .bind(session_id)
            .bind(tool)
            .bind(scope)
            .bind(at)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn has_grant<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        scope: &'a str,
    ) -> StoreFuture<'a, bool> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT 1 FROM grants WHERE user_id = ? AND (session_id = ? OR session_id IS NULL) \
                 AND scope = ? LIMIT 1",
            )
            .bind(user_id)
            .bind(session_id)
            .bind(scope)
            .fetch_optional(&self.pool)
            .await?;
            Ok(row.is_some())
        })
    }

    fn add_episode(&self, user_id: &str, episode: Episode) -> StoreFuture<'_, ()> {
        let user_id = user_id.to_owned();
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO episodes (id, user_id, session_id, summary, content, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&episode.id)
            .bind(&user_id)
            .bind(&episode.session_id)
            .bind(&episode.summary)
            .bind(&episode.content)
            .bind(episode.created_at)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn get_episodes<'a>(
        &'a self,
        user_id: &'a str,
        ids: &'a [String],
    ) -> StoreFuture<'a, Vec<Episode>> {
        Box::pin(async move {
            let mut out = Vec::new();
            for id in ids {
                let row = sqlx::query(
                    "SELECT id, session_id, summary, content, created_at FROM episodes \
                     WHERE id = ? AND user_id = ?",
                )
                .bind(id)
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?;
                if let Some(r) = row {
                    out.push(Episode {
                        id: r.try_get("id")?,
                        session_id: r.try_get("session_id")?,
                        summary: r.try_get("summary")?,
                        content: r.try_get("content")?,
                        created_at: r.try_get("created_at")?,
                    });
                }
            }
            Ok(out)
        })
    }

    fn get_knowledge<'a>(
        &'a self,
        user_id: &'a str,
        ids: &'a [String],
    ) -> StoreFuture<'a, Vec<Knowledge>> {
        Box::pin(async move {
            let mut out = Vec::new();
            for id in ids {
                let row = sqlx::query(
                    "SELECT id, topic, content, updated_at FROM knowledge \
                     WHERE id = ? AND user_id = ? AND superseded_by IS NULL",
                )
                .bind(id)
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?;
                if let Some(r) = row {
                    out.push(Knowledge {
                        id: r.try_get("id")?,
                        topic: r.try_get("topic")?,
                        content: r.try_get("content")?,
                        updated_at: r.try_get("updated_at")?,
                    });
                }
            }
            Ok(out)
        })
    }

    fn knowledge_by_topic<'a>(
        &'a self,
        user_id: &'a str,
        topic: &'a str,
    ) -> StoreFuture<'a, Vec<Knowledge>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, topic, content, updated_at FROM knowledge \
                 WHERE user_id = ? AND superseded_by IS NULL AND lower(topic) = lower(?) \
                 ORDER BY updated_at DESC",
            )
            .bind(user_id)
            .bind(topic.trim())
            .fetch_all(&self.pool)
            .await?;
            rows.iter()
                .map(|r| {
                    Ok(Knowledge {
                        id: r.try_get("id")?,
                        topic: r.try_get("topic")?,
                        content: r.try_get("content")?,
                        updated_at: r.try_get("updated_at")?,
                    })
                })
                .collect()
        })
    }

    fn supersede_knowledge<'a>(
        &'a self,
        user_id: &'a str,
        old_id: &'a str,
        new_id: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE knowledge SET superseded_by = ?, updated_at = ? \
                 WHERE id = ? AND user_id = ? AND superseded_by IS NULL",
            )
            .bind(new_id)
            .bind(at)
            .bind(old_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn forget_knowledge<'a>(
        &'a self,
        user_id: &'a str,
        id: &'a str,
        at: i64,
    ) -> StoreFuture<'a, bool> {
        Box::pin(async move {
            let done = sqlx::query(
                "UPDATE knowledge SET superseded_by = ?, updated_at = ? \
                 WHERE id = ? AND user_id = ? AND superseded_by IS NULL",
            )
            .bind(FORGOTTEN)
            .bind(at)
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
            Ok(done.rows_affected() > 0)
        })
    }

    fn search_memory_text<'a>(
        &'a self,
        user_id: &'a str,
        query: &'a str,
        limit: u32,
    ) -> StoreFuture<'a, Vec<TextHit>> {
        Box::pin(async move {
            let Some(fts) = fts_query(query) else {
                return Ok(Vec::new());
            };
            let rows = sqlx::query(
                "SELECT owner_kind, owner_id, bm25(memory_fts) AS rank FROM memory_fts \
                 WHERE memory_fts MATCH ? AND user_id = ? ORDER BY rank LIMIT ?",
            )
            .bind(&fts)
            .bind(user_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
            let mut hits = Vec::new();
            for r in &rows {
                let kind: String = r.try_get("owner_kind")?;
                if let Some(kind) = MemoryKind::parse(&kind) {
                    hits.push(TextHit {
                        kind,
                        id: r.try_get("owner_id")?,
                        rank: r.try_get("rank")?,
                    });
                }
            }
            Ok(hits)
        })
    }

    fn put_embedding<'a>(
        &'a self,
        user_id: &'a str,
        kind: MemoryKind,
        id: &'a str,
        model: &'a str,
        vector: &'a [f32],
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let bytes: Vec<u8> = vector.iter().flat_map(|f| f.to_le_bytes()).collect();
            sqlx::query(
                "INSERT INTO embeddings (owner_kind, owner_id, user_id, model, vector) \
                 VALUES (?, ?, ?, ?, ?) ON CONFLICT (owner_kind, owner_id) \
                 DO UPDATE SET model = excluded.model, vector = excluded.vector",
            )
            .bind(kind.as_str())
            .bind(id)
            .bind(user_id)
            .bind(model)
            .bind(bytes)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn embeddings<'a>(
        &'a self,
        user_id: &'a str,
        model: &'a str,
    ) -> StoreFuture<'a, Vec<StoredVector>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT owner_kind, owner_id, vector FROM embeddings WHERE user_id = ? AND model = ?",
            )
            .bind(user_id)
            .bind(model)
            .fetch_all(&self.pool)
            .await?;
            let mut out = Vec::with_capacity(rows.len());
            for r in &rows {
                let kind: String = r.try_get("owner_kind")?;
                let bytes: Vec<u8> = r.try_get("vector")?;
                let Some(kind) = MemoryKind::parse(&kind) else {
                    continue;
                };
                let (chunks, _) = bytes.as_chunks::<4>();
                let vector = chunks.iter().map(|c| f32::from_le_bytes(*c)).collect();
                out.push(StoredVector {
                    kind,
                    id: r.try_get("owner_id")?,
                    vector,
                });
            }
            Ok(out)
        })
    }

    fn compact_messages<'a>(
        &'a self,
        user_id: &'a str,
        session_id: &'a str,
        ids: &'a [String],
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let mut tx = self.pool.begin().await?;
            for id in ids {
                sqlx::query(
                    "UPDATE messages SET compacted = 1 WHERE id = ? AND session_id = ? AND user_id = ?",
                )
                .bind(id)
                .bind(session_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
            Ok(())
        })
    }
    fn add_agent(&self, user_id: &str, agent: AgentRow) -> StoreFuture<'_, ()> {
        let user_id = user_id.to_owned();
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO agents (id, user_id, session_id, title, status, state, started_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&agent.id)
            .bind(&user_id)
            .bind(&agent.session_id)
            .bind(&agent.title)
            .bind(&agent.status)
            .bind(&agent.state)
            .bind(agent.started_at)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn finish_agent<'a>(
        &'a self,
        user_id: &'a str,
        agent_id: &'a str,
        status: &'a str,
        state: &'a str,
        at: i64,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE agents SET status = ?, state = ?, ended_at = ? WHERE id = ? AND user_id = ?",
            )
            .bind(status)
            .bind(state)
            .bind(at)
            .bind(agent_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn get_agent<'a>(
        &'a self,
        user_id: &'a str,
        agent_id: &'a str,
    ) -> StoreFuture<'a, Option<AgentRow>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, session_id, title, status, state, started_at FROM agents \
                 WHERE id = ? AND user_id = ?",
            )
            .bind(agent_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
            row.map(|r| {
                Ok(AgentRow {
                    id: r.try_get("id")?,
                    session_id: r.try_get("session_id")?,
                    title: r.try_get("title")?,
                    status: r.try_get("status")?,
                    state: r.try_get("state")?,
                    started_at: r.try_get("started_at")?,
                })
            })
            .transpose()
        })
    }

    fn add_job(&self, user_id: &str, job: Job) -> StoreFuture<'_, ()> {
        let user_id = user_id.to_owned();
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO jobs (id, user_id, session_id, title, schedule, prompt, enabled, \
                 next_run_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&job.id)
            .bind(&user_id)
            .bind(&job.session_id)
            .bind(&job.title)
            .bind(&job.schedule)
            .bind(&job.prompt)
            .bind(job.enabled)
            .bind(job.next_run_at)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn list_jobs<'a>(&'a self, user_id: &'a str) -> StoreFuture<'a, Vec<Job>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, session_id, title, schedule, prompt, enabled, last_run_at, next_run_at \
                 FROM jobs WHERE user_id = ? ORDER BY next_run_at",
            )
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
            rows.iter().map(job_from_row).collect()
        })
    }

    fn due_jobs<'a>(&'a self, user_id: &'a str, now: i64) -> StoreFuture<'a, Vec<Job>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, session_id, title, schedule, prompt, enabled, last_run_at, next_run_at \
                 FROM jobs WHERE user_id = ? AND enabled = 1 AND next_run_at <= ? \
                 ORDER BY next_run_at",
            )
            .bind(user_id)
            .bind(now)
            .fetch_all(&self.pool)
            .await?;
            rows.iter().map(job_from_row).collect()
        })
    }

    fn job_ran<'a>(
        &'a self,
        user_id: &'a str,
        job_id: &'a str,
        ran_at: i64,
        next_run_at: i64,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE jobs SET last_run_at = ?, next_run_at = ? WHERE id = ? AND user_id = ?",
            )
            .bind(ran_at)
            .bind(next_run_at)
            .bind(job_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn delete_job<'a>(&'a self, user_id: &'a str, job_id: &'a str) -> StoreFuture<'a, bool> {
        Box::pin(async move {
            let done = sqlx::query("DELETE FROM jobs WHERE id = ? AND user_id = ?")
                .bind(job_id)
                .bind(user_id)
                .execute(&self.pool)
                .await?;
            Ok(done.rows_affected() > 0)
        })
    }

    fn add_job_run(&self, user_id: &str, run: JobRun) -> StoreFuture<'_, ()> {
        let user_id = user_id.to_owned();
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO job_runs (id, job_id, user_id, started_at, ended_at, status, summary) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&run.id)
            .bind(&run.job_id)
            .bind(&user_id)
            .bind(run.started_at)
            .bind(run.ended_at)
            .bind(&run.status)
            .bind(&run.summary)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }
    fn append_audit(&self, entry: AuditEntry) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO audit_log (id, user_id, at, gate, action, decision, detail) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&entry.id)
            .bind(&entry.user_id)
            .bind(entry.at)
            .bind(&entry.gate)
            .bind(&entry.action)
            .bind(&entry.decision)
            .bind(&entry.detail)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn search_knowledge<'a>(
        &'a self,
        user_id: &'a str,
        query: &'a str,
        limit: u32,
    ) -> StoreFuture<'a, Vec<Knowledge>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, topic, content, updated_at FROM knowledge \
                 WHERE user_id = ? AND superseded_by IS NULL ORDER BY updated_at DESC",
            )
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;
            // Word matching in Rust keeps LIKE wildcards in user text from meaning anything.
            let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
            let mut found = Vec::new();
            for row in &rows {
                let topic: String = row.try_get("topic")?;
                let content: String = row.try_get("content")?;
                let haystack = format!("{topic}\n{content}").to_lowercase();
                if words.iter().all(|w| haystack.contains(w.as_str())) {
                    found.push(Knowledge {
                        id: row.try_get("id")?,
                        topic,
                        content,
                        updated_at: row.try_get("updated_at")?,
                    });
                    if found.len() >= limit as usize {
                        break;
                    }
                }
            }
            Ok(found)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn events_cannot_be_edited_or_deleted_directly() {
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
        store
            .append_event(NewEvent {
                user_id: "u".into(),
                session_id: "s".into(),
                turn_id: None,
                kind: "turn.started".into(),
                payload: "{}".into(),
                at: 2,
            })
            .await
            .unwrap();
        let update = sqlx::query("UPDATE events SET payload = 'x'")
            .execute(&store.pool)
            .await;
        assert!(update.is_err(), "events must not be editable");
        let delete = sqlx::query("DELETE FROM events").execute(&store.pool).await;
        assert!(delete.is_err(), "events must not be deletable on their own");
    }

    #[tokio::test]
    async fn the_audit_log_is_append_only() {
        let store = SqliteStore::open_in_memory().await.unwrap();
        store.ensure_user("u", "U", 1).await.unwrap();
        store
            .append_audit(AuditEntry {
                id: "a".into(),
                user_id: "u".into(),
                at: 1,
                gate: "tool".into(),
                action: "shell".into(),
                decision: "allow".into(),
                detail: "{}".into(),
            })
            .await
            .unwrap();
        assert!(
            sqlx::query("UPDATE audit_log SET decision = 'x'")
                .execute(&store.pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("DELETE FROM audit_log")
                .execute(&store.pool)
                .await
                .is_err()
        );
    }
}
