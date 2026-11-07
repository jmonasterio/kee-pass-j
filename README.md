# Our Logins

A KDBX 4 password manager in Rust for two people: one on Windows with the database on a USB
SSD, one on macOS with her own database. Files stay fully compatible with KeePass 2.x and
KeePassXC. The window is a Tauri 2 app: a Rust backend with a TypeScript + Fluent UI web
frontend in the system webview.

Older KDBX 3.1 files open normally and are rewritten as KDBX 4 on their first save
(the `keepass` crate writes only KDBX 4). KeePass 2.35+ and KeePassXC 2.3+ open KDBX 4;
older KeePass versions do not.

## Features

- Open, edit, and save `.kdbx` (KDBX 4) databases. Several databases open at once, tabbed.
- On launch, reopen the last database(s). If the file is missing (USB unplugged) or the
  password is wrong, fall back to the picker/unlock screen once.
- Autosave on every change. Writes are atomic (`.tmp` → fsync → rename). Every save also
  drops a copy into a rolling backup folder in the per-user app data directory (off the
  USB): the newest 20 saves plus one per day for 30 days. The first save of each session
  first copies the file as it was on disk, so edits made by other apps are kept too. A
  KDBX 3.1 file also keeps a permanent `<name>-kdbx3-original.kdbx` copy.
- Sidebar filters: All accounts (the default), Not working, Documents, folders, Recycle Bin.
  Folders can be added, coloured and deleted (with their contents, to the Recycle Bin);
  letter tiles take their folder's colour (Settings can turn that off). Always-visible
  search box, view/edit/add/delete entries. Notes render readably with optional monospace
  and per-line copy.
- Documents (+ Add → Document, Ctrl+Shift+N): an entry for IDs, passports, a will or other
  papers, with a long text in Markdown (plain line breaks are kept) and attached files.
  Stored as an ordinary entry with the string field `OurLogins.Kind = document`, so other
  KeePass clients show it as a normal entry with notes and attachments. The overflow menu
  turns an account into a document and back.
- Files on any entry: add (shows the size; asks first when one file is over 5 MB or all
  files together would pass 50 MB, because every save rewrites them and each of the up to 50
  backups copies them), open in the usual app (a temporary copy under the app data `open`
  folder, deleted when the last database locks, at exit and at startup), save as, remove.
- About (sidebar): version, the project's goals, link to the source on GitHub.
- "Add account" capture screen: site, URL, username/email, password (generator
  prefilled), recovery PIN/phone, security questions, notes. Target: under ten seconds
  to record a new account. If the site already exists it offers to replace the old entry
  and flags it stale.
- Stale flag: entries marked "not working" are visibly flagged and sorted to the bottom.
  Stored as the custom field `KPJ_Stale = 1`, so other KeePass clients still open the file.
- Generator: ambiguity-free alphabet by default (no `0 O o 1 l I | 5 S 8 B 2 Z`), one
  length slider, digits and symbols checkboxes. Passwords always displayed in a large
  monospace font.
- Auto-Type with KeePass 2.x semantics: global hotkey with window-title matching, and
  select-entry-then-type. Default sequence `{USERNAME}{TAB}{PASSWORD}{ENTER}`, per-entry
  override, KeePass placeholder grammar. Windows `SendInput`, macOS `CGEvent` via `enigo`
  (macOS asks for the Accessibility permission once).
- Quick Access: when the hotkey finds several matching accounts or none, a small window
  opens over the current app to pick or search one; Enter types it, Ctrl+Enter copies the
  password.
- Quick unlock: after one unlock with the password, Windows Hello (PIN, fingerprint or face)
  or Touch ID unlocks the database. The password is kept encrypted with a Windows Hello key,
  or in the login Keychain on macOS (see `src-tauri/src/biometric.rs` for the trade-off).

## Not included

Browser extension, phone, a database shared between two people, second key for
recovery, web sharing, agent sharing, TOTP, SQLite.

## Layout

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
