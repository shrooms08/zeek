#!/bin/sh
# Shows what is running and whether the vault covers what it holds.
#
# Hosted on Railway (apps/web/.zeek-deploy names ENGINE_ORIGIN): each
# Railway service, the public engine and relay, and the public site. On this
# Mac: each launchd service, the local health checks, the published tunnels
# and the public site. Both end with the vault ledger, which warns when the
# vault would not cover the Deep Cache and what is owed.
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

ENGINE_ORIGIN=$(deploy_value ENGINE_ORIGIN || true)
RELAY_ORIGIN=$(deploy_value RELAY_ORIGIN || true)

if [ -n "$ENGINE_ORIGIN" ]; then
  say "Services (Railway, project linked in data/railway):"
  # shellcheck disable=SC2016 # JavaScript, not shell
  (cd "$ZEEK_ROOT/data/railway" 2>/dev/null && railway service list --json 2>/dev/null) |
    node -e '
      let s = ""; process.stdin.on("data", (d) => (s += d)).on("end", () => {
        try {
          for (const x of JSON.parse(s)) console.log(`  ${x.name}: ${String(x.status ?? "unknown").toLowerCase()}`);
        } catch { console.log("  (the Railway CLI did not answer)"); }
      });'
  say "Public health:"
  check "engine" "$ENGINE_ORIGIN/v1/health"
  check "relay" "$RELAY_ORIGIN/v1/chain/tip"
else

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
fi

if ORIGIN=$(public_origin); then
  say "Public site $ORIGIN:"
  check "page" "$ORIGIN/"
  check "engine through the proxy" "$ORIGIN/api/v1/health"
  check "relay through the proxy" "$ORIGIN/relay/v1/chain/tip"
fi

# The vault must cover the Deep Cache, unpaid payouts and their fees.
say "Vault:"
LEDGER_URL="${ENGINE_ORIGIN:-http://$ENGINE_LOCAL}/v1/vault/ledger"
ledger=$(curl -fsS -m 10 "$LEDGER_URL" 2>/dev/null) || ledger=""
if [ -z "$ledger" ]; then
  say "  ?     ledger unavailable ($LEDGER_URL)"
else
  # shellcheck disable=SC2016 # JavaScript, not shell
  printf '%s' "$ledger" | node -e '
    let s = ""; process.stdin.on("data", (d) => (s += d)).on("end", () => {
      const l = JSON.parse(s);
      const zec = (z) => (z / 1e8).toFixed(8).replace(/0+$/, "").replace(/\.$/, "");
      const owed = l.cache + l.unpaid_obligations + l.reserved_fees + l.unclaimable;
      const at = `(read at block ${l.last_reconciliation_height})`;
      if (l.shortfall > 0) {
        console.log(`  WARN  vault ${zec(l.vault_balance)} ZEC does not cover ${zec(owed)} ZEC owed (Deep Cache ${zec(l.cache)}): short by ${zec(l.shortfall)} ZEC ${at}`);
      } else {
        console.log(`  ok    vault ${zec(l.vault_balance)} ZEC covers ${zec(owed)} ZEC owed (Deep Cache ${zec(l.cache)}), ${zec(l.operator_float)} ZEC spare ${at}`);
      }
    });'
fi
