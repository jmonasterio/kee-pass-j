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
