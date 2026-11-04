# KeePassJ

A KDBX 4 password manager in Rust (Slint UI) for two people: one on Windows with the
database on a USB SSD, one on macOS with her own database. Files stay fully compatible
with KeePass 2.x and KeePassXC.

Older KDBX 3.1 files open normally and are rewritten as KDBX 4 on their first save
(the `keepass` crate writes only KDBX 4). KeePass 2.35+ and KeePassXC 2.3+ open KDBX 4;
older KeePass versions do not.

## v1 scope

- Open, edit, and save `.kdbx` (KDBX 4) databases. Several databases open at once, tabbed.
- On launch, reopen the last database(s). If the file is missing (USB unplugged) or the
  password is wrong, fall back to the picker/unlock screen once.
- Autosave on every change. Writes are atomic (`.tmp` → fsync → rename). Every save also
  drops a copy into a rolling backup folder in the per-user app data directory (off the
  USB): the newest 20 saves plus one per day for 30 days.
- Group tree, entry list, always-visible search box, view/edit/add/delete entries.
  Notes render readably with optional monospace and per-line copy.
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

## Not in v1

Browser extension, phone, a database shared between two people, second key for
recovery, web sharing, agent sharing, TOTP, Windows Hello / Touch ID, SQLite.

## Layout

- `src/vault.rs` — KDBX open/save/backup, entry and group model, search, stale flag.
- `src/generator.rs` — password generator.
- `src/settings.rs` — per-user settings and recent files.
- `src/autotype.rs` — sequence grammar, keystroke injection, window matching.
- `src/main.rs` — Slint UI and wiring.
- `ui/*.slint` — UI definitions.
- `docs/ARCHITECTURE.md` — module contracts.

## Build

```
cargo build --release
```
