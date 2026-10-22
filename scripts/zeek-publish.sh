#!/bin/sh
# Runs as the launchd agent dev.zeek.publish. Every 15 s it reads each quick
# tunnel's current hostname from cloudflared's metrics port and, when it
# changed (cloudflared restarted), writes it to Workers KV so the public
# site's proxy follows it. Writes happen only on change (the free plan allows
# 1,000 KV writes a day). It also keeps the small launchd log files under
# 10 MB (engine and keyholders rotate their own logs).
#
# Watchdog: a quick tunnel can lose its connection to Cloudflare and never
# get it back while cloudflared keeps running (seen 2026-10-19 to 22). Once
# a minute each tunnel's public hostname is called; any answer from the
# service (the engine and relay refuse calls without the proxy secret with
# 403) proves the whole path works. After 5 failed minutes in a row the
# tunnel's agent is restarted (at most once every 10 minutes); its new
# hostname is then published as usual.
set -u
# shellcheck source=scripts/zeek-env.sh
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

# Whether https://HOST answers from the service behind the tunnel: a 2xx-4xx
# status. Cloudflare's own errors when the tunnel is down are 52x and 530.
tunnel_works() {
  code=$(curl -s -o /dev/null -m 10 -w '%{http_code}' "https://$1$2" 2>/dev/null)
  [ "$code" -ge 200 ] 2>/dev/null && [ "$code" -lt 500 ]
}

# A number kept in data/run (0 when missing).
count() {
  cat "$RUN_DIR/$1" 2>/dev/null || echo 0
}

# watch SERVICE PATH: counts failed minutes and restarts a dead tunnel.
watch() {
  service=$1
  marker="$RUN_DIR/$service.published"
  [ -s "$marker" ] || return 0
  if tunnel_works "$(cat "$marker")" "$2"; then
    echo 0 >"$RUN_DIR/$service.fails"
    return 0
  fi
  fails=$(($(count "$service.fails") + 1))
  echo "$fails" >"$RUN_DIR/$service.fails"
  now=$(date +%s)
  if [ "$fails" -ge 5 ] && [ $((now - $(count "$service.restarted"))) -ge 600 ]; then
    say "$(date -u +%FT%TZ) the $service tunnel has not answered for $fails minutes; restarting it"
    launchctl kickstart -k "$DOMAIN/dev.zeek.tunnel-$service" >/dev/null 2>&1 ||
      say "$(date -u +%FT%TZ) could not restart the $service tunnel"
    echo "$now" >"$RUN_DIR/$service.restarted"
    echo 0 >"$RUN_DIR/$service.fails"
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

tick=0
while :; do
  publish engine "$ENGINE_METRICS"
  publish relay "$RELAY_METRICS"
  if [ $((tick % 4)) -eq 0 ]; then
    watch engine /v1/health
    watch relay /v1/chain/tip
  fi
  trim_logs
  tick=$((tick + 1))
  sleep 15
done
