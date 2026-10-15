#!/bin/sh
# Runs as the launchd agent dev.zeek.publish. Every 15 s it reads each quick
# tunnel's current hostname from cloudflared's metrics port and, when it
# changed (cloudflared restarted), writes it to Workers KV so the public
# site's proxy follows it. Writes happen only on change (the free plan allows
# 1,000 KV writes a day). It also keeps the small launchd log files under
# 10 MB (engine and keyholders rotate their own logs).
set -u
. "$(dirname "$0")/zeek-env.sh"

NAMESPACE=$(deploy_value KV_NAMESPACE) || fail "run scripts/cloudflare-setup.sh first"
mkdir -p "$RUN_DIR"

publish() {
  service=$1
  metrics=$2
  host=$(tunnel_host "$metrics")
  [ -n "$host" ] || return 0
  marker="$RUN_DIR/$service.published"
  if [ -f "$marker" ] && [ "$(cat "$marker")" = "$host" ]; then
    return 0
  fi
  value=$(printf '{"url":"https://%s","updated":%s}' "$host" "$(date +%s)")
  if "$WRANGLER" kv key put --remote --namespace-id "$NAMESPACE" "$service" "$value" >/dev/null 2>&1; then
    printf '%s\n' "$host" >"$marker"
    say "$(date -u +%FT%TZ) published $service tunnel $host"
  else
    say "$(date -u +%FT%TZ) could not publish the $service tunnel; retrying"
  fi
}

trim_logs() {
  for log in "$LOG_DIR"/*.log; do
    [ -f "$log" ] || continue
    size=$(stat -f %z "$log")
    if [ "$size" -gt 10485760 ]; then
      # launchd opens these files for appending, so truncating in place is safe.
      cp "$log" "$log.1"
      : >"$log"
    fi
  done
}

while :; do
  publish engine "$ENGINE_METRICS"
  publish relay "$RELAY_METRICS"
  trim_logs
  sleep 15
done
