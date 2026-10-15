#!/bin/sh
# Stops Zeek on this Mac and removes its launchd agents, so nothing restarts
# at login until scripts/zeek-up.sh runs again. The public site stays up and
# reports the services as offline (the tunnel hostnames are removed from KV).
#
#   scripts/zeek-down.sh            stop everything except the system daemons
#                                   (engine and keyholders under their own users)
#   scripts/zeek-down.sh --daemons  also stop those daemons (asks for your password)
set -eu
# shellcheck source=scripts/zeek-env.sh
. "$(dirname "$0")/zeek-env.sh"

STOP_DAEMONS=no
if [ "${1:-}" = "--daemons" ]; then
  STOP_DAEMONS=yes
fi

# Reverse start order: the publisher first, so it does not republish.
for agent in demo-digger publish tunnel-relay tunnel-engine relay engine keyholder-3 keyholder-2 keyholder-1 caffeinate; do
  if agent_loaded "$agent"; then
    launchd_stop "$DOMAIN/dev.zeek.$agent" ||
      fail "$agent did not stop within ${LAUNCHD_STOP_WAIT}s; see: launchctl print $DOMAIN/dev.zeek.$agent"
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
  for service in engine keyholder-1 keyholder-2 keyholder-3; do
    if is_daemon "$service" && launchd_present "system/dev.zeek.$service"; then
      sudo launchctl bootout "system/dev.zeek.$service" 2>/dev/null || true
      launchd_wait_gone "system/dev.zeek.$service" ||
        fail "$service did not stop within ${LAUNCHD_STOP_WAIT}s; see: sudo launchctl print system/dev.zeek.$service"
      say "stopped $service (daemon; it starts again at boot, or with: sudo launchctl bootstrap system /Library/LaunchDaemons/dev.zeek.$service.plist)"
    fi
  done
fi
say "Zeek is down."
