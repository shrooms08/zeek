#!/bin/sh
# Shows what is running: each launchd service, the local health checks, the
# published tunnels and the public site.
set -u
# shellcheck source=scripts/zeek-env.sh
. "$(dirname "$0")/zeek-env.sh"

# check NAME URL [more curl arguments]
check() {
  name=$1
  url=$2
  shift 2
  if curl -fsS -m 5 -o /dev/null "$@" "$url" 2>/dev/null; then
    say "  ok    $name"
  else
    say "  DOWN  $name ($url)"
  fi
}

say "Services (launchd):"
for agent in $AGENTS; do
  if is_daemon "$agent"; then
    state=$(launchctl print "system/dev.zeek.$agent" 2>/dev/null | sed -n 's/^[[:space:]]*state = //p' | head -n 1)
    say "  $agent (system daemon, own user): ${state:-not loaded}"
    continue
  fi
  state=$(launchctl print "$DOMAIN/dev.zeek.$agent" 2>/dev/null | sed -n 's/^[[:space:]]*state = //p' | head -n 1)
  say "  $agent: ${state:-not loaded}"
done

say "Local health:"
for port in $KEYHOLDER_PORTS; do
  check "keyholder :$port" "http://127.0.0.1:$port/v1/health"
done
check "engine" "http://$ENGINE_LOCAL/v1/health"
check "relay" "http://$RELAY_BIND/v1/chain/tip" \
  -H "x-zeek-proxy: $(proxy_secret)" -H "x-zeek-client: 127.0.0.1"

say "Tunnels:"
for service in engine relay; do
  if [ -s "$RUN_DIR/$service.published" ]; then
    say "  $service: https://$(cat "$RUN_DIR/$service.published") (published)"
  else
    say "  $service: not published"
  fi
done

if ORIGIN=$(public_origin); then
  say "Public site $ORIGIN:"
  check "page" "$ORIGIN/"
  check "engine through the proxy" "$ORIGIN/api/v1/health"
  check "relay through the proxy" "$ORIGIN/relay/v1/chain/tip"
fi
