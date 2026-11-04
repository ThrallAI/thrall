# Thrall

**An AI agent that works on your own computer, with your own AI key.** Ask in plain words and
Thrall reads and changes files, runs commands, browses the web and remembers what matters, while
asking before anything changes. Use it from the desktop app, a browser, the terminal or your phone.

> **Preview (0.x).** Early and actively changing; see the [changelog](CHANGELOG.md) for what's new. Windows is used day to day; Linux
> (Ubuntu, Omarchy) is new; **macOS builds are made automatically but haven't been tested by hand
> yet: Mac users, your feedback is very welcome** (see [Contributing](CONTRIBUTING.md)).

## What you get

- **The desktop app** (Windows, Linux, macOS): Thrall in its own window and the system tray, with
  notifications when a task finishes or needs you.
- **The same app in any browser** on the computer (`thrall open` opens it, signed in), and a
  **terminal app** (`thrall`) for keyboard people.
- **An Android app** that pairs with your computer, so you can start and answer tasks from your
  phone (on your home network, or anywhere with [Tailscale](#away-from-home)).
- Sessions with their own folders, groups, plans and task lists, file changes shown as diffs and
  undone with a rewind, slash commands, skills, MCP servers, scheduled jobs and a personal Home
  conversation with daily check-ins.
- **People:** a household can share one Thrall; each person has their own sessions and memory.

## Install

Download the file for your system from the [latest release](../../releases).

| System | File | Notes |
|---|---|---|
| Windows 10/11 | `Thrall_<version>_x64-setup.exe` | Not code-signed yet: Windows SmartScreen says "Windows protected your PC". Click **More info**, then **Run anyway**. |
| Linux (Ubuntu, Debian, Mint…) | `Thrall_<version>_amd64.deb` | `sudo apt install ./Thrall_<version>_amd64.deb` |
| Linux (any) | `Thrall_<version>_amd64.AppImage` | `chmod +x Thrall*.AppImage`, then run it. |
| Arch, Omarchy | AUR `thrall-bin` | `yay -S thrall-bin` (see [packaging/aur](packaging/aur)). |
| macOS 11+ (Apple silicon) | `Thrall_<version>_aarch64.dmg` | **Untested.** Not signed yet: open it once, then allow it in System Settings → Privacy & Security → **Open Anyway**. |
| macOS 11+ (Intel) | `Thrall_<version>_x64.dmg` | As above. |
| Android 11+ | `Thrall-for-Android-<version>.apk` | Allow "Install unknown apps" for your browser or file manager. |

Each release lists `SHA256SUMS.txt`, so you can check a download wasn't changed on the way.
Prefer to build it yourself? See [BUILDING.md](BUILDING.md).

## First run

1. Open Thrall. It asks for an **OpenRouter key**: make one at
   [openrouter.ai/keys](https://openrouter.ai/keys) and paste it in. You pay OpenRouter only for
   what you use, and you choose the model (the Model setting in Settings, or `/model`). The key is
   kept in your computer's
   keychain, never in a file, and is only sent to OpenRouter.
2. A short welcome offers the **one-minute tour** of the screen (take it again any time from the
   command palette, Ctrl+K → "Take the tour").
3. To use your phone: Settings → Phones → **Add a phone**, then scan the code with the Android app.

## Local models (no key, no cost)

Running [Ollama](https://ollama.com) or [LM Studio](https://lmstudio.ai)? Set the model to
`local/<name>` (Settings → Models, or `/model local/llama3.1`). Thrall talks to Ollama at
`http://localhost:11434/v1` by default; for LM Studio set Settings → Models → Local model server to
`http://localhost:1234/v1`. With a local model as the main model, Thrall starts without an
OpenRouter key. Pictures, speech and transcription still use OpenRouter for now.

## Safety: read this once

Thrall acts on **your** computer with **your** permissions. That's what makes it useful, and why
it asks first.

- **Modes** decide what Thrall may do without asking, per chat:
  - **Ask** (the default): it asks before every file edit and every command.
  - **Auto-accept edits:** file edits inside the session's folder go ahead; commands still ask.
  - **Plan:** it only reads and plans until you approve the plan.
  - **Full auto:** edits and commands go ahead without asking, risky ones too. Only the person who
    runs this Thrall can switch it on; it ends when the task finishes (or after a time you pick),
    and never lasts past a restart. On the phone it needs your fingerprint.
- **"Always" answers** cover one kind of command for the rest of a session; risky commands
  (deleting, force-pushing and the like) are never covered by "Always". Restarting Thrall,
  sending things and deleting outside the folder always ask.
- **Your data stays with you.** Sessions, memory and files stay on your computer. Thrall talks to
  the AI provider you choose (OpenRouter), to the websites and services you ask it to use, and to
  GitHub only when you press "Check for updates". No analytics, no ads, nothing phones home.
- Commands and tools run as your user. Don't give it a folder you wouldn't let a helpful but
  fallible person work in, and read what you approve.

> **First run safely**
> - Stay in **Ask** mode until you know how Thrall behaves.
> - Start it on a **throwaway folder**, not your real projects.
> - Give it an OpenRouter key with a **spending limit** (you can set one on the key at OpenRouter).
> - Check the installer against **`SHA256SUMS.txt`** from the release.
> - Text on web pages and in files can try to steer an AI. Thrall is told to ignore it, but the
>   real safeguard is you reading what you approve. More in [SECURITY.md](SECURITY.md).

## Away from home

The phone talks to your computer over your home network. To use it elsewhere, install
[Tailscale](https://tailscale.com) on the computer and the phone, and pair over the Tailscale
address. (A built-in way to reach home is planned.)

## Where Thrall keeps things

| | Windows | Linux | macOS |
|---|---|---|---|
| Settings, sessions, logs | `%APPDATA%\Thrall` | `~/.local/share/Thrall` | `~/Library/Application Support/Thrall` |
| Home and new folders (the working folder) | `Documents\Thrall` (`%USERPROFILE%\Thrall` when OneDrive syncs Documents) | `~/Documents/Thrall` | `~/Documents/Thrall` |
| Your keys | Windows Credential Manager | Secret Service (GNOME Keyring, KWallet…) | Keychain |

Set `THRALL_HOME` to keep everything in another folder. Settings → Thrall's folders opens them.

## Updating and removing Thrall

**Updating.** From 0.2.2, the desktop app tells you when a new version is out, and **Update** (in
Settings → Check for updates, or the tray menu) installs it: no admin prompt, and Thrall restarts by
itself. Updates are signed, and one that isn't signed with Thrall's key is refused. Copies older
than 0.2.2 update by installing the new version over them. The `.deb` updates through your package
manager.

**Uninstalling** (Windows: Settings → Apps → Thrall; Linux: your package manager or delete the
AppImage; macOS: drag Thrall to the Bin) removes the app and, on Windows, starting Thrall at
sign-in. It **keeps** your data folder, your keys and your working folder, so reinstalling picks up
where you left off.

**Removing everything.** To take away your data and keys too, quit Thrall first (tray → Quit Thrall
→ Quit and stop Thrall), then run this and type `remove everything` when it asks:

```powershell
& "$env:LOCALAPPDATA\Thrall\thrall.exe" remove-everything
```

On Linux and macOS it's `thrall remove-everything`. It deletes the data folder (settings,
sessions, memory, logs), Thrall's keys and starting at sign-in, and keeps your working folder and
the files in it. On Linux and macOS, remove Thrall's keys in your keychain app (listed under
"Thrall"). Then uninstall the app as above.

## Troubleshooting

- **Linux: the window is blank** (often Wayland with an NVIDIA card): start it with
  `WEBKIT_DISABLE_DMABUF_RENDERER=1 thrall-desktop`.
- **Linux: "couldn't keep the key"**: Thrall needs a Secret Service keyring (GNOME Keyring or
  KWallet) running in your session. Or set `OPENROUTER_API_KEY` in your environment.
- **Linux: no tray icon** on a desktop without one: install an AppIndicator extension (GNOME), or
  use the window; Hyprland/Omarchy show it in Waybar.
- **The phone says "Reconnecting"**: check the computer is on and Thrall is running, then tap
  **Try now**.
- Logs are in the `logs` folder of the data folder above. Please attach the relevant lines (with
  any keys removed) to bug reports.

## Who's behind this

Thrall is built by one developer and published as **ThrallAI**. Issues and pull requests are read
and answered here on GitHub, and every change is listed in the [changelog](CHANGELOG.md).

## Contact

Bugs and ideas: [GitHub Issues](https://github.com/ThrallAI/thrall/issues). Anything private, including security
problems (see [SECURITY.md](SECURITY.md)): thrallAI@protonmail.com

## Licence

Thrall is open source under the [MIT licence](LICENSE). Unless you say otherwise, anything you
contribute is offered under the same licence.
