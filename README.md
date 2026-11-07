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
- **Small and local.** No cloud service, no account, no network access, no telemetry.

## Download

Each push to `main` builds both apps in GitHub Actions: open the latest
[build run](https://github.com/jmonasterio/kee-pass-j/actions/workflows/build.yml) and
download `OurLogins-windows-x64` (a single `.exe`) or `OurLogins-macos-universal` (an
`.app` for Intel and Apple silicon). The Mac app is not notarized: the first time,
right-click it and choose Open. Pushing a `v*` tag publishes both as a GitHub release.

## Features

**Everyday use**

- Sidebar filters: All accounts, Not working, Documents, folders, Recycle Bin. Folders can
  be added, renamed, coloured and deleted (with their contents, to the Recycle Bin); letter
  tiles take their folder's colour. Search is always visible (Ctrl+K).
- "Add account" screen: site, URL, username or email, password (generator prefilled),
  recovery PIN and phone, security questions, notes. If the site already exists it offers
  to replace the old entry and flags that one "not working".
- Generator: no `0 O o 1 l I | 5 S 8 B 2 Z` by default, one length slider, digits and
  symbols checkboxes.
- Notes render with optional monospace and copy one line at a time.
- Auto-Type with KeePass 2.x semantics: a global hotkey matches the current window's title,
  or type the selected entry. Default sequence `{USERNAME}{TAB}{PASSWORD}{ENTER}`, per-entry
  override, KeePass placeholder grammar. Windows `SendInput`, macOS `CGEvent` via `enigo`
  (macOS asks for the Accessibility permission once).
- Quick Access: when the hotkey finds several matching accounts or none, a small window
  opens over the current app to pick or search one; Enter types it, Ctrl+Enter copies the
  password.
- Quick unlock: after one unlock with the password, Windows Hello (PIN, fingerprint or face)
  or Touch ID unlocks the database. The password is kept encrypted with a Windows Hello key,
  or in the login Keychain on macOS (see `src-tauri/src/biometric.rs` for the trade-off).

**Documents and files**

- Documents (+ Add → Document, Ctrl+Shift+N): an entry for IDs, passports, a will or other
  papers, with a long text in Markdown (plain line breaks are kept) and attached files. The
  ⋯ menu turns an account into a document and back.
- Files on any entry: add, open in the usual app, save as, remove. Sizes are shown for each
  file and for the whole database. Adding asks first when one file is over 5 MB or all files
  together would pass 50 MB, because every save rewrites them and each of the up to 50
  backups copies them. Scans of a few MB each are fine; photo collections and videos belong
  elsewhere.
- "Open" writes a temporary copy under the app data `open` folder; it is deleted when the
  last database locks, at exit and at startup.

**Saving and backups**

- Autosave on every change. Writes are atomic (`.tmp` → fsync → rename), so a crash or a
  pulled USB drive leaves the old file intact.
- Every save also copies the file into a backup folder in the per-user app data directory:
  the newest 20 saves plus one per day for 30 days. The first save of each session first
  copies the file as it was on disk, so edits made by other apps are kept too.
- Several databases can be open at once. On launch the last ones reopen; if a file is
  missing (drive not connected) or the password is wrong, the unlock screen shows instead.

**Compatibility with other KeePass apps**

- Reads KDBX 3.1 and 4, writes KDBX 4 (the `keepass` crate writes only KDBX 4). KeePass
  2.35+ and KeePassXC 2.3+ open KDBX 4. A KDBX 3.1 file keeps a permanent
  `<name>-kdbx3-original.kdbx` copy from before its first save.
- Extra data lives in ordinary fields that other clients show and keep: "not working" is the
  custom field `KPJ_Stale = 1`, a document has `OurLogins.Kind = document`, and a folder
  colour is group custom data `OurLogins.Color`.

## Not included

Browser extension, phone app, syncing one database between several computers, a second key
for recovery, sharing, TOTP codes.

## Layout

A Tauri 2 app: a Rust backend, and a TypeScript + Fluent UI frontend in the system webview.

- `src/vault.rs`: KDBX open/save/backup, entry and group model, search, stale flag.
- `src/generator.rs`: password generator.
- `src/settings.rs`: per-user settings and recent files.
- `src/autotype.rs`: sequence grammar, keystroke injection, window matching.
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
