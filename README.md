# Our Logins

A desktop password manager for a household's logins and important papers, stored in
standard KeePass (`.kdbx`) files. It runs on Windows and macOS, and the files keep opening in
KeePass 2, KeePassXC and the KeePass phone apps, so nothing is locked into this program.

## Goals

- **Record a new account in under ten seconds.** One screen for site, username and
  password, with a generated password already filled in.
- **Keep old accounts instead of losing them.** An account that stopped working is flagged
  "not working" and sorted to the bottom; it is not deleted.
- **Never lose an edit.** Every change is saved at once, with rolling backups kept on a
  different drive from the database.
- **Readable passwords.** The generator leaves out look-alike characters, and passwords are
  shown in a large monospace font.
- **Type logins into other apps with one hotkey** (Auto-Type and Quick Access).
- **Keep the papers that matter next to the passwords:** IDs, passports, a will, insurance
  documents, with notes in Markdown.
- **Small and local.** No cloud service, no account, no telemetry. The only network use is
  checking GitHub for a newer version.

## Download

Each push to `main` builds both apps in GitHub Actions: open the latest
[build run](https://github.com/jmonasterio/kee-pass-j/actions/workflows/build.yml) and
download `OurLogins-windows-x64` (a single `.exe`) or `OurLogins-macos-universal` (an
`.app` for Intel and Apple silicon). The Mac app is not notarized: the first time,
right-click it and choose Open. Pushing a `v*` tag publishes both as a GitHub release.

Windows: `scripts\install-windows.ps1` copies the newest downloaded
`OurLogins-windows-x64*.exe` (Downloads or Desktop) to
`%LOCALAPPDATA%\Programs\Our Logins\OurLogins.exe` and points every Our Logins shortcut
(taskbar pin, Start menu, Desktop), including ones aimed at a local `target\release` build,
at that copy. Run it again after each download:
`powershell -ExecutionPolicy Bypass -File scripts\install-windows.ps1`.

## Updates

About 5 s after it starts, the app reads `latest.json` from the newest GitHub release. If that
release has a newer version, a toast offers "Update now", and the About dialog has
"Check for updates". Updating saves every open database, installs the new version and restarts
the app. Every package is signed with the updater key, and the app refuses a package whose
signature does not match the public key in `src-tauri/tauri.conf.json`.

- Windows: updates install through `OurLogins-windows-x64-setup.exe` (per user, no admin
  prompt, into `%LOCALAPPDATA%\Our Logins`). A copy run from the single `.exe` also gets the
  offer; the update then installs a second copy in that folder.
- macOS: the `.app` is replaced in place from `OurLogins-macos-universal.app.tar.gz`. The app
  must sit in a folder the user can write to (`/Applications` for an admin user).
- Linux: only the AppImage updates itself; `.deb` installs report an error.

The private key is `%USERPROFILE%\.tauri\ourlogins-updater.key` on the build machine and the
`TAURI_SIGNING_PRIVATE_KEY` repository secret. Keep a backup: without it, installed copies
cannot be updated again. To change the key, put the new public key in `tauri.conf.json` and
re-sign `src-tauri/tests/fixtures/signed.txt`
(`npx tauri signer sign -f <key> -p "" src-tauri/tests/fixtures/signed.txt`).

Test an update end to end on Windows: `scripts\test-update-windows.ps1`.

## Features

**Everyday use**

- Sidebar filters: All accounts, Not working, Documents, folders, Recycle Bin. Folders can
  be added, renamed, colored and deleted (with their contents, to the Recycle Bin); two
  folders side by side cannot have the same name (ignoring case). Letter tiles take their
  folder's color. Search is always visible (Ctrl+K).
- Add account (+ Add, Ctrl+N) opens a blank account in the details pane: site, URL,
  username or email, password (generator prefilled), recovery PIN and phone, security
  questions, notes. If the site already exists it offers to replace the old entry and flags
  that one "not working". Nothing is written until Save.
- Editing happens in the details pane: hover a field and click its pencil, double-click it,
  or press F2. Changes wait under an "Unsaved changes" bar until Save (Ctrl+S) or Discard;
  Esc puts back the field being typed in. Leaving the entry, locking or closing the window
  asks Save / Discard / Stay. Each save keeps the previous version in the entry's history,
  and the "Saved" message offers Undo for a few seconds.
- History: every entry keeps its earlier versions (10 by default, set per database from its
  ⋯ menu). "History" in the entry's ⋯ menu, or a click on its title, lists them with what
  changed; a version shows what differs from now, and can be restored (with Undo) or
  deleted. Files stay with the versions that had them.
- Generator: no `0 O o 1 l I | 5 S 8 B 2 Z` by default, one length slider, digits and
  symbols checkboxes.
- Notes render with optional monospace and copy one line at a time.
- Auto-Type with KeePass 2.x semantics: a global hotkey matches the current window's title
  (by entry title, URL host or window pattern), or type the selected entry. Default sequence
  `{USERNAME}{TAB}{PASSWORD}{ENTER}`, overridden as in KeePass by a matching window
  association's sequence, the entry's sequence, or its folder's default sequence; KeePass's
  Auto-Type on/off switches on entries and folders are respected. KeePass placeholder grammar
  including `{CLEARFIELD}` and `~` for Enter. The default sequence looks at the focused field first (Windows
  UI Automation, macOS Accessibility): it replaces text already in a field, skips a username
  that is already filled in, and types only the password into a password box. Like KeePass,
  it lets go of the hotkey's modifier keys before typing and stops if another window comes
  to the front. Windows: `SendInput` called directly; macOS: `CGEvent` via `enigo` (macOS
  asks for the Accessibility permission once).
- Copying a password (or anything else) keeps it out of Windows clipboard history (Win+V)
  and cloud clipboard sync; on macOS it carries the marker clipboard-history apps honour.
  The clipboard is emptied 20 seconds later (Settings: Never to 120 s), and when Our Logins
  quits, unless something else was copied meanwhile.
- KeePass shortcuts on the selected entry: Ctrl+B copies the username, Ctrl+C the password,
  Ctrl+U opens the website.
- Quick Access: when the hotkey finds several matching accounts or none, a small window
  opens over the current app to pick or search one; Enter types it, Shift+Enter types only
  the password, Ctrl+Enter copies the password.
- Quick unlock: after one unlock with the password, Windows Hello (PIN, fingerprint or face)
  or Touch ID unlocks the database. The password is kept encrypted with a Windows Hello key,
  or in the login Keychain on macOS (see `src-tauri/src/biometric.rs` for the trade-off).
- Auto-lock: every open database locks after a time without use (Settings: Never to 60
  minutes, default 15), and when the computer locks or goes to sleep (Windows and macOS).
  Unsaved edits in the details pane are saved first, and a copied password is cleared.
- Change master password from the database's ⋯ menu (the key file stays the same).
  Password boxes have a show/hide button.

**Documents and files**

- Documents (+ Add → Document, Ctrl+Shift+N): an entry for IDs, passports, a will or other
  papers, with a long text in Markdown (plain line breaks are kept) and attached files.
  Files picked while adding or editing (several at once) are attached when it is saved. The
  ⋯ menu turns an account into a document and back.
- Files on any entry: add, open in the usual app, save as, remove (adding and removing wait
  for Save, like other edits). Sizes are shown for each file. Adding asks first when one
  file is over 5 MB or all files together would pass 50 MB, showing the database's total
  file sizes, because every save rewrites them and each of the up to 50 backups copies
  them. Scans of a few MB each are fine; photo collections and videos belong elsewhere.
- "Open" writes a temporary copy under the app data `open` folder; it is deleted when the
  last database locks, at exit and at startup.

**Saving and backups**

- Autosave on every change, in the background: the window stays usable and the database
  shows "Saving…" until the file is written. Writes are atomic (`.tmp` → fsync → rename),
  so a crash or a pulled USB drive leaves the old file intact. Locking or quitting waits
  for the save; if it fails, the window says so and offers Retry.
- Every save encrypts the whole file (KDBX has no partial updates), so its time grows with
  the file size: about 1.2 s for 13.7 MB in a debug build, plus the network write when the
  file is on a share.
- Every save also copies the file into a backup folder in the per-user app data directory:
  the newest 20 saves plus one per day for 30 days. The first save of each session first
  copies the file as it was on disk, so edits made by other apps are kept too.
- Several databases can be open at once; the sidebar then names the one the lists belong
  to ("In …"). On launch the last ones reopen; if a file is missing (drive not connected)
  or the password is wrong, the unlock screen shows instead.

**Compatibility with other KeePass apps**

- Reads KDBX 3.1 and 4, writes KDBX 4 (the `keepass` crate writes only KDBX 4). KeePass
  2.35+ and KeePassXC 2.3+ open KDBX 4. A KDBX 3.1 file keeps a permanent
  `<name>-kdbx3-original.kdbx` copy from before its first save.
- Extra data lives in ordinary fields that other clients show and keep: "not working" is the
  custom field `KPJ_Stale = 1`, a document has `OurLogins.Kind = document`, and a folder
  color is group custom data `OurLogins.Color`.

## Not included

Browser extension, phone app, syncing one database between several computers, a second key
for recovery, sharing, TOTP codes.

## Layout

A Tauri 2 app: a Rust backend, and a TypeScript + Fluent UI frontend in the system webview.

- `src/vault.rs`: KDBX open/save/backup, entry and group model, search, stale flag.
- `src/generator.rs`: password generator.
- `src/settings.rs`: per-user settings and recent files.
- `src/autotype.rs`: sequence grammar, typing a sequence, window matching.
- `src/input.rs`: keystrokes, the foreground window and the focused field (Win32 and UI
  Automation on Windows; `enigo`, AppKit, Core Graphics and Accessibility on macOS).
- `src-tauri/`: the desktop app (Tauri commands, app state, hotkey and Auto-Type wiring).
- `web/`: the frontend (vanilla TypeScript, Fluent UI Web Components, CSS).
- `docs/ARCHITECTURE.md`: module contracts and the webview security rules.

## Build

Needs Rust and Node 22.

```
npm ci
npm run build                       # web/src -> web/dist
cargo build --release -p ourlogins-app
```

Windows: the result is one file, `target/release/OurLogins.exe`, with the frontend compiled
in. It uses the WebView2 runtime that ships with Windows 11 and current Windows 10.

macOS (universal `.app`, needs macOS 12 or later):

```
npx tauri build --target universal-apple-darwin --bundles app
```

Checks: `cargo test -p ourlogins --lib` and `npm run check` (TypeScript).

Up to version 0.2 the app was called KeePassJ. On first start it moves the old settings and
backups folder (`logonpro\KeePassJ` / `com.logonpro.KeePassJ`) to the new `OurLogins` one.

## CI builds and releases

`.github/workflows/build.yml` builds and tests on Linux, Windows and macOS for every push
to `main`, every pull request, and every `v*` tag. Only a tag publishes a release, so
merging a fix to `main` does not change what the Download links serve.

Publish a release: bump the version in `Cargo.toml`, `src-tauri/Cargo.toml` and
`src-tauri/tauri.conf.json`, run `cargo update -w`, commit, then

```
git tag v0.3.2
git push origin main v0.3.2
```

The `release` job runs only when all three platform jobs pass. It also publishes
`latest.json` (`scripts/update-feed.mjs`), which makes installed apps offer the new version.

Force a build without a new commit:

```
gh workflow run build.yml --ref main          # any branch; never publishes a release
gh run rerun <run-id> --failed                # retry failed or cancelled jobs, e.g. macOS
gh run list --workflow build.yml -L 5         # find <run-id>
```

Rerunning a `v*` tag's run creates that release, or uploads its files to the existing one,
once all three platform jobs pass.
