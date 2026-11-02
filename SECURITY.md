# Security

## Reporting a problem

Please don't report security problems in public issues.

Email **thrallAI@protonmail.com** with what you found, how to reproduce it and which version you
were using (Settings → Help shows it). You'll get a reply within a week, and a fix or a plan before
anything is made public. Credit is given in the release notes unless you'd rather stay anonymous.

Only the current release gets security fixes. The original Python version,
[Thrall-Legacy](https://github.com/ThrallAI/thrall-legacy), is archived and isn't patched.

## What Thrall protects you from, and what it can't

Thrall is an agent: it reads files, runs commands and browses the web **as your user account**.
That's what makes it useful, so it helps to know where the safety lines are.

**What's in place**

- **The engine only listens on this computer** (127.0.0.1). Clients prove who they are with a
  token: the terminal and desktop app read it from Thrall's data folder; a browser is signed in
  with a one-time link and gets its own token, which can be signed out.
- **Other web pages can't drive Thrall.** Requests from browsers must come from Thrall's own pages
  (Origin check), and the Host check stops pages that rebind their name to your computer.
- **Phones use an encrypted, authenticated link** (the Noise protocol): pairing needs the one-time
  code from the QR; after that only the paired phone's own key gets in. Repeated failed pairing
  attempts pause pairing.
- **Permission modes**: in Ask (the default) every file change and command waits for you. Risky
  commands are never covered by an "Always" answer. Full auto is owner-only and short-lived.
- **Your AI key** sits in the system keychain, not in a file.
- **No telemetry.** Nothing is sent anywhere you didn't ask for.

**What it can't fully protect you from**

- **Prompt injection.** A web page, document or tool output can contain text written to trick an
  AI ("ignore your instructions and…"). Thrall's instructions tell the model to treat such text
  as information, never as orders, and that helps, but no model follows that perfectly. The real
  safeguard is the permission prompt: **read what you approve**, especially after Thrall has read
  something from the internet.
- **Everyone works through one computer account.** When several people share one Thrall, each has
  their own sessions and memory, but Thrall's tools all run as the account Thrall runs under. Give
  people you don't fully trust their own computer (or their own Thrall) rather than a seat on yours.
- **Anything already running as you.** A program running under your account can read Thrall's data
  folder, like any other file of yours. Thrall can't defend against malware that's already on the
  computer.
- **The AI provider sees what you send.** Prompts and the file contents Thrall reads for a task go
  to the provider you chose (OpenRouter today) and the model behind it.
- **Unsigned installers.** The installers aren't code-signed yet. Check downloads against
  `SHA256SUMS.txt` from the release, or build from source ([BUILDING.md](BUILDING.md)).
