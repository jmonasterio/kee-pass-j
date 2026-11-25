# Notes for agents

## Download page (keypass.argw.com)

The page is `site/` in this repo (moved from `C:\github\cf-static-sites` on 2026-11-25). It is
the Cloudflare Pages project `keypass-argw-com`, published by local wrangler:
`C:/bin/git/bin/bash.exe scripts/deploy-site.sh` (git-bash, not WSL `bash`; needs
`../cf-infra/.env`).

Its links use `releases/latest/download/<file>` and it reads the version from the GitHub API,
so a release needs no page change. Edit and deploy it when release file names change; the
`site-links` CI job fails when a page link no longer resolves.
