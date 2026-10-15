#!/bin/sh
# One-time Cloudflare setup, after `wrangler login` (free account, no card):
#
#   scripts/cloudflare-setup.sh [project-name]     (default: playzeek)
#
# 1. Creates the Pages project (production branch `main`). If the name is
#    taken, Pages gives the project a *.pages.dev address with a few extra
#    characters; the script reads the real address back.
# 2. Creates the Workers KV namespace that holds the tunnel hostnames.
# 3. Writes apps/web/wrangler.toml (KV binding) and apps/web/.zeek-deploy.
# 4. Creates .secrets/proxy-secret (if missing) and stores it as the Pages
#    secret ZEEK_PROXY_SECRET.
#
# Then run scripts/deploy-web.sh and scripts/zeek-up.sh.
set -eu
. "$(dirname "$0")/zeek-env.sh"

PROJECT=${1:-playzeek}
cd "$ZEEK_ROOT/apps/web"

"$WRANGLER" whoami >/dev/null 2>&1 || fail "not logged in: run (in apps/web) pnpm exec wrangler login"

# The pages.dev subdomain of project $1, from `pages project list --json`.
subdomain_of() {
  "$WRANGLER" pages project list --json 2>/dev/null | node -e '
    let raw = "";
    process.stdin.on("data", (c) => (raw += c));
    process.stdin.on("end", () => {
      const entry = JSON.parse(raw).find((p) => Object.values(p).includes(process.argv[1]));
      const match = entry && JSON.stringify(entry).match(/([a-z0-9-]+)\.pages\.dev/);
      if (match) console.log(match[1]);
    });' "$1"
}

# The id of KV namespace $1, from `kv namespace list` (JSON).
namespace_id() {
  "$WRANGLER" kv namespace list 2>/dev/null | node -e '
    let raw = "";
    process.stdin.on("data", (c) => (raw += c));
    process.stdin.on("end", () => {
      const ns = JSON.parse(raw).find((n) => n.title === process.argv[1]);
      if (ns) console.log(ns.id);
    });' "$1"
}

if [ -n "$(subdomain_of "$PROJECT")" ]; then
  say "Pages project $PROJECT already exists."
else
  "$WRANGLER" pages project create "$PROJECT" --production-branch main
fi
SUBDOMAIN=$(subdomain_of "$PROJECT")
[ -n "$SUBDOMAIN" ] || fail "could not read the project's pages.dev address"

NAMESPACE=$(namespace_id zeek-tunnels)
if [ -z "$NAMESPACE" ]; then
  "$WRANGLER" kv namespace create zeek-tunnels >/dev/null
  NAMESPACE=$(namespace_id zeek-tunnels)
fi
[ -n "$NAMESPACE" ] || fail "could not create the KV namespace"

cat >wrangler.toml <<EOF
# Cloudflare Pages project for the public Zeek site (written by
# scripts/cloudflare-setup.sh). The KV namespace holds the current quick
# tunnel hostnames; see functions/_lib/proxy.ts.
name = "$PROJECT"
pages_build_output_dir = "./dist"
compatibility_date = "2026-10-01"

[[kv_namespaces]]
binding = "ZEEK"
id = "$NAMESPACE"
EOF

cat >"$DEPLOY_FILE" <<EOF
PROJECT=$SUBDOMAIN
PAGES_PROJECT=$PROJECT
KV_NAMESPACE=$NAMESPACE
EOF

SECRET_FILE="$ZEEK_ROOT/.secrets/proxy-secret"
if [ ! -s "$SECRET_FILE" ]; then
  (umask 077 && openssl rand -hex 32 >"$SECRET_FILE")
fi
"$WRANGLER" pages secret put ZEEK_PROXY_SECRET --project-name "$PROJECT" <"$SECRET_FILE" >/dev/null

say "Cloudflare is set up: https://$SUBDOMAIN.pages.dev (KV namespace $NAMESPACE)."
say "Next: scripts/deploy-web.sh, then scripts/zeek-up.sh"
