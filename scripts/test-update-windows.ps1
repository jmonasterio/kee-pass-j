# End-to-end check of the updater on Windows, against a local feed instead of GitHub.
#
#   powershell -ExecutionPolicy Bypass -File scripts\test-update-windows.ps1
#
# Needs the updater private key at %USERPROFILE%\.tauri\ourlogins-updater.key (no password).
# 1. Builds the current version's installer with the feed at http://127.0.0.1:47615/latest.json
#    and installs it for this user (%LOCALAPPDATA%\Our Logins).
# 2. Builds the same code as version <current>+1 patch, signs it, and serves it with a feed.
# 3. Starts the installed app. About 5 s after it opens, a toast offers the new version; press
#    "Update now". Pass: the app restarts and the About button (sidebar) shows the new version.
# Uninstall afterwards from Settings > Apps if the test copy is not wanted.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw "$env:USERPROFILE\.tauri\ourlogins-updater.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
$conf = Get-Content -Raw src-tauri\tauri.conf.json | ConvertFrom-Json
$old = $conf.version
$parts = $old.Split('.')
$new = "$($parts[0]).$($parts[1]).$([int]$parts[2] + 1)"
$port = 47615
if (Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue) { throw "Port $port is in use; the test feed needs it." }
$feed = "{`"plugins`":{`"updater`":{`"endpoints`":[`"http://127.0.0.1:$port/latest.json`"],`"dangerousInsecureTransportProtocol`":true}}}"
$serve = Join-Path $env:TEMP 'ourlogins-update-test'
# Files from an earlier run; an aborted run's feed server may still hold the folder open.
Get-ChildItem $serve -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force
New-Item -ItemType Directory -Force $serve | Out-Null
$nsis = 'target\release\bundle\nsis'

function Build([string]$version) {
    Remove-Item "$nsis\*" -ErrorAction SilentlyContinue
    $config = Join-Path $serve "config-$version.json"
    $feed.Replace('{"plugins"', "{`"version`":`"$version`",`"plugins`"") | Set-Content -NoNewline $config
    npx tauri build --bundles nsis --config $config
    if ($LASTEXITCODE) { throw "tauri build $version failed" }
    Get-ChildItem "$nsis\*-setup.exe" | Select-Object -First 1
}

Write-Host "Building $old (installed) ..."
$setup = Build $old
# The installer cannot replace a running copy. Close its window (the app saves on exit); copies
# run from elsewhere (the daily one in Programs\Our Logins) are left alone.
$installed = Get-Process -Name OurLogins -ErrorAction SilentlyContinue | Where-Object Path -eq "$env:LOCALAPPDATA\Our Logins\OurLogins.exe"
$installed | ForEach-Object { [void]$_.CloseMainWindow() }
$installed | Wait-Process -Timeout 15 -ErrorAction SilentlyContinue
Start-Process $setup.FullName -ArgumentList '/S' -Wait

Write-Host "Building $new (served) ..."
$setup = Build $new
Copy-Item $setup.FullName "$serve\OurLogins-windows-x64-setup.exe"
$sig = (Get-Content -Raw "$($setup.FullName).sig").Trim()
@{
    version   = $new
    pub_date  = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    notes     = ''
    platforms = @{ 'windows-x86_64' = @{ url = "http://127.0.0.1:$port/OurLogins-windows-x64-setup.exe"; signature = $sig } }
} | ConvertTo-Json -Depth 4 | Set-Content "$serve\latest.json"

$server = Start-Process python -ArgumentList '-m', 'http.server', "$port", '--bind', '127.0.0.1' -WorkingDirectory $serve -PassThru -WindowStyle Hidden
try {
    Start-Process "$env:LOCALAPPDATA\Our Logins\OurLogins.exe"
    Write-Host "Started $old. Press 'Update now' in the toast; About should then show $new."
    Read-Host 'Press Enter when done to stop the local feed'
} finally {
    Stop-Process $server
}
