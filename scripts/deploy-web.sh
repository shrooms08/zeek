#!/bin/sh
# Builds the web app for the public site and deploys it to Cloudflare Pages.
# The app reaches the engine and the second-source relay through the site's
# own /api and /relay paths (functions/), so no tunnel address is baked in.
#
#   scripts/deploy-web.sh
set -eu
. "$(dirname "$0")/zeek-env.sh"

PROJECT=$(deploy_value PAGES_PROJECT) || fail "run scripts/cloudflare-setup.sh first"
ORIGIN=$(public_origin)

cd "$ZEEK_ROOT/apps/web"
VITE_ZEEK_ENGINE_URL=/api VITE_ZEEK_SECOND_SOURCE=/relay pnpm -s build
"$WRANGLER" pages deploy dist --project-name "$PROJECT" --branch main --commit-dirty=true
wait_http "$ORIGIN/" 120 || fail "$ORIGIN does not load after the deploy"
say "Deployed: $ORIGIN"
