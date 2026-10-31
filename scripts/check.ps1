# Runs every check a phase must pass before it is committed.
# Usage: powershell -File scripts/check.ps1
# Each step is judged by its exit code; "Stop" would turn tools' stderr warnings into failures.
$ErrorActionPreference = "Continue"
Set-Location (Split-Path $PSScriptRoot)

# The web app is built first, so the Rust build embeds the real thing. "Web types" fails when
# web/src/protocol.ts was stale against docs/protocol.schema.json (it regenerates it, so a second
# run passes).
$steps = @(
    @{ Name = "Web install";  Cmd = { npm ci --prefix web --no-audit --no-fund } },
    @{ Name = "Web types";    Cmd = {
        $before = (Get-FileHash web/src/protocol.ts).Hash
        npm run --prefix web gen:types
        if ((Get-FileHash web/src/protocol.ts).Hash -ne $before) {
            Write-Host "web/src/protocol.ts was out of date with docs/protocol.schema.json (now regenerated)."
            cmd /c exit 1
        } else { cmd /c exit 0 }
    } },
    # Compiled .js next to the .tsx sources (from a stray `tsc -b`) would be bundled instead of the
    # real sources, since the bundler prefers .js.
    @{ Name = "Web sources";  Cmd = {
        $stray = Get-ChildItem web/src -Recurse -Include *.js, *.jsx
        if ($stray) {
            Write-Host "Compiled files in web/src (delete them; type-check with npm run typecheck):"
            $stray | ForEach-Object { Write-Host "  $($_.FullName)" }
            cmd /c exit 1
        } else { cmd /c exit 0 }
    } },
    @{ Name = "Web check";    Cmd = { npm run --prefix web typecheck } },
    @{ Name = "Web tests";    Cmd = { npm run --prefix web test } },
    @{ Name = "Web licences"; Cmd = { npm run --prefix web licences } },
    @{ Name = "Web build";    Cmd = { npm run --prefix web build } },
    @{ Name = "Format";       Cmd = { cargo fmt --all -- --check } },
    @{ Name = "Lint";         Cmd = { cargo clippy --workspace --all-targets -- -D warnings } },
    @{ Name = "Tests";        Cmd = { cargo test --workspace } },
    @{ Name = "Build";        Cmd = { cargo build --workspace } },
    @{ Name = "Licences";     Cmd = { cargo deny check } },
    # The Android app: unit tests (Noise against the engine's vectors, protocol drift against the
    # schema) and a debug build. Needs the Android SDK; machine paths come from
    # android/local.properties (sdk.dir, and optionally java.home and gradle.user.home).
    @{ Name = "Android";      Cmd = {
        $props = @{}
        if (Test-Path android/local.properties) {
            Get-Content android/local.properties | Where-Object { $_ -match '^\s*([\w.]+)\s*=\s*(.*)$' } |
                ForEach-Object { $props[$matches[1]] = $matches[2] -replace '\\(.)', '$1' }
        }
        if (-not $props['sdk.dir'] -and -not $env:ANDROID_HOME) {
            Write-Host "No Android SDK (set sdk.dir in android/local.properties)."
            cmd /c exit 1
            return
        }
        if ($props['java.home']) { $env:JAVA_HOME = $props['java.home'] }
        if ($props['gradle.user.home']) { $env:GRADLE_USER_HOME = $props['gradle.user.home'] }
        Push-Location android
        .\gradlew.bat testDebugUnitTest assembleDebug --console=plain --quiet
        $code = $LASTEXITCODE
        Pop-Location
        cmd /c exit $code
    } }
)

$failed = @()
foreach ($step in $steps) {
    Write-Host "== $($step.Name)" -ForegroundColor Cyan
    & $step.Cmd
    if ($LASTEXITCODE -ne 0) { $failed += $step.Name }
}

if ($failed.Count -gt 0) {
    Write-Host "FAILED: $($failed -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host "All checks passed." -ForegroundColor Green
