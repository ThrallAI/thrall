# Builds the signed phone app and offers it to paired phones as an update.
# Usage: powershell -File scripts/release-android.ps1 -Notes "App lock, new folders"
#
# The APK goes to Thrall's data folder (phone-app\, with release.json describing it), where the
# engine offers it (`app.latest` / `app.get`), and a copy to Downloads for sending by hand. Raise
# versionCode in android/app/build.gradle.kts first: a release that isn't newer is refused,
# because phones only take an update newer than what they have.
param(
    [string]$Notes = "",
    # Where Thrall keeps its data (the engine's --home); default %APPDATA%\Thrall.
    [string]$DataDir = (Join-Path $env:APPDATA "Thrall")
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot)

$props = @{}
Get-Content android/local.properties | Where-Object { $_ -match '^\s*([\w.]+)\s*=\s*(.*)$' } |
    ForEach-Object { $props[$matches[1]] = $matches[2] -replace '\\(.)', '$1' }
if (-not $props['thrall.signing.file'] -or -not (Test-Path $props['thrall.signing.file'])) {
    throw "No release key: set thrall.signing.file and thrall.signing.password in android/local.properties."
}
if ($props['java.home']) { $env:JAVA_HOME = $props['java.home'] }
if ($props['gradle.user.home']) { $env:GRADLE_USER_HOME = $props['gradle.user.home'] }
$sdk = if ($props['sdk.dir']) { $props['sdk.dir'] } else { $env:ANDROID_HOME }

Push-Location android
try {
    $ErrorActionPreference = "Continue"
    # Unit tests only exist for debug builds here (same code), then the signed release build.
    .\gradlew.bat testDebugUnitTest assembleRelease --console=plain --quiet
    if ($LASTEXITCODE -ne 0) { throw "The release build failed." }
} finally {
    $ErrorActionPreference = "Stop"
    Pop-Location
}
$apk = "android/app/build/outputs/apk/release/app-release.apk"

$tools = Get-ChildItem (Join-Path $sdk "build-tools") | Sort-Object { [version]($_.Name -replace '[^\d.].*$', '') } | Select-Object -Last 1
$badging = & (Join-Path $tools.FullName "aapt2.exe") dump badging $apk | Select-String "^package:"
if ($badging -notmatch "versionCode='(\d+)' versionName='([^']+)'") { throw "Couldn't read the APK's version." }
$code = [int]$matches[1]
$name = $matches[2]
$signer = cmd /c "`"$(Join-Path $tools.FullName 'apksigner.bat')`" verify --print-certs $apk 2>&1" | Select-String "certificate DN"
if ("$signer" -notmatch "CN=Thrall") { throw "The APK isn't signed with Thrall's release key: $signer" }

$dir = Join-Path $DataDir "phone-app"
New-Item -ItemType Directory -Force $dir | Out-Null
$releaseFile = Join-Path $dir "release.json"
if (Test-Path $releaseFile) {
    $old = Get-Content $releaseFile -Raw | ConvertFrom-Json
    if ($code -le [int]$old.version_code) {
        throw "versionCode $code isn't newer than the release already offered ($($old.version_code), $($old.version_name)). Raise it in android/app/build.gradle.kts."
    }
}

$file = "Thrall-$name.apk"
Copy-Item $apk (Join-Path $dir $file) -Force
Get-ChildItem $dir -Filter *.apk | Where-Object Name -ne $file | Remove-Item
$sha = (Get-FileHash (Join-Path $dir $file) -Algorithm SHA256).Hash.ToLower()
$json = [ordered]@{ version_code = $code; version_name = $name; file = $file; sha256 = $sha; notes = $Notes } |
    ConvertTo-Json
# No byte-order mark: the engine reads it as plain JSON.
[IO.File]::WriteAllText($releaseFile, $json, (New-Object Text.UTF8Encoding $false))

$downloads = Join-Path $env:USERPROFILE "Downloads\Thrall-for-Android-$name.apk"
Copy-Item $apk $downloads -Force
Write-Host "Released $name (versionCode $code): paired phones are offered it now."
Write-Host "Copy to send by hand: $downloads"
