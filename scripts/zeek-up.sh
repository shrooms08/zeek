#!/bin/sh
# Starts Zeek on this Mac and checks it end to end through the public site.
#
#   scripts/zeek-up.sh
#
# 1. Builds the release binaries if needed.
# 2. Installs and starts the launchd agents (they restart on crash and at
#    login): caffeinate, the three keyholders (unless they already run as
#    system daemons, see scripts/keyholder-users.sh), the engine, the
#    second-source relay, one Cloudflare quick tunnel each for the engine's
#    public listener and the relay, and the publisher that stores the tunnel
#    hostnames in Workers KV.
# 3. Waits for every local service, for the tunnel hostnames to reach KV, and
#    for the public site to answer through them (health, relay, live stream).
# 4. Prints the public address.
#
# Keyholders listen on localhost only and are never tunneled.
set -eu
. "$(dirname "$0")/zeek-env.sh"

command -v cloudflared >/dev/null || fail "cloudflared is not installed (brew install cloudflared)"
[ -x "$WRANGLER" ] || fail "wrangler is missing: run pnpm install"
ORIGIN=$(public_origin) || fail "no Cloudflare deployment yet: run scripts/cloudflare-setup.sh"
[ -s "$ZEEK_ROOT/.secrets/proxy-secret" ] || fail "missing .secrets/proxy-secret: run scripts/cloudflare-setup.sh"
CLOUDFLARED=$(command -v cloudflared)

say "Building release binaries (no-op when up to date)…"
(cd "$ZEEK_ROOT" && CARGO_INCREMENTAL=0 cargo build --release -q \
  -p zeek-engine -p zeek-keyholder -p zeek-verify --features zeek-verify/cli)

mkdir -p "$AGENTS_DIR" "$LOG_DIR" "$RUN_DIR"
BIN="$ZEEK_ROOT/target/release"

write_agent caffeinate Background "$LOG_DIR/caffeinate.log" "" -- \
  /usr/bin/caffeinate -i -s
for n in 1 2 3; do
  if ! keyholder_is_daemon "$n"; then
    write_agent "keyholder-$n" Standard "$LOG_DIR/keyholder-$n.out.log" \
      "ZEEK_LOG_DIR=$LOG_DIR/keyholder-$n RUST_LOG=info" -- \
      "$BIN/zeek-keyholder" serve --config "config/keyholder-$n.json"
  fi
done
# The app says when Zeek's own test wallets dig (scripts/demo-digger.sh).
DEMO=off
[ -f "$RUN_DIR/demo-digger" ] && DEMO=on
write_agent engine Interactive "$LOG_DIR/engine.out.log" \
  "ZEEK_DEMO_DIGS=$DEMO ZEEK_LOG_DIR=$LOG_DIR/engine RUST_LOG=info ZEEK_PUBLIC_BIND=$ENGINE_PUBLIC ZEEK_PROXY_SECRET_FILE=.secrets/proxy-secret ZEEK_PUBLIC_URL=$ORIGIN ZEEK_APP_URL=$ORIGIN ZEEK_LIGHTWALLETD_URL=$TESTNET_LIGHTWALLETD" -- \
  "$BIN/zeek-engine"
write_agent relay Standard "$LOG_DIR/relay.log" "" -- \
  "$BIN/zeek-verify" relay --lightwalletd "$TESTNET_LIGHTWALLETD" --bind "$RELAY_BIND" \
  --cors http://localhost:4174
write_agent tunnel-engine Standard "$LOG_DIR/tunnel-engine.log" "" -- \
  "$CLOUDFLARED" tunnel --no-autoupdate --metrics "$ENGINE_METRICS" --url "http://$ENGINE_PUBLIC"
write_agent tunnel-relay Standard "$LOG_DIR/tunnel-relay.log" "" -- \
  "$CLOUDFLARED" tunnel --no-autoupdate --metrics "$RELAY_METRICS" --url "http://$RELAY_BIND"
write_agent publish Background "$LOG_DIR/publish.log" "" -- \
  /bin/sh "$ZEEK_ROOT/scripts/zeek-publish.sh"
if [ "$DEMO" = on ]; then
  write_agent demo-digger Background "$LOG_DIR/demo-digger.log" "" -- \
    /bin/sh "$ZEEK_ROOT/scripts/demo-digger.sh" run "$(cat "$RUN_DIR/demo-digger")"
else
  launchctl bootout "$DOMAIN/dev.zeek.demo-digger" 2>/dev/null || true
  rm -f "$AGENTS_DIR/dev.zeek.demo-digger.plist"
fi

say "Starting services…"
for agent in $AGENTS; do
  plist="$AGENTS_DIR/dev.zeek.$agent.plist"
  [ -f "$plist" ] || continue
  if agent_loaded "$agent"; then
    launchctl bootout "$DOMAIN/dev.zeek.$agent" 2>/dev/null || true
    # bootout returns before the job is fully gone.
    i=0
    while agent_loaded "$agent" && [ "$i" -lt 20 ]; do
      sleep 0.5
      i=$((i + 1))
    done
  fi
  launchctl bootstrap "$DOMAIN" "$plist"
done
# The published markers belong to the tunnels just replaced.
rm -f "$RUN_DIR"/*.published

say "Waiting for local services…"
for port in $KEYHOLDER_PORTS; do
  wait_http "http://127.0.0.1:$port/v1/health" 120 || fail "keyholder on $port did not start (see $LOG_DIR)"
done
wait_http "http://$ENGINE_LOCAL/v1/health" 120 || fail "engine did not start (see $LOG_DIR/engine)"
wait_http "http://$RELAY_BIND/v1/chain/tip" 120 || fail "relay did not start (see $LOG_DIR/relay.log)"

say "Waiting for the tunnels and their publication…"
for service in engine relay; do
  i=0
  while [ ! -s "$RUN_DIR/$service.published" ]; do
    [ "$i" -lt 180 ] || fail "the $service tunnel was not published (see $LOG_DIR/publish.log, $LOG_DIR/tunnel-$service.log)"
    sleep 3
    i=$((i + 3))
  done
  say "  $service: https://$(cat "$RUN_DIR/$service.published")"
done

say "Checking the public site (KV changes can take up to a minute to spread)…"
wait_http "$ORIGIN/" 60 || fail "$ORIGIN does not load"
i=0
until curl -fsS -m 10 "$ORIGIN/api/v1/health" 2>/dev/null | grep -q '"ok":true'; do
  [ "$i" -lt 150 ] || fail "$ORIGIN/api/v1/health does not reach the engine"
  sleep 5
  i=$((i + 5))
done
wait_http "$ORIGIN/relay/v1/chain/tip" 150 || fail "$ORIGIN/relay does not reach the relay"
node "$ZEEK_ROOT/scripts/ws-check.mjs" "$ORIGIN/api/v1/stream" ||
  fail "the live stream does not connect through $ORIGIN"

say ""
say "Zeek is up: $ORIGIN"
say "Status page: $ORIGIN/status"
