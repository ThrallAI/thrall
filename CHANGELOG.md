# Changelog

Everything that changes in Thrall, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/): while Thrall is 0.x, a new feature bumps the middle
number (0.2.0) and a fix-only release bumps the last (0.1.1).

## [Unreleased]

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

[Unreleased]: https://github.com/ThrallAI/thrall/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ThrallAI/thrall/releases/tag/v0.1.0
