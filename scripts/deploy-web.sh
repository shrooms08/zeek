#!/bin/sh
# Builds the web app for the public site and deploys it to Cloudflare Pages.
#
# Hosted (apps/web/.zeek-deploy names ENGINE_ORIGIN and RELAY_ORIGIN, the
# Railway services): the app calls the engine and the relay directly, and
# only share links go through the site's /share proxy (functions/), which
# keeps them stable and edge-cached.
# On this Mac (no origins): the app reaches the engine and the relay through
# the site's own /api and /relay paths, so no tunnel address is baked in.
# The build also carries the last settled rounds for Replay mode
# (scripts/snapshot-rounds.mjs), shown while the engine is offline.
#
#   scripts/deploy-web.sh
set -eu
# shellcheck source=scripts/zeek-env.sh
. "$(dirname "$0")/zeek-env.sh"

PROJECT=$(deploy_value PAGES_PROJECT) || fail "run scripts/cloudflare-setup.sh first"
ORIGIN=$(public_origin)

# zeek-verify writes each saved round's replay.
release_build
ENGINE_ORIGIN=$(deploy_value ENGINE_ORIGIN || true)
RELAY_ORIGIN=$(deploy_value RELAY_ORIGIN || true)
cd "$ZEEK_ROOT/apps/web"
if [ -n "$ENGINE_ORIGIN" ]; then
  [ -n "$RELAY_ORIGIN" ] || fail ".zeek-deploy names ENGINE_ORIGIN but not RELAY_ORIGIN"
  VITE_ZEEK_ENGINE_URL=$ENGINE_ORIGIN VITE_ZEEK_SECOND_SOURCE=$RELAY_ORIGIN VITE_ZEEK_SHARE_URL=/ \
    pnpm -s build
  SNAPSHOT_FROM=$ENGINE_ORIGIN
else
  VITE_ZEEK_ENGINE_URL=/api VITE_ZEEK_SECOND_SOURCE=/relay pnpm -s build
  SNAPSHOT_FROM="http://$ENGINE_LOCAL"
fi
# Replay mode's saved rounds (real data from the engine; the deploy stops
# here if the engine does not answer, rather than ship without them).
node "$ZEEK_ROOT/scripts/snapshot-rounds.mjs" --engine "$SNAPSHOT_FROM" --out dist/replay ||
  fail "could not save the rounds for Replay mode (is the engine running?)"
"$WRANGLER" pages deploy dist --project-name "$PROJECT" --branch main --commit-dirty=true
wait_http "$ORIGIN/" 120 || fail "$ORIGIN does not load after the deploy"
say "Deployed: $ORIGIN"
