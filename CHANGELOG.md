# Changelog

Everything that changes in Thrall, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/): while Thrall is 0.x, a new feature bumps the middle
number (0.2.0) and a fix-only release bumps the last (0.1.1).

## [Unreleased]

### Added

- Settings → Steps: how many steps Thrall may take on one message before it stops and asks you
  to say "carry on" (100 by default, 250 in Full auto, up to 500). In the terminal:
  `/settings agent.max_steps <number>` and `agent.full_auto_max_steps`.
- Near the limit Thrall is told to wrap up: finish what's in hand, or stop at a safe point and say
  what's done and what's left, instead of being cut off mid-change.
- Thrall is told its step limit at the start of a message and where it stands every 25 steps
  ("step 75 of 250"), so it can plan around the limit.
- Thrall cleans up after itself: before testing it checks for programs and files left from
  earlier work, it notes anything it starts that keeps running, and when it's done it stops what
  it started (or says what's still running and why). Starting a program with Start-Process or
  similar reminds it to note the program so it can be stopped later.
- shell keep_running works without waiting for a line: a program still going after a few seconds
  is kept running and tracked. Python programs print as they go instead of holding output back.
- Thrall can make several changes to a file in one step, and they're all made or none are. Each
  edit shows the changed lines as they now read, so Thrall no longer reads the file back after
  changing it.
- When Thrall edits the same file again a step or two later, it's reminded to put all the
  changes to a file in one edit next time.
- If the step limit changes during a message (the mode was changed), Thrall is told the new one.

### Changed

- Thrall was stopped after 60 steps; long jobs now get 100 (250 in Full auto).
- Thrall's file tools work in any folder, not just the session's: asked to work somewhere else, it
  writes and edits files there with the file tools (read first, undoable) instead of going round
  through the shell. Your mode still decides whether it asks, and a person kept out of system
  folders still is.
- Auto edits now runs everyday commands without asking too (building, testing, running a script,
  checking a web address). Deleting, installing and stopping a program still ask, and so does
  anything risky.
- Risky actions ask in every mode, Full auto included, and "always" never covers them: changing
  the registry or system files, wiping or formatting disks, deleting whole folders outside the
  session's, stopping programs by name, force pushes, and restarts or shutdowns. Before, Full auto
  ran them without asking.
- A change to the registry or system files through the shell has to be reversible: Thrall makes
  a backup first, and the permission card says what will change and how to undo it. Without the
  backup it's refused. A system file changed with the file tools asks every time, and its copy
  is kept so rewinding the message puts it back.
- Approving a plan goes on in Auto edits, so the work doesn't stop to ask before every change.
  "Approve, ask before each change" (`a`) keeps asking. The phone app still approves into Ask for
  now.

### Fixed

- A command that started a program on its own (like Start-Process with its output saved to a file)
  made Thrall wait the full timeout. It now hands back as soon as the command ends and says a
  program it started is still running.
- After a command was stopped, Thrall said everything it started had been stopped, when a program
  started on its own could still be running. It now says so, and to check, when the command
  starts one (a plain search that timed out no longer gets the warning).
- A message typed while Thrall was working could be taken as the whole new task: he dealt with it
  and finished, dropping what he was asked first without saying so. He's now told the task in
  hand still stands, and to say what's left if he stops early.
- Summarising a long conversation mid-task could summarise the request being worked on, so it
  read as done and the newest message as the whole job. The summary now stops at the start of the
  task in progress, unless that task alone is too big to fit.
- Searching MCP tools by key words started every configured MCP server, all at once, for a guess
  (18 programs, some signed in to your accounts). Key words now search only the servers already
  running, and Thrall is told which aren't started; naming a server starts just that one.
- Opening a windowed app (a Python app with a window, for one) could hang Thrall until the timeout,
  report the app stopped while it kept running, and leave copies open. Thrall now hands back once
  the command ends, names what's still running with its process ID, and keeps track of it, so
  shell_output and shell_stop work on it. Waiting for a line and being stopped really stops it.
- The reminder to note a program started out of sight came up when a program was only named, as
  in `Get-Process pythonw`. It now only comes up when one is started.

## [0.2.4] - 2026-11-09

Mode changes take hold while Thrall is working, and Full auto is easier to set.

### Added

- `/mode full 30m`, `/mode full 2h` or `/mode full on`: Full auto for a while, or until you
  switch it off. The desktop app's mode chip opens a menu with the same choices (owner only).

### Fixed

- Changing the mode while Thrall is working takes effect from his next step, not his next
  message. Switching Full auto on mid-task stops the questions; switching it off brings them back.
- Thrall no longer takes the summary of a long chat as a new request and picks old tasks back up
  by himself after the chat is summarised.
- A chat left in Full auto when Thrall restarts goes back to Ask and says so in the chat, and the
  app no longer keeps showing the Full auto banner for it.
- Files saved as UTF-16 (what PowerShell's `>` writes) can be read.
- An unknown mode in `/mode` now lists the Full auto choices too.

## [0.2.3] - 2026-11-06

One Thrall for the whole household or a small team: companion PCs, and controls per person.

### Added

- Companion PCs, first part: other PCs at home or at work can find this Thrall and ask to join as
  someone's (Settings → PCs → Let PCs on this network connect, off by default). A card pops up
  with the PC's name and a code that its screen shows too: Let in (as whose PC), Decline or
  Block. Blocked PCs can't ask again until unblocked. Everything between the PCs is encrypted.
- Companion PCs do their own work: a chat started on a family member's or a worker's PC (or by
  them on their phone) works in that PC's folders, and its file, document and command steps run
  on that PC. The AI, memory and the web stay on the Thrall PC. If that PC is off, Thrall says so
  and does nothing on the Thrall PC instead. The phone's Files view shows the PC's files, and Home
  moves to the PC once it's connected.
- The desktop app can be a companion: on a new PC, "Connect to Thrall on another PC" finds Thrall
  on the network (or takes its name over Tailscale), shows the code, and once let in it needs no
  AI key of its own. In a terminal: `thrall join`, `thrall runner`, `thrall leave`.
- Settings → People → Controls: for each person, switch off commands, file changes in system
  folders, or their phones, or make it a child's account (answers fit for their age). Everything is
  allowed until you switch it off.
- Controls → "What you can see of their chats": nothing (the default), chat titles, titles and the
  chats where something was blocked, or every chat. They see a note in every chat saying so, and
  each time you read one it goes in the audit log.
- One-off reminders: "remind me at 8:15", "in 20 minutes", "tomorrow at 7". Thrall uses its own
  scheduler for every reminder and alarm, and says when it's time.
- Rewinding a chat on a companion PC puts that PC's files back too, and a project's `THRALL.md`
  and skills in a folder on that PC are read from there.

### Fixed

- Text read from PDFs no longer comes out with stray gaps ("P romptly", "P P E"). The PDF
  reader misread how wide some letters are in the way Chrome and others save fonts; Thrall now
  rewrites that part before reading, without changing the file.
- The preview picture of a PDF Thrall makes is as tall as the pages, instead of always eight
  pages tall with blank space at the bottom.
- On Windows, Thrall's engine (`thrall.exe`) shows its version and name in Task Manager and file
  properties, like the desktop app. It was blank, which looked like a second, different Thrall.
- Installing, updating or uninstalling on Windows stops Thrall's engine first, so its files are
  never locked mid-install.

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

[Unreleased]: https://github.com/ThrallAI/thrall/compare/v0.2.4...HEAD
[0.2.4]: https://github.com/ThrallAI/thrall/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/ThrallAI/thrall/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/ThrallAI/thrall/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/ThrallAI/thrall/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/ThrallAI/thrall/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ThrallAI/thrall/releases/tag/v0.1.0
