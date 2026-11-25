#!/usr/bin/env bash
# Publishes site/ (the keypass.argw.com download page) to the Cloudflare Pages project
# keypass-argw-com.
#
#   C:/bin/git/bin/bash.exe scripts/deploy-site.sh
#
# Run it with git-bash: on this Windows box the bare `bash` is the WSL launcher, which cannot
# run the Windows `node`. Reads CLOUDFLARE_API_TOKEN / CLOUDFLARE_ACCOUNT_ID from
# ../cf-infra/.env (override with CF_INFRA_DIR).
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INFRA_DIR="${CF_INFRA_DIR:-$REPO/../cf-infra}"
[ -f "$INFRA_DIR/.env" ] || { echo "error: $INFRA_DIR/.env not found. Set CF_INFRA_DIR." >&2; exit 1; }
set -a; source "$INFRA_DIR/.env"; set +a

# Node prefers IPv6, which fails on this LAN and makes the first wrangler calls fail. Force
# IPv4 and retry (see cf-infra/AGENTS.md).
export NODE_OPTIONS=--dns-result-order=ipv4first
for i in 1 2 3 4 5; do
  npx wrangler pages deploy "$REPO/site" --project-name=keypass-argw-com --branch=main --commit-dirty=true && exit 0
  echo "attempt $i failed; retrying" >&2
  sleep 2
done
echo "error: deploy failed after 5 attempts" >&2
exit 1
