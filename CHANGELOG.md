# Changelog

Everything that changes in Thrall, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/): while Thrall is 0.x, a new feature bumps the middle
number (0.2.0) and a fix-only release bumps the last (0.1.1).

## [Unreleased]

### Fixed

- Text read from PDFs no longer comes out with stray gaps ("P romptly", "P P E"). The PDF
  reader misread how wide some letters are in the way Chrome and others save fonts; Thrall now
  rewrites that part before reading, without changing the file.
- The preview picture of a PDF Thrall makes is as tall as the pages, instead of always eight
  pages tall with blank space at the bottom.

## [0.2.2] - 2026-11-04

Updates in one click from now on, and Home's files where you can find them.

### Added

- Updates in one click: the desktop app looks for a new version when it starts and every few
  hours, says so once, and installs it when you click Update (Settings → Check for updates, or
  the tray menu). No admin prompt, and Thrall restarts by itself. Updates are signed: one that
  isn't signed with Thrall's key is refused. From 0.2.2 on; 0.2.1 and older are updated by hand
  one last time.
- Settings → Thrall's folders shows the working folder and the data folder, each with
  an Open button.
- `thrall remove-everything` removes Thrall's data folder, its keys and starting at sign-in, for
  when you're done with Thrall. It asks you to type `remove everything` first, only runs with
  Thrall stopped, and keeps your working folder. The README says how to update, uninstall and
  remove everything.
- The welcome screen shows where Thrall puts what it makes for you, with Open and Change.

### Changed

- Home works in `Documents\Thrall\Home` (anyone else in the household gets `Home - <name>`),
  where what it makes is easy to find, instead of a hidden folder inside the data folder. Home's
  files move there the first time Home opens.
- When OneDrive syncs Documents, Thrall's working folder is `Thrall` in your user folder instead,
  so projects and build folders aren't uploaded and locked mid-build. A `Documents\Thrall` that's
  already there stays in use.
- "New folders go in" is now "Thrall's working folder", and Home uses it too.
- Uninstalling on Windows also stops Thrall starting at sign-in. Installing a new version over an
  old one keeps it on.
- The welcome screen says plainly that Thrall works as you: it can change and delete your files
  and run commands, and asks first unless you choose otherwise.

### Fixed

- Settings → Check for updates always said it couldn't reach GitHub: the page's security rules
  blocked the request. Now the page may ask GitHub's API, and nothing else outside Thrall.

## [0.2.1] - 2026-11-04

Fixes for long jobs that stopped with an empty, cut-off reply, and the Intel Mac installer is back.

### Fixed

- A reply cut off at its length limit before saying anything is no longer asked the same way
  again (which ran out again): Thrall tells the model it was cut off and to work in smaller
  steps. A new install lets a reply run to 16,384 tokens instead of 8,192 (Settings → Models, or
  `/settings provider.max_tokens <number>` in the terminal).
- A model that thinks before replying (such as DeepSeek) could spend the whole length limit
  thinking and come back with nothing, again and again. Once that happens, Thrall asks it to think
  less for the rest of the task, so it has room to act (OpenRouter only).
- Both Mac installers are built again (0.2.0 came without the Intel one).

## [0.2.0] - 2026-11-03

Thrall learns, asks, gets a personality, and works with the supplier you pick.

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
- Settings → Learning: open any of your skills to change when it's used and what it says. A
  skill Thrall learned is rated again when you save it, and a medium or high-rated one can be
  made safe: Thrall writes a version that asks before deleting, sending, handling keys or running
  downloads, shown beside yours with its rating, and yours stays until you pick the safer one.
- Helpers work side by side: when Thrall asks for several in one reply they run at once (3 by
  default, up to 8), each with its steps under its own row. Built-in helper types (Researcher,
  Coder, Reviewer, Planner; only the Coder changes files) and your own in the data folder's
  `agents/`. Each helper stops at its spending limit ($1 by default). Settings → Permissions &
  safety → Helpers.
- Settings → Providers: pick a supplier and model for each job (chat, reading pictures, making
  pictures, speaking, transcribing, memory search), with a Test for each supplier. Use OpenAI,
  Anthropic and Google directly with your own key (`direct/<supplier>/<model>`, `thrall key set
  <supplier>`), and ElevenLabs voices. Their cost is worked out from OpenRouter's prices, so
  spending limits keep counting.
- Programs on your own computer, free and with nothing leaving it: whisper.cpp transcribes
  (`program/whisper`), Piper reads aloud (`program/piper`) and a Stable Diffusion web UI makes
  pictures (`program/sd-webui`). "Found on this PC" in Settings → Providers shows what's
  installed and what's missing; Thrall never downloads or installs them itself.
- Local models: `local/<model>` runs a model from Ollama or LM Studio on your own computer or
  network, free and with no key (Settings → Models → "Local model server").
- A new project gets its own folder inside the working folder, even a one-file script.
- The terminal catches up with the desktop's settings screens: `/settings` shows every setting
  and changes one (`/settings agent.verify true`), `/skills` and `/skill <name>` show skills
  (with `safe` for a safer version and `remove` to delete one), `/lessons` lists and forgets what
  Thrall learned from corrections, and `/group <name>` puts a session in a group. Each helper's
  steps are numbered, so helpers working side by side can be told apart.
- The shell can wait for a line of output (such as "server ready") and leave a program running
  in the background, with tools to read its output and stop it.
- `thrall open` opens the web app already signed in.

### Changed

- Quitting the desktop app says how many phones stopping Thrall would disconnect.

### Fixed

- Two tests that failed now and then on Windows.
- The browser gets a minute to start instead of 20 seconds, so it no longer fails on a slow or
  busy computer.
- Errors from a local model server or a direct supplier name that supplier, not OpenRouter.
- A command that deletes files says "Deletes files" at the start of its permission card, even when
  the delete is chained onto something harmless; other risky commands say "Risky".
- Learning a skill says "Learning the … skill" instead of "Proposing": only high-risk skills wait
  for your OK.

### Security

- The web page no longer carries the access token: browsers sign in with a one-time link, and
  the desktop app signs its own window in.
- The data folder is private to your user on Linux and macOS (0700).
- SECURITY.md now says plainly what Thrall protects and what it can't.
- Updated pulldown-cmark (RUSTSEC-2026-0433) and calamine (now on the fixed zip 9); the zip 8
  advisory RUSTSEC-2026-0429 is in archive extraction, which Thrall's Office libraries never use.
- Updated async-executor to 1.14.1: 1.14.0 was withdrawn by its maintainers for a
  use-after-free.

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

[Unreleased]: https://github.com/ThrallAI/thrall/compare/v0.2.2...HEAD
[0.2.2]: https://github.com/ThrallAI/thrall/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/ThrallAI/thrall/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/ThrallAI/thrall/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ThrallAI/thrall/releases/tag/v0.1.0
