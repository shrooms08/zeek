#!/bin/sh
# Optional demo digger, OFF by default: keeps the testnet field alive by
# digging one random tile per round from one of Zeek's own test wallets
# (zeek-dig players under .secrets/players). While it runs, the engine tells
# the app (ZEEK_DEMO_DIGS=on), and How to play and the Trust page say:
# "Some digs on testnet come from Zeek's own test wallets."
#
#   scripts/demo-digger.sh on [player]   turn on (default player: bob); then run zeek-up.sh
#   scripts/demo-digger.sh off           turn off; then run zeek-up.sh
#   scripts/demo-digger.sh run [player]  the loop itself (what the launchd agent runs)
set -eu
. "$(dirname "$0")/zeek-env.sh"

MARKER="$RUN_DIR/demo-digger"
AMOUNT=0.001

case ${1:-} in
  on)
    player=${2:-bob}
    mkdir -p "$RUN_DIR"
    printf '%s\n' "$player" >"$MARKER"
    say "Demo digger on ($player). Start it, and the app's notice, with: scripts/zeek-up.sh"
    ;;
  off)
    launchctl bootout "$DOMAIN/dev.zeek.demo-digger" 2>/dev/null || true
    rm -f "$AGENTS_DIR/dev.zeek.demo-digger.plist" "$MARKER"
    say "Demo digger off. Restart Zeek so the app stops saying so: scripts/zeek-up.sh"
    ;;
  run)
    player=${2:-bob}
    dig="$ZEEK_ROOT/target/release/zeek-dig"
    last=""
    while :; do
      round=$(curl -fsS -m 10 "http://$ENGINE_LOCAL/v1/board" 2>/dev/null |
        sed -n 's/.*"round":\([0-9]*\),"status":"open".*/\1/p')
      if [ -n "$round" ] && [ "$round" != "$last" ]; then
        # A random moment in the round's first half, then a random tile.
        sleep $((60 + $(od -An -N1 -tu1 /dev/urandom) % 120))
        tile=$(($(od -An -N1 -tu1 /dev/urandom) % 25))
        if (cd "$ZEEK_ROOT" && "$dig" sync "$player" >/dev/null 2>&1 &&
          "$dig" dig "$player" --tile "$tile" --amount "$AMOUNT" --round "$round"); then
          say "$(date -u +%FT%TZ) round $round: $player dug tile $tile"
        else
          say "$(date -u +%FT%TZ) round $round: dig failed (balance or network)"
        fi
        last=$round
      fi
      sleep 30
    done
    ;;
  *)
    sed -n '2,12p' "$0"
    exit 2
    ;;
esac
