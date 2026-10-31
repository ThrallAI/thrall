# Thrall storage — schema v1

*Phase 0 · draft 1 · 2026-10-13*

Everything Thrall remembers lives in one database: **SQLite** on your PC, **Postgres** on a server.
Both are reached through one `Store` trait in `thrall-store`; the rest of the engine never knows which
one it's talking to. Built on `sqlx`, with migrations kept in the repo.

## 1. Rules

- **One logical schema, two dialects.** Migrations live in `crates/thrall-store/migrations/sqlite/`
  and `…/postgres/`, numbered the same. A test checks that both define the same tables and columns.
- **Portable column types.** Ids are `TEXT`. Times are `INTEGER` Unix milliseconds. Money is
  `INTEGER` micro-dollars (no floats). Structured data is JSON (`TEXT` in SQLite, `JSONB` in Postgres).
  Booleans are `INTEGER 0/1` in SQLite, `BOOLEAN` in Postgres.
- **Ids are UUIDv7** with a short type prefix (`s_`, `t_`, `m_` …). They sort by creation time, which
  makes "newest first" cheap.
- **Every row has an owner.** Every table except `users` carries `user_id`, and every query filters on
  it, even locally where there is one user. That way server mode (many users) needs no schema change
  and one user can never read another's data.
- **Never store secrets.** API keys live in the OS keychain locally. On servers they're in
  `secrets`, encrypted per user (§4). Nothing else holds a key, and logs never do.
- **Append-only where it matters.** `events` and `audit_log` are never updated or deleted by the
  engine. Postgres enforces it with a trigger; SQLite with a trigger that raises an error.
- **WAL mode** for SQLite, so the engine can write while clients read history.

## 2. Where it lives

| Mode | Location |
|---|---|
| Local (Windows) | `%APPDATA%\Thrall\thrall.db` |
| Local (macOS / Linux) | the OS data folder (`directories` crate), `thrall.db` |
| Override | `THRALL_HOME` environment variable |
| Server | Postgres connection string from the server config |

Uploaded files live next to the database in `files/`, named by file id. The database holds only their
metadata.

## 3. Tables

### Identity

| Table | Columns | Notes |
|---|---|---|
| `devices` | `id`, `user_id`, `name`, `token_hash` (SHA-256 of the device token, unique), `created_at`, `last_seen_at`, `removed_at` | Browsers signed in with a sign-in link (M2c). Signing out sets `removed_at`; the row stays |
| `users` | `id`, `name`, `created_at`, `role`, `plan`, `removed_at`, `own_key`, `model`, `monthly_limit_micros` | Locally: one row, made on first start; the person running the engine is `owner`, everyone added later a `member`. `plan` is `free` (a hook for paid plans). `removed_at` set: the owner removed them; they can't connect, their things are kept. `own_key` 1: their turns use their own AI key (keychain `openrouter-<id>`); `model`: their own; `monthly_limit_micros`: their monthly limit in millionths of a dollar (M2e/M2f) |

### Conversations

| Table | Columns | Notes |
|---|---|---|
| `sessions` | `id`, `user_id`, `title`, `workspace`, `mode`, `state`, `last_seq`, `created_at`, `updated_at`, `archived_at?` | `mode`: ask · auto_edits · plan · full_auto |
| `events` | `session_id`, `seq`, `user_id`, `turn_id?`, `type`, `payload` (JSON), `at` | **Primary key (`session_id`, `seq`).** Every protocol event, exactly as sent. This is what `session.open {after_seq}` replays |
| `turns` | `id`, `session_id`, `user_id`, `status`, `started_at`, `ended_at?`, `input_tokens`, `output_tokens`, `cost_micros` | One row per user message |
| `checkpoints` | `session_id`, `turn_id`, `user_id`, `path`, `existed`, `content?` (bytes), `too_big`, `after_hash?`, `at` | A file as it was before a turn first changed it (one per turn and path), so the person can rewind. `after_hash`: SHA-256 of what the turn last left there, to spot later changes. Copies over 10 MB aren't kept. Deleted with the session |
| `messages` | `id`, `session_id`, `user_id`, `turn_id?`, `role`, `content` (JSON), `tokens`, `created_at`, `compacted` | What the **model** sees: user, assistant, tool calls and results. Separate from `events`, which is what **people** see |
| `steps` | `id`, `turn_id`, `user_id`, `tool`, `title`, `args` (JSON), `status`, `summary`, `started_at`, `ended_at?` | One row per tool call |
| `tasks` | `id`, `session_id`, `user_id`, `text`, `status`, `position`, `updated_at` | The visible task list |

Why `events` and `messages` are separate: the model's context gets compacted and rewritten, but what
the user saw must replay exactly. Mixing them is how history gets lost.

### Permissions and safety

| Table | Columns | Notes |
|---|---|---|
| `grants` | `id`, `user_id`, `session_id?`, `tool`, `scope` (JSON), `created_at`, `expires_at?` | "Always" answers. `session_id` empty = for all sessions. Never written for always-ask actions |
| `audit_log` | `id`, `user_id`, `at`, `gate`, `action`, `decision`, `detail` (JSON) | Append-only. Every gate decision. `detail` is checked for secrets before writing |
| `approved_folders` | `id`, `user_id`, `path`, `access` (`read` · `write`), `created_at` | Workspace boundaries; also used by the desktop app's companion mode |

### Memory

| Table | Columns | Notes |
|---|---|---|
| `episodes` | `id`, `user_id`, `session_id?`, `summary`, `content`, `created_at` | Searchable conversation history (the old `memory/episodes/` JSONL) |
| `knowledge` | `id`, `user_id`, `topic`, `content`, `source` (JSON), `confidence`, `created_at`, `updated_at`, `superseded_by?` | Long-term facts (the old `memory/knowledge/` JSONL). Facts are superseded, not overwritten |
| `embeddings` | `owner_kind` (`episode` · `knowledge`), `owner_id`, `user_id`, `model`, `vector` | Semantic search. SQLite: f32 BLOB compared in Rust (sqlite-vec would need `unsafe` to load; fine at personal scale). Postgres: `pgvector` column |
| `memory_fts` | | SQLite FTS5 / Postgres `tsvector` over `episodes` and `knowledge`, for keyword search beside semantic search |

### Work and money

| Table | Columns | Notes |
|---|---|---|
| `jobs` | `id`, `user_id`, `session_id?`, `title`, `schedule` (cron), `prompt`, `enabled`, `last_run_at?`, `next_run_at` | Scheduled jobs |
| `job_runs` | `id`, `job_id`, `user_id`, `started_at`, `ended_at?`, `status`, `summary` | |
| `agents` | `id`, `user_id`, `session_id`, `title`, `status`, `state` (JSON), `started_at`, `ended_at?` | Sub-agents and background tasks; `state` keeps progress between spawns |
| `usage` | `id`, `user_id`, `session_id?`, `turn_id?`, `provider`, `model`, `input_tokens`, `output_tokens`, `cost_micros`, `at` | One row per model call. `/cost` and spending limits read from here, so cost tracking works across every client (fixes the Python gap) |
| `files` | `id`, `user_id`, `session_id?`, `name`, `mime`, `bytes`, `sha256`, `created_at` | Upload metadata; the bytes are in `files/` |

### Settings

| Table | Columns | Notes |
|---|---|---|
| `settings` | `user_id`, `key`, `value` (JSON), `updated_at` | Per-user preferences (model, theme, …). Engine-wide config stays in the config file |
| `secrets` | `user_id`, `name`, `ciphertext`, `nonce`, `key_version`, `preview`, `updated_at` | **Server mode only.** Locally this table stays empty and the OS keychain is used |

## 4. Secrets on servers

Envelope encryption: each user has a data key, itself encrypted by a master key the database never
sees (an environment secret or a key service). A key is decrypted only for the provider call that
needs it, then dropped. `preview` (`sk-…a1b2`) is all a client is ever shown.

## 5. Indexes (beyond primary keys)

- `sessions (user_id, updated_at DESC)` — the session list
- `events (session_id, seq)` — the primary key; replay is a range scan
- `messages (session_id, created_at)`
- `steps (turn_id)`, `tasks (session_id, position)`
- `usage (user_id, at)` — cost by day
- `jobs (enabled, next_run_at)` — the scheduler's next job
- `audit_log (user_id, at)`

## 6. Retention

- `events` and `messages` are kept until the user deletes the session (then removed together).
- `audit_log` is kept for 90 days by default (a constant; configurable). Pruning is the only delete it
  allows, done by a maintenance task, never by the agent.
- Deleting a session never deletes `knowledge` it produced. Memory is managed separately (`/memory`).

## 7. Importing from Python Thrall (phase 5)

`thrall import --from ../Thrall` reads the Python data and writes it in:

| Python | Rust |
|---|---|
| `memory/episodes/*.jsonl` | `episodes` (+ embeddings rebuilt) |
| `memory/knowledge/*.jsonl` | `knowledge` |
| scheduled jobs config | `jobs` |
| `config/config.toml` (model, provider) | config file + `settings` |
| API keys in `.env` | OS keychain, after asking — never copied into the database |

The importer is safe to run twice (it skips what's already there) and never changes the Python files.

## 8. Testing

- Every migration runs against both SQLite and Postgres in CI (Postgres from phase 10; SQLite from
  phase 1).
- Store tests run the same suite against both backends through the `Store` trait.
- The "same tables in both dialects" test from §1.
