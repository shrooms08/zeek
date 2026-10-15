#!/bin/sh
# Builds the web app for the public site and deploys it to Cloudflare Pages.
# The app reaches the engine and the second-source relay through the site's
# own /api and /relay paths (functions/), so no tunnel address is baked in.
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
cd "$ZEEK_ROOT/apps/web"
VITE_ZEEK_ENGINE_URL=/api VITE_ZEEK_SECOND_SOURCE=/relay pnpm -s build
# Replay mode's saved rounds (real data from the engine; the deploy stops
# here if the engine does not answer, rather than ship without them).
node "$ZEEK_ROOT/scripts/snapshot-rounds.mjs" --engine "http://$ENGINE_LOCAL" --out dist/replay ||
  fail "could not save the rounds for Replay mode (is the engine running?)"
"$WRANGLER" pages deploy dist --project-name "$PROJECT" --branch main --commit-dirty=true
wait_http "$ORIGIN/" 120 || fail "$ORIGIN does not load after the deploy"
say "Deployed: $ORIGIN"
