# Known issues

Things we know aren't right yet, or that work differently from what you might expect. If you hit
something that isn't here, please open an [issue](https://github.com/ThrallAI/thrall/issues) with
the relevant lines from the `logs` folder (keys removed). Fixed items move to the
[changelog](CHANGELOG.md).

## Installing

- **Not code-signed yet.** Windows SmartScreen says "Windows protected your PC" (click **More
  info**, then **Run anyway**), and macOS needs **Open Anyway** in System Settings → Privacy &
  Security. Check the download against `SHA256SUMS.txt` from the release if you want to be sure
  it's ours.
- **macOS hasn't been tried by hand.** The builds are made and tested automatically, but nobody
  has used them day to day yet. Reports are very welcome.
- **The Android app isn't on the Play Store.** Install the `.apk` from the release (allow "Install
  unknown apps" for your browser or file manager).

## The phone

- **Away from home needs Tailscale.** The phone reaches your computer over your home network; for
  anywhere else, use [Tailscale](https://tailscale.com) on both (see
  [Away from home](README.md#away-from-home)).
- **Some things are only in the desktop app**, such as editing skills and lessons. Typing a
  terminal-only command like `/settings` on the phone tells you where its version is.

## Models and providers

- **Local models cover chat only.** With a `local/` model, pictures, speech and transcription
  still go through OpenRouter, so they need a key.
- **How well Thrall works depends on the model.** Some cheaper models stop early, skip steps or
  call tools wrongly in long tasks. If a model keeps going wrong, try another in Settings → Models.

## Windows

- **A command can show "!" although it worked.** In Windows PowerShell 5.1, a program that writes
  a normal message to its error output (pip's cache notice, git's progress) can be reported as
  failed when the model adds `2>&1`. Thrall is told not to do this and usually reads the output
  correctly anyway; the step's details show what really happened.
- **The desktop window uses more memory the longer it's open** (several hundred MB after a day,
  even while hidden in the tray). Closing the window to the tray doesn't free it yet; quitting
  and reopening does.

## Safety checks

- **Installs are checked by their command, not by what a project file asks for.** An install from
  somewhere other than the official place (a web or git address, another index, a package file, a
  local folder, or a requirements file naming one) always asks, even in Full auto. But a plain
  `npm install` in a project whose `package.json` lists git dependencies runs like any other
  install.
- **Commands run as you.** Thrall's risk checks look for the usual risky commands; a script it
  writes and then runs can do anything you can. Read what you approve, and use a throwaway folder
  while you get to know it.

## Not in this version

- Running Thrall as a server for people outside your home or team network, with accounts.
- Telegram, Discord and Slack.
