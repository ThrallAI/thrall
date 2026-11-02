# Changelog

Everything that changes in Thrall, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/): while Thrall is 0.x, a new feature bumps the middle
number (0.2.0) and a fix-only release bumps the last (0.1.1).

## [Unreleased]

### Added

- Settings → Learning: open any of your skills to change when it's used and what it says. A
  skill Thrall learned is rated again when you save it, and a medium or high-rated one can be
  made safe: Thrall writes a version that asks before deleting, sending, handling keys or running
  downloads, shown beside yours with its rating, and yours stays until you pick the safer one.
- Helpers work side by side: when Thrall asks for several in one reply they run at once (3 by
  default, up to 8), each with its steps under its own row. Built-in helper types (Researcher,
  Coder, Reviewer, Planner; only the Coder changes files) and your own in the data folder's
  `agents/`. Each helper stops at its spending limit ($1 by default). Settings → Permissions & safety → Helpers.
- Settings → Providers: pick a supplier and model for each job (chat, reading pictures, making
  pictures, speaking, transcribing, memory search), with a Test for each supplier. Use OpenAI,
  Anthropic and Google directly with your own key (`direct/<supplier>/<model>`, `thrall key set
  <supplier>`), and ElevenLabs voices. Their cost is worked out from OpenRouter's prices, so
  spending limits keep counting.
- Programs on your own computer, free and with nothing leaving it: whisper.cpp transcribes
  (`program/whisper`), Piper reads aloud (`program/piper`) and a Stable Diffusion web UI makes
  pictures (`program/sd-webui`). "Found on this PC" in Settings → Providers shows what's
  installed and what's missing; Thrall never downloads or installs them itself.

### Fixed

- The browser gets a minute to start instead of 20 seconds, so it no longer fails on a slow or
  busy computer.
- Errors from a local model server or a direct supplier name that supplier, not OpenRouter.

### Security

- Updated async-executor to 1.14.1: 1.14.0 was withdrawn by its maintainers for a
  use-after-free.
## [0.2.0] - 2026-11-02

Thrall learns, asks, and gets a personality.

### Added

- Question cards: when a choice really matters, Thrall asks with a short card of options (on the
  desktop, in the browser, on the phone with a notification, and in the terminal) instead of
  guessing. Settings → Questions sets how often it asks and how long it waits.
- Personality: everyone starts with Thrall's standard personality, and each person (children
  included) can pick another (friendly, brief, teacher, playful) or write their own. It changes
  tone only, never the rules.
- Learning (Settings → Learning): Thrall saves skills it works out by itself, each with a safety
  rating; low and medium are kept on their own, high-rated ones always ask first. It also keeps
  lessons from what went wrong or what you corrected, and saves the lasting facts you state when
  a long chat is compacted. Everything learned can be viewed and forgotten.
- Local models: `local/<model>` runs a model from Ollama or LM Studio on your own computer or
  network, free and with no key (Settings → Models → "Local model server").
- The shell can wait for a line of output (such as "server ready") and leave a program running
  in the background, with tools to read its output and stop it.
- `thrall open` opens the web app already signed in.

### Changed

- Quitting the desktop app says how many phones stopping Thrall would disconnect.

### Security

- The web page no longer carries the access token: browsers sign in with a one-time link, and
  the desktop app signs its own window in.
- The data folder is private to your user on Linux and macOS (0700).
- SECURITY.md now says plainly what Thrall protects and what it can't.
- Updated pulldown-cmark (RUSTSEC-2026-0433) and calamine (now on the fixed zip 9); the zip 8
  advisory RUSTSEC-2026-0429 is in archive extraction, which Thrall's Office libraries never use.

### Fixed

- Two tests that failed now and then on Windows.

## [0.1.0] - 2026-10-31

The first public preview.

### Added

- Desktop app for Windows, Linux and macOS (window, system tray, notifications, start at login),
  the same app in a browser, a terminal app (`thrall`) and an Android app that pairs with the
  computer over an encrypted link.
- Sessions with their own folders, groups, plans and task lists; file changes shown as diffs and
  undone with a rewind; slash commands, skills, MCP servers, scheduled jobs and a Home
  conversation with daily check-ins.
- Permission modes (Ask, Auto-accept edits, Plan, Full auto). Full auto is owner-only, ends with
  the task or after a set time, never survives a restart, and needs a fingerprint on the phone.
- People: a household can share one Thrall, each person with their own sessions and memory.
- Tools for files, documents (Word, Excel, PDF), the shell, git, the web and a browser, pictures,
  video, speech and transcription.
- Long-term memory with search, and compaction for long sessions.
- First-run key setup in the desktop app, a welcome screen and a one-minute tour.
- Installers built by CI for every release, with `SHA256SUMS.txt`.

[Unreleased]: https://github.com/ThrallAI/thrall/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/ThrallAI/thrall/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ThrallAI/thrall/releases/tag/v0.1.0
