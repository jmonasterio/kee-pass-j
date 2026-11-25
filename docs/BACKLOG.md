# Backlog

## 2FA codes (authenticator)

Show the 6-digit login codes that apps like Google Authenticator show, inside Our Logins.

- An entry gets a **Set up 2FA** button. It reads the QR code the website shows on screen, or
  takes the text key from the site's "Can't scan?" link.
- The entry then shows the current code with a countdown and a copy button; Quick Access can
  type it.
- Store the secret where other KeePass apps read it: KeePassXC's `otp` field
  (`otpauth://` URL) and KeePass 2.47+'s `TimeOtp-Secret-Base32`, so codes also work in
  KeePassXC, KeePassDX and Strongbox.
- Guidance: at most one line under the button, e.g. "Tip: for your email account, keep 2FA on
  your phone as well." No popups or warnings.

Open question: whether this is understandable enough for users; revisit the wording before
building.

## Auto-Type: KeePass compatibility gaps

Found while adding KeePass sequence handling (commit 3c33471d).

- **Missing `<Enabled>` reads as off.** The keepass crate sets `AutoType::enabled = false` when
  an entry's `<AutoType>` block has no `<Enabled>` element. KeePass and KeePassXC always write
  it, but files from other tools may not; since we now honour the flag, the hotkey skips those
  entries. Treat a missing element as on. Needs a look at how keepass parses the block, because
  the absence is lost before we see the entry.
- **`//regex//` window patterns** are matched as a case-insensitive substring. A regex engine
  would be a new dependency. `docs/ARCHITECTURE.md` still says they are real regexes; fix that
  line either way.
- **One row per entry in the picker.** When several associations of one entry match the
  window, we use the first. KeePass lists each entry/sequence pair.
- **Association sequence errors show after our window hides**, as a toast. Errors in the entry,
  folder or Settings sequence are still reported before hiding.
- **No editor for** per-window sequences, folder default sequences, or the Auto-Type on/off
  switches. They are read from the file and kept on save.

## Supply chain

- **chrono is unmaintained** (RUSTSEC-2026-0543), ignored in `deny.toml` because keepass 0.17
  depends on it. Remove the ignore when keepass moves to jiff or time.
- **Rust 1.95 upgrades:** `windows-future` and `windows-version` 0.100 and the latest
  cargo-deny need rustc 1.95 (local is 1.93.1; `rust-version` is 1.89).
- **global-hotkey 0.10** is a new major version; not upgraded.
- **Optional trims, a few packages each:** tauri `compression` feature (removes brotli, about
  3–4 build-time packages; costs exe size, unmeasured); `tauri-plugin-dialog` replaced by `rfd`
  directly (3 packages; macOS main-thread risk); hand-written Windows clipboard instead of
  `arboard` (3 packages on Windows, about 80 lines of unsafe code).
- **Unreachable branch** in `autotype.rs` line 159 (`DanglingModifier` in `parse_items`).
