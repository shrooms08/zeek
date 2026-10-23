#!/bin/sh
# One-time Railway setup for the testnet (docs/deployment.md): the project
# zeek-testnet in EU West, its five services, the four volumes, every
# service's variables, and public domains for the engine and the relay only.
#
#   scripts/railway-setup.sh
#
# Links the project in data/railway. Safe to re-run: it creates only what
# is missing and sets the variables again. No variable holds a secret: the
# engine and the keyholders read theirs from their volumes.
set -eu
umask 077
# shellcheck source=scripts/zeek-env.sh
. "$(dirname "$0")/zeek-env.sh"

PROJECT=zeek-testnet
# EU West (Amsterdam); every other region at 0 replicas, since a service
# with a volume runs in one region only.
REGIONS="eu-west=1 us-west=0 us-east=0 southeast-asia=0"
LINK_DIR="$ZEEK_ROOT/data/railway"
SITE=$(public_origin) || fail "no Cloudflare deployment (apps/web/.zeek-deploy)"
# The engine finds each keyholder on the private network; the identities
# are public keys (the same ones the Mac's .env pins).
IDENTITIES=$(sed -n 's/^ZEEK_KEYHOLDERS=//p' "$ZEEK_ROOT/.env" | tr ',' '\n' | awk -F= '{print $1"="$3}')
[ "$(printf '%s\n' "$IDENTITIES" | grep -c .)" = 3 ] || fail ".env must name three keyholder identities"
KEYHOLDERS=$(printf '%s\n' "$IDENTITIES" | while IFS='=' read -r n id; do
  printf '%s=http://keyholder-%s.railway.internal:910%s=%s,' "$n" "$n" "$n" "$id"
done | sed 's/,$//')
THRESHOLD=$(sed -n 's/^ZEEK_VAULT_THRESHOLD=//p' "$ZEEK_ROOT/.env")
[ "$THRESHOLD" = 2 ] || fail "unexpected ZEEK_VAULT_THRESHOLD"

mkdir -p "$LINK_DIR"
rw() { (cd "$LINK_DIR" && railway "$@"); }
# A service's ID (`railway volume` takes IDs, not names).
service_id() {
  rw service list --json | node -e '
    let s = ""; process.stdin.on("data", (d) => (s += d)).on("end", () => {
      const hit = JSON.parse(s).find((x) => x.name === process.argv[1]);
      if (hit) console.log(hit.id);
    });' "$1"
}

if ! rw status >/dev/null 2>&1; then
  rw init --name "$PROJECT" >/dev/null || fail "could not create the project"
  say "created the project $PROJECT"
fi

existing=$(rw service list 2>/dev/null || true)
for svc in keyholder-1 keyholder-2 keyholder-3 engine relay; do
  if ! printf '%s\n' "$existing" | grep -qw "$svc"; then
    rw add --service "$svc" >/dev/null || fail "could not add $svc"
    say "added $svc"
  fi
  # shellcheck disable=SC2086 # one REGION=N word each
  rw service scale --service "$svc" $REGIONS >/dev/null ||
    fail "could not place $svc in EU West"
done

volumes=$(rw volume list 2>/dev/null || true)
for svc in keyholder-1 keyholder-2 keyholder-3 engine; do
  if ! printf '%s\n' "$volumes" | grep -q "$svc-data"; then
    sid=$(service_id "$svc")
    [ -n "$sid" ] || fail "no service $svc"
    id=$(rw volume --service "$sid" add --mount-path /data --json |
      sed -n 's/.*"id": *"\([^"]*\)".*/\1/p' | head -n 1)
    [ -n "$id" ] || fail "could not add the volume of $svc"
    rw volume update --volume "$id" --name "$svc-data" --json >/dev/null
    say "added the volume $svc-data"
  fi
done

set_vars() {
  svc=$1
  shift
  rw variable set "$@" --service "$svc" --skip-deploys >/dev/null || fail "could not set $svc's variables"
}
for n in 1 2 3; do
  set_vars "keyholder-$n" ZEEK_SERVICE="keyholder-$n" RUST_LOG=info
done
set_vars relay ZEEK_SERVICE=relay \
  ZEEK_LIGHTWALLETD_URL="$TESTNET_LIGHTWALLETD" ZEEK_APP_URL="$SITE" \
  ZEEK_CLIENT_IP_HEADER=X-Real-IP
set_vars engine ZEEK_SERVICE=engine RUST_LOG=info \
  ZEEK_GAME_CONFIG=/app/game.json \
  ZEEK_ENGINE_DB=/data/data/zeek-engine.sqlite \
  ZEEK_ENGINE_BIND=127.0.0.1:8787 \
  ZEEK_PROXY_SECRET_FILE=/data/secrets/proxy-secret \
  ZEEK_CLIENT_IP_HEADER=X-Real-IP \
  ZEEK_PUBLIC_URL="$SITE" ZEEK_APP_URL="$SITE" ZEEK_ENGINE_CORS="$SITE" \
  ZEEK_LIGHTWALLETD_URL="$TESTNET_LIGHTWALLETD" \
  ZEEK_KEYHOLDERS="$KEYHOLDERS" ZEEK_VAULT_THRESHOLD="$THRESHOLD" \
  ZEEK_COORDINATOR_IDENTITY=/data/secrets/coordinator/identity.json \
  ZEEK_VAULT_SECRETS_DIR=/data/secrets ZEEK_VAULT_DATA_DIR=/data/data \
  ZEEK_DEMO_DIGS=off
say "variables set"

# Public domains: the engine and the relay only. The keyholders get none.
for svc in engine relay; do
  rw domain --service "$svc" >/dev/null 2>&1 || true
done
rw domain list 2>/dev/null || true
say "Next: scripts/railway-deploy.sh"
