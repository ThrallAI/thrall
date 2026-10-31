# Building Thrall from source

These are the same steps the release workflow (`.github/workflows/release.yml`) runs, so if they
stop working, the release build breaks too and it gets fixed.

## What you need

- **Rust** 1.95 or newer: [rustup.rs](https://rustup.rs).
- **Node.js** 22 or newer (builds the web app, which is built into the engine).
- **Linux only**, the desktop app's window and tray libraries:
  - Ubuntu / Debian: `sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev build-essential`
  - Arch / Omarchy: `sudo pacman -S --needed webkit2gtk-4.1 libayatana-appindicator librsvg xdotool openssl base-devel`
- **macOS:** Xcode's command line tools (`xcode-select --install`).
- **Windows:** the Visual Studio C++ build tools (rustup offers to install them).
- For the phone app only: JDK 17+ and the Android SDK (Android Studio installs both).

## The engine and terminal app

```sh
cd web && npm ci && npm run build && cd ..
cargo build --release -p thrall
```

The program is `target/release/thrall` (`thrall.exe` on Windows). Then:

```sh
thrall key set openrouter   # paste your OpenRouter key; it goes in the system keychain
thrall                      # the terminal app (starts the engine)
thrall engine               # or just the engine; the web app is at the address it prints
```

## The desktop app and an installer

Build the engine as above, then put it beside the desktop app under the name Tauri expects
(`thrall-<target triple>`):

```sh
# The target triple of this computer, e.g. x86_64-unknown-linux-gnu or aarch64-apple-darwin
TRIPLE=$(rustc -vV | sed -n 's/host: //p')
mkdir -p desktop/binaries
cp target/release/thrall desktop/binaries/thrall-$TRIPLE        # thrall.exe → thrall-$TRIPLE.exe on Windows

cd desktop
npm ci
npx tauri build --config tauri.bundle.json                      # Windows: the .exe installer
npx tauri build --config tauri.bundle.json --bundles deb,appimage   # Linux
npx tauri build --config tauri.bundle.json --bundles dmg        # macOS
```

The installers land in `target/release/bundle/`. For a quick look without an installer,
`cargo run -p thrall-desktop` starts the desktop app (it finds the engine beside itself, or set
`THRALL_EXE` to its path).

## The phone app

```sh
cd android
./gradlew assembleDebug     # app/build/outputs/apk/debug/app-debug.apk
```

A release build needs a signing key of your own (see `android/app/build.gradle.kts`); an app signed
with a different key can't update one signed with another.

## Checks before a pull request

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd web && npm run typecheck && npm test
cd android && ./gradlew testDebugUnitTest
```

On Windows, `scripts/check.ps1` runs all of them (and `cargo deny check`).
