#!/bin/sh
# Stops Zeek on this Mac and removes its launchd agents, so nothing restarts
# at login until scripts/zeek-up.sh runs again. The public site stays up and
# reports the services as offline (the tunnel hostnames are removed from KV).
#
#   scripts/zeek-down.sh               stop everything except system-daemon keyholders
#   scripts/zeek-down.sh --keyholders  also stop keyholder daemons (asks for your password)
set -eu
. "$(dirname "$0")/zeek-env.sh"

STOP_DAEMONS=no
[ "${1:-}" = "--keyholders" ] && STOP_DAEMONS=yes

# Reverse start order: the publisher first, so it does not republish.
for agent in demo-digger publish tunnel-relay tunnel-engine relay engine keyholder-3 keyholder-2 keyholder-1 caffeinate; do
  if agent_loaded "$agent"; then
    launchctl bootout "$DOMAIN/dev.zeek.$agent" 2>/dev/null || true
    say "stopped $agent"
  fi
  rm -f "$AGENTS_DIR/dev.zeek.$agent.plist"
done
rm -f "$RUN_DIR"/*.published

NAMESPACE=$(deploy_value KV_NAMESPACE || true)
if [ -n "${NAMESPACE:-}" ] && [ -x "$WRANGLER" ]; then
  for service in engine relay; do
    "$WRANGLER" kv key delete --remote --namespace-id "$NAMESPACE" "$service" >/dev/null 2>&1 &&
      say "unpublished the $service tunnel"
  done
fi

if [ "$STOP_DAEMONS" = yes ]; then
  for n in 1 2 3; do
    if keyholder_is_daemon "$n"; then
      sudo launchctl bootout "system/dev.zeek.keyholder-$n" 2>/dev/null || true
      say "stopped keyholder-$n (daemon; it starts again at boot or with: sudo launchctl bootstrap system /Library/LaunchDaemons/dev.zeek.keyholder-$n.plist)"
    fi
  done
fi
say "Zeek is down."
