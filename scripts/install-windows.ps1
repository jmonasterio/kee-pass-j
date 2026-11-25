# Installs a downloaded Our Logins release for the current user and points every shortcut at it.
#
#   powershell -ExecutionPolicy Bypass -File scripts\install-windows.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\install-windows.ps1 -Exe C:\path\OurLogins-windows-x64.exe
#
# Without -Exe it takes the newest OurLogins-windows-x64*.exe from Downloads or the Desktop.
# The program is copied to %LOCALAPPDATA%\Programs\Our Logins\OurLogins.exe, so a later
# download only needs this script run again. Taskbar pins, Start menu and Desktop shortcuts
# whose target is any OurLogins.exe / KeePassJ.exe (for example a local target\release build)
# are repointed there, and a Start menu shortcut is created if none exists.
param([string]$Exe)
$ErrorActionPreference = 'Stop'

if (-not $Exe) {
    # Skips the -setup.exe installer: a separate download that installs and updates on its own.
    $candidates = "$env:USERPROFILE\Downloads", "$env:USERPROFILE\Desktop", "$env:USERPROFILE\OneDrive\Desktop" |
        Where-Object { Test-Path $_ } |
        ForEach-Object { Get-ChildItem $_ -Filter 'OurLogins-windows-x64*.exe' -ErrorAction SilentlyContinue } |
        Where-Object { $_.Name -notlike '*-setup.exe' }
    $newest = $candidates | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $newest) { throw 'No OurLogins-windows-x64*.exe in Downloads or on the Desktop. Pass -Exe <path>.' }
    $Exe = $newest.FullName
}
$Exe = (Resolve-Path $Exe).Path

$dir = Join-Path $env:LOCALAPPDATA 'Programs\Our Logins'
$target = Join-Path $dir 'OurLogins.exe'

# A running copy holds the file open. Close its window (the app saves on exit), then wait.
$running = Get-Process -Name OurLogins -ErrorAction SilentlyContinue
if ($running) {
    Write-Host 'Closing the running Our Logins (it saves on exit)...'
    $running | ForEach-Object { [void]$_.CloseMainWindow() }
    $running | Wait-Process -Timeout 15 -ErrorAction SilentlyContinue
    if (Get-Process -Name OurLogins -ErrorAction SilentlyContinue) {
        throw 'Our Logins is still running. Close it (save or discard any changes), then run this again.'
    }
}

New-Item -ItemType Directory -Force -Path $dir | Out-Null
if ($Exe -ne $target) { Copy-Item $Exe $target -Force }
Unblock-File $target   # drop the "downloaded from the internet" mark so SmartScreen asks once at most
Write-Host "Installed $Exe -> $target"

$shell = New-Object -ComObject WScript.Shell
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$places = (Join-Path $env:APPDATA 'Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar'),
          $startMenu, "$env:USERPROFILE\Desktop", "$env:USERPROFILE\OneDrive\Desktop" | Where-Object { Test-Path $_ }
$changed = 0
foreach ($place in $places) {
    foreach ($file in Get-ChildItem $place -Filter *.lnk -Recurse -ErrorAction SilentlyContinue) {
        $link = $shell.CreateShortcut($file.FullName)
        if (-not $link.TargetPath -or (Split-Path $link.TargetPath -Leaf) -notmatch '^(OurLogins|KeePassJ)(-windows-x64.*)?\.exe$') { continue }
        if ($link.TargetPath -eq $target) { continue }
        Write-Host "  $($file.FullName)`n    was $($link.TargetPath)"
        $link.TargetPath = $target
        $link.WorkingDirectory = $dir
        $link.IconLocation = "$target,0"
        $link.Save()
        $changed++
    }
}

$menuLink = Join-Path $startMenu 'Our Logins.lnk'
if (-not (Test-Path $menuLink)) {
    $link = $shell.CreateShortcut($menuLink)
    $link.TargetPath = $target
    $link.WorkingDirectory = $dir
    $link.IconLocation = "$target,0"
    $link.Save()
    Write-Host "  created $menuLink"
}

Write-Host "$changed shortcut(s) repointed."
if ($changed -gt 0) {
    Write-Host 'Taskbar pins can keep the old icon until Explorer restarts: sign out and in, or run'
    Write-Host '  Stop-Process -Name explorer   (Windows starts it again)'
}
