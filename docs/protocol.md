# Thrall client protocol — v1

*Protocol v1 · changes are listed in [CHANGELOG.md](../CHANGELOG.md)*

Every client (terminal, web app, desktop app, Android, Telegram) talks to the engine with the messages
in this document, and nothing else. Clients display and collect input; all agent logic stays in the
engine. The Rust types live in the `thrall-protocol` crate; the TypeScript (web, desktop) and Kotlin
(Android) types are generated from its JSON Schema, so no client hand-writes them.

## 1. What this fixes from Python Thrall

| Python Thrall today | v1 |
|---|---|
| Reply arrives whole at the end | Replies stream (`message.delta`) |
| Permission prompt is a text notice answered by typing "yes" | Structured `permission.request` with buttons; one answer resolves it on every client |
| Free-form message types, no version | Every frame has `v` and a typed `type`; versions are negotiated |
| A reply can't be matched to its request | Requests carry `id`; replies carry `re` |
| A dropped connection loses what happened | Every session event has a `seq`; clients resume from the last one they saw |
| Telegram and web app keep separate state | One engine process; every client attached to a session gets the same events |
| Errors are free text | Errors have a stable `code` plus a plain-language `message` |

## 2. Transport

- **WebSocket**, text frames, one JSON object per frame, UTF-8. Endpoint: `/v1/ws`.
- **File uploads** use HTTP (`POST /v1/files`, returns a file id), never base64 inside a frame. The body is the file;
  `Authorization: Bearer <token>`; `X-File-Name` carries the URL-encoded name; at most 25 MiB. Reply:
  `{file, name, size}`. `turn.send {files: [id]}` attaches them: text files and documents (PDF, Word,
  PowerPoint, spreadsheets) go in as text; pictures and scanned PDFs are read by the picture model
  (`[provider] vision_model`, with `vision_backup_model`), shown as a step; anything else is described
  with where it's saved. The user message's `message.completed` lists them in `files`.
- **File downloads:** `GET /v1/files/{id}` with the same Bearer token returns an attached file.
  Pictures and PDFs are served inline, everything else as a download, always with
  `X-Content-Type-Options: nosniff`.
- **Local mode:** the engine listens on `127.0.0.1` only. A random access token is written to the data
  folder at first start; local clients read it from there. The engine rejects WebSocket upgrades whose
  `Origin` is not its own web app, so a web page in your browser can't drive Thrall.
- **Phone link:** paired phones connect to a second listener (`127.0.0.1:7419`, `[remote] port`),
  only while `remote.enabled` is on; a tunnel such as Tailscale Funnel forwards the public address
  (`remote.address`) to it. Its one path, `/v1/remote`, takes binary WebSocket messages, all
  encrypted with Noise (see §13a); anything sending `Origin` (a browser page) is refused.
- **Server mode (planned):** TLS only; the token comes from signing in to an account.
- **Frame size:** at most 1 MiB. Larger content goes through `/v1/files`.
- **Heartbeat:** the client sends `ping` every 20 s; the engine answers `pong`. Either side treats
  60 s of silence as a dead connection. (App-level, because browsers can't send WebSocket ping frames.)

All numbers in this document are defaults held in `thrall-protocol`'s constants, not literals in code.

## 3. Frame shape

```json
{ "v": 1, "type": "turn.send", "id": "c-17", "session": "s_01J…", "text": "fix the failing test" }
```

| Field | Who sends it | Meaning |
|---|---|---|
| `v` | both | Protocol major version, always present |
| `type` | both | Message type, `noun.verb`, lowercase |
| `id` | client | Request id, unique per connection, chosen by the client |
| `re` | engine | The request `id` this frame answers (`ack`, `error`, direct results) |
| `session` | both | Session id, when the message belongs to a session |
| `seq` | engine | Position in the session's event log (see §6); only on session events |
| `at` | engine | Time the event happened, Unix milliseconds |

Ids are opaque strings. Clients must not parse them.

**Every client request gets exactly one reply:** either a typed result (named in the tables below),
an `ack {re}` when there is nothing to return, or an `error {re}`.

## 4. Compatibility rules

1. **Must ignore.** Unknown fields are ignored. Unknown *event* types from the engine are ignored
   (a client may show "update Thrall to see this"). This lets the engine add things without breaking
   older clients.
2. **Unknown requests are refused.** An engine that doesn't know a request type replies
   `error {code: "unsupported"}`.
3. **Additions are minor, removals are major.** New optional fields and new types keep `v: 1`.
   Renaming, removing or changing the meaning of anything means `v: 2`.
4. **Negotiation.** The client lists the majors it speaks in `hello`; the engine picks the highest
   both support. The engine supports the current and the previous major.

## 5. Connecting

```
client → hello    { id, protocols: [1], client: { kind: "tui", name: "Thrall Terminal", version: "0.1.0" },
                    token: "…" }
engine → welcome  { protocol: 1, engine: { version: "0.1.0", model, edition: "home" },
                    user: { id, name, role: "owner" | "member" },
                    capabilities: ["streaming", "diffs", "plan_mode", …], limits: { frame_bytes: 1048576 } }
```

- `client.kind`: `tui` · `web` · `desktop` · `android` · `telegram` · `headless`.
- `hello` must be the first frame, within 10 s, or the engine closes the connection.
- A bad token gets `error {code: "unauthorized"}` and the connection closes. The engine never says
  whether the token was close.
- `capabilities` lets one client build work against older or newer engines without guessing.

## 6. Sessions and the event log

A session is one conversation with its own history, working folder, permission mode and task list.
Everything the engine tells clients about a session is an **event** with a `seq` that only goes up.
The engine stores every event, so:

- **Resume after a dropped connection:** `session.open {session, after_seq}` replays every event after
  `after_seq`, then continues live. Nothing is lost.
- **Several clients at once:** every client attached to a session receives every event. Typing in the
  terminal shows up live on your phone.
- **Reopen an old session:** `session.open` without `after_seq` replays the whole visible history.
- **Live-only events** (`message.delta`, `reasoning.delta`, `step.output`) carry no `seq` and are never
  stored. A client that reconnects mid-reply gets the full text in the matching `message.completed`;
  a step's result is in its `step.completed`.

| Request | Result | What it does |
|---|---|---|
| `session.create {workspace?, title?, mode?}` | `session.opened` | New session; `workspace` is a folder path |
| `session.open {session, after_seq?}` | `session.opened`, then replayed events | Attach to a session |
| `session.detach {session}` | `ack` | Stop receiving its events; the session keeps running |
| `session.list {limit?}` | `session.listed {items}` | Newest first, each shaped like `session.opened` |
| `session.rename {session, title}` | `ack` + `session.updated` event | |
| `session.set_mode {session, mode, until?, minutes?}` | `ack` + `session.updated` event (with `mode_limit` for Full auto) | See §9 |
| `session.delete {session}` | `ack` | Clients ask the user first; the engine does not |

`session.opened {session, title, workspace?, mode, state, last_seq, updated, cost_usd, group?, pinned?, home?}`.

Added in phases 6 and 6b:

| Request | Result | What it does |
|---|---|---|
| `session.list {limit?, archived?}` | `session.listed` | `archived: true` lists archived sessions instead |
| `session.archive {session, archived}` | `ack` | Hide a session from the list (kept in full), or bring it back |
| `session.pin {session, pinned}` | `ack` | Pinned sessions list first |
| `session.move {session, group?}` | `ack` | Put a session in one of the person's groups; no `group` takes it out |
| `session.set_folder {session, folder}` | `ack` | Move a session to another existing working folder; attached clients get `session.updated`. Refused (`invalid`) for Home, while a turn runs, or for a folder that isn't there |
| `group.list {}` | `group.listed {items: [{group, name}]}` | The person's groups, in order |
| `group.create {name}` · `group.rename {group, name}` · `group.delete {group}` · `group.reorder {groups}` | `group.listed` | Deleting a group keeps its sessions, ungrouped |
| `session.watch_all {}` | `ack` | Also receive every other session's events (not live text); the web and desktop apps use it for the sidebar, tray and notifications |
| `engine.stop {}` | `ack` | The engine shuts down (the desktop app's Quit, when asked to stop Thrall too) |
| `audio.transcribe {file, session?}` | `audio.transcribed {text}` | An uploaded recording (`/v1/files`) as text: the app's microphone button. Cost goes to `session` |
| `home.get {}` | `home.shown {page, earlier}` | Home, Thrall's built-in conversation: the page to use now and earlier pages (newest first), each shaped like `session.opened` with `home: true`. Open `page` with `session.open`. A fresh page starts per `home.fresh_start` (daily, weekly, never; an unused page is re-dated instead); pages untouched for `home.keep_days` (0 = never) are deleted |
| `home.clear {}` | `home.shown` | Delete every Home page (refused while Thrall works in one); a fresh page starts. Clients ask the user first |

| `heartbeat.get {}` | `heartbeat.shown {checklist, next_run?}` | Check-ins: the checklist (`HEARTBEAT.md` in the data folder) and when the next is due (absent while `heartbeat.enabled` is off) |
| `heartbeat.set {checklist}` | `heartbeat.shown` | Replace the checklist; empty removes it (no check-ins without one) |
| `heartbeat.run {}` | `ack` | Check in now, whatever the hours (`error {invalid}` without a checklist) |
| `startup.get {}` | `startup.shown {available, on}` | Owner only. Whether Thrall starts when the person signs in to Windows: the desktop app, in the tray (the same entry as its tray menu). `available` is false off Windows or without the desktop app beside the engine |
| `startup.set {on}` | `startup.shown` | Owner only. Start at sign-in, or stop (`error {invalid}` where it isn't available) |

`turn.started` carries `scheduled: true` when nobody started the turn (a scheduled job or a check-in), so clients don't announce it as the person's own work finishing. The desktop app's pop-ups follow `[notifications]` in `config.toml` (`needs_you`, `done`, `failed`, `background`, `check_ins`, `quiet_hours` when only "needs you" comes through, `only_when_away`), read when each pop-up is due; clicking one calls the web app's `window.thrallOpen(session)`.

Check-ins run in Home's current page every `heartbeat.every_minutes` within `heartbeat.active_hours` (optionally on `heartbeat.model`). They only look: any tool that would change something is refused, and nobody is asked. A reply of `HEARTBEAT_OK` (at its start or end, little else) means nothing needs the person: the turn is taken back (`turn.retracted`), and a check-in the person asked for leaves a `notice`. Otherwise the message stays and an `agent.completed {agent: "check-in", title: "Check-in", summary}` follows, which clients notify for (and don't show as a line, since the message is there). Check-ins aren't kept in conversation memory.

Home pages never appear in `session.list` and can't be renamed, archived, pinned, moved or deleted (`error {invalid}`). They work in the data folder. In Home the model is told it's Home; in every session the read-only tools `sessions_list` and `session_read` read the person's other sessions and Home pages.
`state` is `idle` · `working` · `waiting_for_you`.

## 7. Turns

A turn is one user message and everything Thrall does about it.

| Message | Direction | Notes |
|---|---|---|
| `turn.send {session, text, files?, mentions?}` | client → | `files` = ids from `/v1/files`; `mentions` = `@file` paths |
| `turn.started {turn}` | event | |
| `turn.queued {turn, position}` | event | Sent while a turn is already running: the text joins the running turn at its next step, as Python Thrall's interjections do |
| `turn.interrupt {session}` | client → | Esc. Stops the current turn after killing any running tool's process tree |
| `turn.retry {session}` · `turn.edit {session, text}` | client → | Take back the last turn and send its message again (or `text` instead). Refused while a turn runs. Files it changed go back to how they were, except ones someone changed since (a warning `notice` names them) |
| `checkpoint.preview {session, turn}` | client → | Answered by `checkpoint.shown {turn, text, files: [{path, action: restore·delete·unavailable, changed?}]}`: what rewinding to before `turn` would do. `changed`: the file was changed after Thrall's last edit |
| `turn.rewind {session, turn, files, conversation, keep?}` | client → | Go back to before `turn`: put files back (except paths in `keep`) and/or take back it and every later turn (one `turn.retracted {turn}`). Refused while a turn runs. Answered by `turn.rewound {text, restored, failed}`; `text` is the message to put back in the box. Only Thrall's file tools are checkpointed (copies over 10 MB aren't kept); what shell commands change isn't |
| `turn.retracted {turn}` | event | The turn was taken back, with every turn after it: clients remove what they showed; the model no longer sees them |
| `turn.completed {turn, status, usage}` | event | `status`: `done` · `interrupted` · `failed`. `usage`: tokens in/out, cost |

## 8. What the user watches: messages, steps, tasks

**Streamed text**

| Event | Notes |
|---|---|
| `message.started {turn, message, role}` | `role`: `assistant` (Thrall) or `user` (echo of a user message, so other clients see it) |
| `message.delta {message, text}` | Append `text`. The engine batches deltas about every 30 ms. Live-only: no `seq`, never stored |
| `reasoning.delta {message, text}` | Optional thinking text, for models that expose it; clients may fold it away. Live-only |
| `message.completed {message, text, files?}` | Full final text. Clients replace what they streamed with this. `files` (user messages): `[{file, name, kind, size}]`, `kind` = `image` · `document` · `text` · `other` |

**Tool steps** (collapsible rows)

| Event | Notes |
|---|---|
| `step.started {turn, step, tool, title}` | `title` is plain language: "Reading src/main.rs", "Running cargo test" |
| `step.output {step, text}` | Optional live output (shell). Live-only, capped; the model gets the full result |
| `step.completed {step, status, summary, diff?, files?}` | `status`: `ok` · `error` · `denied` · `cancelled`. `diff` for file edits (below). `files`: pictures the step made (`make_image`), as in `message.completed` |

**Diffs**: `{path, kind: "create"|"modify"|"delete", unified, added, removed}`. `unified` is a standard
unified diff; clients render it. Diffs over the frame limit carry a file id instead of `unified`.

**Task list**: `tasks.updated {session, items: [{id, text, status}]}`, the whole list each time
(it's small). `status`: `pending` · `in_progress` · `done`.

**Session details**: `session.updated {session, title?, mode?, workspace?}` when the title changes
(Thrall names a session after its first turn), the mode changes, or the session moves to another
folder (`/folder <path>`).

**Status line**: `status {session, state, model}`, live-only (no `seq`); a client that attaches reads the state from `session.opened`.
`state`: `idle` · `working` · `waiting_for_you`. Sent when any of these change.

**Facts**: `memory.saved {fact, topic, content}` (stored) whenever Thrall saves a lasting fact about the person; clients show it with a Forget action (`memory.forget`). Saving needs no approval unless `memory.save_facts` is off; proposing a skill (`skill_propose`) always asks, with the `SKILL.md` as the request's `diff`.

**Notices**: `notice {level, text}`. `level`: `info` · `warning`. For things like "background agent
finished" or "MCP server reconnected". Never used for questions (those have their own types below).

## 9. Asking the user

Every client shows these the same way: a card with buttons, never "type yes".

**Permission**

```
engine → permission.request  { request, session, turn, step, tool, summary, risk, diff?,
                               choices: ["yes", "no", "always"], expires_at? }
client → permission.answer   { request, choice }
engine → permission.resolved { request, choice, answered_by: "desktop" }
```

- `summary` is plain language: "Delete 3 files in build/".
- `risk`: `low` · `medium` · `high`. Clients colour by it, from the design system.
- **`always` is left out of `choices`** for actions that always ask (restarts, deleting outside the
  workspace, anything that sends or publishes). The engine refuses an `always` it didn't offer.
- The first answer wins. `permission.resolved` goes to every attached client so the others close the
  card. A late second answer gets `error {code: "already_answered"}`.
- Edits in *Ask* mode come as a permission request carrying the `diff`: accept is `yes`, reject is `no`.

**Permission modes** (`session.set_mode`): `ask` (default) · `auto_edits` (file edits inside the
workspace go through; everything else asks) · `plan` (read-only until a plan is approved) ·
`full_auto` (no prompts, except the always-ask actions above).

Full auto is the owner's only: the engine refuses it (in `session.set_mode`, `session.create` and
`/mode full`) for anyone else, whatever a client shows. It lasts `until` `task_end` (the default:
back to `ask` when the task finishes), `time` (after `minutes`, 1 to 1440) or `manual` (until
switched off), and never past a restart of Thrall. One chat per person is in Full auto: switching
it on in another chat switches the first back to `ask`. `session.updated` and `session.opened`
carry `mode_limit {until, ends_at?}` while it's on; a `notice` says why it ended.

**Questions** (Thrall needs information, not permission)

```
engine → question.ask      { question, items: [{text, options?, recommended}], why?, expires_at }
client → question.answer   { question, answers: [string] }       // one per item, in order
engine → question.resolved { question, answers, answered_by }    // answered_by "timeout": nobody did
```

- Thrall decides what it sensibly can and lists what it assumed at the end; it asks (the
  `ask_user` tool) only per `[agent] questions`: `never`, `when_it_matters` (default: things that
  can't be undone, cost money, or could take the work two very different ways) or `when_unsure`.
- One card per task, at the start, holding every question, each with Thrall's recommended answer
  already picked. Clients let the person pick an option or type their own answer.
- Nobody answering by `expires_at` (`[agent] question_wait_minutes`, default 5) means the
  recommended answers are used (`answered_by: "timeout"`). Full auto, scheduled jobs and helpers
  never show a card: Thrall goes with its own answers.
- First answer wins, as with permissions; an empty answer counts as the recommended one.

**Plans** (plan mode)

```
engine → plan.proposed { plan, session, markdown }
client → plan.decide   { plan, approve: true|false, auto_edits? }
engine → plan.resolved { plan, approved, answered_by }
```

## 10. Slash commands

Clients never send `/compact` as text. They send it as a command, so the engine (not the client)
decides what it means.

| Request | Result |
|---|---|
| `command.list {}` | `command.listed {items: [{name, summary, args_hint, skill?}]}`, for autocomplete. Items with `skill: true` are the person's own skills (turned on) and built-ins Thrall carries out itself (`/humanise <text or file>`, also typed `/humanize`; with `writing.detector_aware` on it also works on sentence-length variation and word choice): clients send `/name …` as a message (`turn.send`) and Thrall does it; a project's skills work typed the same way |
| `experience.list {}` | `experience.listed {items: [{lesson, text, at}]}`: your experience log, newest first. Thrall notes a lesson (the `experience_note` tool, no permission needed) when you correct it or a step failed and it found what works; the newest 15 go into every turn's instructions |
| `experience.forget {lesson}` | `experience.listed`; that lesson is gone (no such lesson: `not_found`) |
| `memory.list {}` | `memory.listed {facts: [{fact, topic, content, at}]}`: the facts Thrall remembers, newest first |
| `memory.forget {fact}` | `ack`: Thrall no longer recalls it (kept, marked forgotten); `error {not_found}` when there's no such fact |
| `skill.remove {name}` | `settings.shown {…}`: deletes one of the person's own skills (its folder in the data folder) and takes it off the turned-off list; project skills can only be turned off. `error {invalid}` otherwise |
| `command.run {session, name, args}` | `command.result {re, markdown}`; long commands also stream events |
| `settings.get {}` | `settings.shown {config, config_file?, model, keys: [{name, purpose, set}]}`: config.toml as JSON for the settings screen (read only). MCP environment and header values that aren't `keychain:` references come back as `••••`, and so does a web server address's query; keys report only whether they're stored. `mcp_signed_in` lists the web MCP servers with a stored sign-in (`/mcp login <name>`) |
| `settings.set {key, value}` | `settings.shown {…, pending}`: changes one setting (`key` dotted, from the engine's editable list; `value` yes/no, whole number or text) in config.toml, keeping comments. Used at once when idle, otherwise when the running work ends (`pending: true`). Wrong key or type: `error {invalid}`. `skills.<name>.enabled` (yes/no) turns a skill on or off (`[skills] disabled`); `memory.save_facts` off makes every fact ask first. `mcp.servers.<name>.shared` (yes/no) shares a connected service with everyone using this Thrall (they use it as the owner, with the owner's sign-ins and keys); off, the default, it's the owner's only. Others see only shared services in `settings.shown`, in `/mcp` and to Thrall, and only the owner can sign Thrall in to services or out of them. `settings.shown` also lists the person's own skills: `skills: [{name, description, personal, helper, enabled}]` |
| `provider.test {supplier}` | `provider.tested {supplier, ok, message, models?}`: the owner only. Checks a supplier with the key Thrall has for it (keys never cross the connection): `openrouter` (credit left), `chatgpt` (the plan's models), `local` (the server's models), `fal` (only whether the key is saved), or one reached directly, `openai`, `anthropic`, `google` or `elevenlabs` (its model list). `models` are named as Thrall takes them: `direct/openai/gpt-5`, `local/llama3.1`. A model named `direct/<supplier>/<model>` goes straight to that company with the person's own key (`thrall key set <supplier>`); its cost comes from OpenRouter's prices for the same model |

Unknown names get `error {code: "unknown_command"}` with a "type /help" hint in `message`.

## 11. Background work

| Event | Notes |
|---|---|
| `agent.started {session, agent, title}` | Sub-agent or background task |
| `agent.completed {agent, status, summary}` | Also raises a `notice`; Android turns it into a phone notification |

Scheduled jobs report the same way, into the session they were created from.

## 12. Errors

```json
{ "v": 1, "type": "error", "re": "c-17", "code": "provider_unavailable",
  "message": "OpenRouter isn't responding. Thrall will retry in 30 seconds.", "retry": true }
```

- `message` is written for a non-technical user: what happened, what to do. No stack traces.
- `code` is stable; clients switch on it, never on `message`.

| Code | Meaning |
|---|---|
| `unauthorized` | Bad or missing token |
| `unsupported` | Unknown request type or protocol version |
| `invalid` | Request is malformed; `message` names the field |
| `not_found` | Session, turn, request or file doesn't exist (or isn't yours) |
| `busy` | Can't do that while a turn is running |
| `already_answered` | Someone else answered that prompt first |
| `unknown_command` | No such slash command |
| `rate_limited` | Too many requests; `retry_after_ms` included |
| `provider_unavailable` | The AI provider failed or timed out |
| `provider_key_missing` | No AI key set; `message` says how to add one |
| `provider_key_rejected` | The AI provider didn't accept the key |
| `provider_rejected` | The AI provider refused the request (unknown model, too long, out of credit) |
| `too_large` | Frame or file over the limit |
| `internal` | Engine bug; logged with an id the user can quote |

## 13. Settings and secrets

- `settings.get {}` and `settings.set {key, value}` → `settings.shown {…}` (built; see §10).
- **API keys are write-only.** `secret.set {name, value}` stores the key (OS keychain locally,
  encrypted at rest on servers) and replies `secret.saved {name, preview: "sk-…a1b2"}`.
  No message ever sends a full key back to any client, and keys never appear in logs or events.

## 13a. Phones

Managed from a local client (refused with `unauthorized` over the phone link itself):

| Request | Reply |
|---|---|
| `phone.list {}` | `phone.listed {items: [{phone, name, added, last_seen?, connected?, person}], remote: {enabled, address, listening, port}}`. `person` is whose phone it is |
| `phone.pair {person?}` | `phone.pairing {link, qr, expires}`: a one-time code (5 minutes, 5 wrong tries) for `person`'s phone (absent: the owner's; someone not here: `not_found`). `qr` is the QR's rows (`1` dark). Needs `remote.enabled` and `remote.address` |
| `phone.assign {phone, person}` | `phone.listed`; the phone becomes `person`'s: its open connections close and it reconnects as them |
| `phone.remove {phone}` | `phone.listed`; the phone's open connections close and it can't connect again |

**Pairing link** (what the QR holds): `thrall://pair?a=<address>&k=<engine public key>&i=<code id>&s=<secret>`,
each value base64url without padding (the key and secret are 32 bytes, the id 16).

**Handshake.** The phone keeps its own X25519 key pair. Its first binary message is one byte, then:

- `1` (pair): the 16-byte code id, then `Noise_IKpsk1_25519_ChaChaPoly_BLAKE2s` message 1 with the
  secret as the pre-shared key and payload `{"name": "<phone name>"}`. A wrong secret fails here.
  The engine's answer (message 2) carries `{"phone": "<id>"}`.
- `2` (connect): `Noise_IK_25519_ChaChaPoly_BLAKE2s` message 1; only paired keys get an answer.

Both use the prologue `thrall-remote-v1`. A refused handshake just closes the socket; 20 failures a
minute pause pairing (not paired phones) for a minute.

**After the handshake** every binary message is one Noise transport message whose plaintext is a
marker byte (`1` more pieces follow, `0` last piece) and up to 65,518 bytes of a frame; the pieces
of a frame joined are the ordinary JSON frame. Inside, it's this protocol: `hello` (its `token` is
ignored: the key already proved who this is), requests, events, `ping`. WebSocket ping frames also
count as signs of life.

**Files over the socket** (any client; phones have no HTTP route):

| Request | Reply |
|---|---|
| `file.put {upload?, name, data}` | `file.saved {file, name, size}`. `data` is base64, at most 512 KiB before encoding; without `upload` it starts a new file, with it (from an earlier `file.saved` on this connection) it adds to the end. Same 25 MiB cap; the id works in `turn.send {files}` |
| `file.get {file, offset}` | `file.part {file, name, offset, size, data}`: up to 512 KiB from `offset`; ask again from `offset + piece length` until `size` |
| `app.latest {}` | `app.latest {release?: {version_code, version_name, size, sha256, notes}}`: the phone app this engine offers as an update (`scripts/release-android.ps1` puts it in the data folder's `phone-app` folder with a `release.json`); absent when there's none |
| `app.get {offset}` | `file.part {file: "app", name, offset, size, data}`: that APK in pieces, like `file.get`. The phone checks `sha256` before installing |

**A session's folder, read only** (any client; the phone's Files view). Paths are relative to the
session's folder; nothing outside it can be reached (no `..`, no drive or absolute paths, no links
leading out), and nothing is changed. A session without a folder of its own shows the folder its
turns work in (the one the engine was started in).

| Request | Reply |
|---|---|
| `folder.places {}` | `folder.places {recent: [path], default?, new_in?}`: folders sessions use, most recently active first (up to 8, only ones still there), and Thrall's own folder (`Documents\Thrall`) for `session.create`; the engine makes that one when a session is created in it. `new_in` is where a new folder goes unless another is picked (the `folders.new_in` setting, else `default`). Any other `workspace` given to `session.create` must exist, or the answer is `error {invalid}` |
| `folder.create {parent, name}` | `folder.created {path}`: the one change allowed here, a new empty folder. `parent` must be one `folder.places` offers (`recent`, `default` or `new_in`; the last two are made if missing); `name` is one plain folder name (no `\ / : * ? " < > \|`, not dots, not a name Windows keeps like `CON`). Something already called that is never replaced: `error {invalid}` |
| `folder.list {session, path}` | `folder.listed {session, path, items: [{name, folder?, size, modified}], more?}`: folders first, then files, each newest first; at most 500 entries (`more` when some were left out). `path` "" is the folder itself |
| `folder.read {session, path, offset}` | `folder.part {session, path, offset, size, data}`: like `file.part`, up to 512 KiB of base64 from `offset`. A file that isn't at `path` is looked for a few folders down (Thrall often names a file without the folder it made it in): one match is read and its real `path` returned; several is a `not_found` error naming them |

## 13b. People

Owner only (anyone else gets `unauthorized`). Each answer is the whole list again.

| Request | Reply |
|---|---|
| `people.list {since?}` | `people.listed {items: [{person, name, role, added, phones, spent, own_key, key_set, model?, monthly_limit?}]}`: the owner first, then oldest first. `spent` is what their turns cost (USD) since `since` (Unix ms; the web app sends the start of this month), or ever when absent. Anyone but the owner gets only their own entry |
| `people.add {name}` | `people.listed`; the new person is a `member`. Names are trimmed, 1–40 characters |
| `people.rename {person, name}` | `people.listed` |
| `people.remove {person}` | `people.listed`; they can't connect again and their phones are forgotten (their links close). Their sessions and files are kept. The owner can't be removed |
| `people.terms {person, own_key, model?, monthly_limit?}` | `people.listed`. Owner only. `own_key`: their turns use their own AI key, not the owner's; `model`: their own model; `monthly_limit`: the most their turns may cost in a calendar month (US$). The owner can set only a limit on themselves (a guard against runaway spending) |
| `people.key {person?, key?}` | `ack`. Keeps (or, without `key`, forgets) a person's own AI key: your own, or anyone's for the owner (others: `unauthorized`). Write-only; `people.listed` says only `key_set`. The owner's key is set with `thrall key set` |

A turn by someone on their own key before it's set fails with `provider_key_missing`; at their
limit with `provider_rejected`, and anything else that costs money for them (pictures, speech,
helpers) stops too. From 80% of the limit a turn starts with a warning notice. Limits run from
local midnight on the 1st.

## 13c. Signing browsers in

No passwords. A browser only gets in through a one-time sign-in link: the page itself carries no
token, because anything on the computer (other accounts, other programs) can load it. The owner
makes links for people; the desktop app makes one for its own window each time it connects (a
window that's already signed in keeps its sign-in); `thrall open` and `thrall engine` make one
for the owner's browser.

| Request | Reply |
|---|---|
| `signin.create {person}` | `signin.code {code, name, expires}` (owner only; someone not here: `not_found`). The link is the web app's address + `#signin=<code>`; it works once, for 30 minutes |
| `device.list {person?}` | `device.listed {person, items: [{device, name, added, last_seen?, current?}]}`: signed-in browsers, newest first. Your own; the owner may name anyone (others: `unauthorized`) |
| `device.remove {device, person?}` | `device.listed`; that browser's token stops working and its open connections close |

**Trading the code.** `POST /v1/signin` with `{"code", "name"}` (`name`: what the browser calls
itself, e.g. "Edge on Windows") answers `{"token", "person": {"id", "name"}}`, or `401
{"error"}` when the code is wrong, used or expired. The browser keeps the token and shows it in
`hello {token}` and as `Authorization: Bearer` for files, exactly like the access token; it acts as
its person (their sessions, files and folder). Only the token's SHA-256 is stored. Removing a
person signs all their browsers out.

## 13d. Personality

How Thrall talks to each person. Everyone starts on the standard one Thrall ships with; each person
(children included) chooses their own, and it never changes anyone else's. It's tone only: it's
added after Thrall's rules and says it can't change them, what needs permission, or what's safe.

| Request | Reply |
|---|---|
| `personality.get {}` | `personality.shown {preset, custom, presets: [{id, name, description}]}`: your choice, and the presets (standard first) |
| `personality.set {preset, custom?}` | `personality.shown`. `preset` is a preset's id or `custom` (then `custom` is your own words, up to 1500 characters). An unknown preset means the standard one |

Kept in the person's own data folder (`personality.json`).

## 14. Not in v1 yet (names reserved)

- `runner.*` — the desktop app's companion link to a server-hosted engine. A separate
  endpoint (`/v1/runner`) with its own message set: the engine asks the runner to run a tool inside an
  approved folder; the runner can always refuse.
- `account.*` — sign-in and accounts, for a server edition.
- `sync.*` — anything else the phone needs beyond events (the phone link and file pieces are in §13a).

**Telegram** runs inside the engine process. It uses these same message types internally, so it
behaves like every other client, but it doesn't need a WebSocket.

## 15. Example: one turn with a permission prompt

```
→ {"v":1,"type":"turn.send","id":"c-4","session":"s_1","text":"delete the build folder"}
← {"v":1,"type":"ack","re":"c-4"}
← {"v":1,"type":"turn.started","session":"s_1","seq":41,"turn":"t_9"}
← {"v":1,"type":"message.started","session":"s_1","seq":42,"turn":"t_9","message":"m_20","role":"assistant"}
← {"v":1,"type":"message.delta","session":"s_1","seq":43,"message":"m_20","text":"I'll remove build/."}
← {"v":1,"type":"message.completed","session":"s_1","seq":44,"message":"m_20","text":"I'll remove build/."}
← {"v":1,"type":"step.started","session":"s_1","seq":45,"turn":"t_9","step":"p_3","tool":"fs_delete","title":"Deleting build/"}
← {"v":1,"type":"permission.request","session":"s_1","seq":46,"request":"r_2","turn":"t_9","step":"p_3",
   "tool":"fs_delete","summary":"Delete build/ (214 files)","risk":"medium","choices":["yes","no","always"]}
→ {"v":1,"type":"permission.answer","id":"c-5","request":"r_2","choice":"yes"}
← {"v":1,"type":"ack","re":"c-5"}
← {"v":1,"type":"permission.resolved","session":"s_1","seq":47,"request":"r_2","choice":"yes","answered_by":"tui"}
← {"v":1,"type":"step.completed","session":"s_1","seq":48,"step":"p_3","status":"ok","summary":"Deleted 214 files"}
← {"v":1,"type":"turn.completed","session":"s_1","seq":49,"turn":"t_9","status":"done",
   "usage":{"input_tokens":1840,"output_tokens":96,"cost_usd":0.0031}}
```

## 16. How it's built (Rust)

- One `serde` enum for client requests and one for engine events, tagged by `type`
  (`#[serde(tag = "type")]`), in `thrall-protocol`. No `serde_json::Value` in the public types.
- `schemars` generates the JSON Schema at build time. The web/desktop TypeScript types and the
  Android Kotlin types are generated from it.
- Round-trip tests: every example in this document is a test fixture that must parse and re-serialise
  unchanged.
- The schema is committed as `docs/protocol.schema.json`; a test fails when it drifts from the types
  (refresh with `THRALL_UPDATE_SCHEMA=1 cargo test -p thrall-protocol`).
- The protocol crate depends on nothing in the engine. Clients depend only on it.
