#!/bin/sh
# Builds the testnet services for Linux on this Mac and deploys them to the
# Railway project linked in data/railway (docs/deployment.md).
#
#   scripts/railway-deploy.sh [SERVICE...]     default: every service
#
# SERVICE is engine, relay, keyholder-1, keyholder-2 or keyholder-3.
#
# Each service is staged in a fresh temporary folder outside the repository:
#   - deploy/railway/Dockerfile, entrypoint.sh, import.sh and ops.sh;
#   - its binary from target/x86_64-unknown-linux-gnu/release;
#   - its public config;
#   - a SHA256SUMS file over all of these, which the image build checks.
# Railway's builder only copies the files, so the binary that runs is the
# one whose full sha256 this script prints. Nothing secret is staged:
# secrets and databases reach the volumes only through
# scripts/railway-migrate.sh.
set -eu
umask 077
# shellcheck source=scripts/zeek-env.sh
. "$(dirname "$0")/zeek-env.sh"

TARGET=x86_64-unknown-linux-gnu
# glibc of the image (Debian bookworm).
GLIBC=2.36
LINUX_BIN="$ZEEK_ROOT/target/$TARGET/release"
LINK_DIR="$ZEEK_ROOT/data/railway"
SERVICES=${*:-"keyholder-1 keyholder-2 keyholder-3 engine relay"}

command -v railway >/dev/null || fail "the Railway CLI is not installed"
command -v cargo-zigbuild >/dev/null || fail "cargo-zigbuild is missing: brew install zig && cargo install cargo-zigbuild --locked"
[ -d "$LINK_DIR" ] || fail "no Railway project linked: see docs/deployment.md (Railway setup)"

for svc in $SERVICES; do
  case $svc in
    engine | relay | keyholder-1 | keyholder-2 | keyholder-3) ;;
    *) fail "unknown service $svc" ;;
  esac
done

say "Building the Linux binaries (no-op when up to date)…"
# The same packages and features as release_build (zeek-env.sh).
(cd "$ZEEK_ROOT" && CARGO_INCREMENTAL=0 cargo zigbuild --release -q --target "$TARGET.$GLIBC" \
  -p zeek-engine -p zeek-keyholder -p zeek-verify --features zeek-verify/cli \
  -p zeek-dig -p zeek-vault)

STAGE_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/zeek-railway.XXXXXX")
trap 'rm -rf "$STAGE_ROOT"' EXIT

for svc in $SERVICES; do
  stage="$STAGE_ROOT/$svc"
  mkdir -p "$stage"
  cp "$ZEEK_ROOT/deploy/railway/Dockerfile" "$ZEEK_ROOT/deploy/railway/entrypoint.sh" \
    "$ZEEK_ROOT/deploy/railway/import.sh" "$ZEEK_ROOT/deploy/railway/ops.sh" "$stage/"
  case $svc in
    engine)
      cp "$LINUX_BIN/zeek-engine" "$stage/"
      cp "$ZEEK_ROOT/config/game.testnet.json" "$stage/game.json"
      mkdir -p "$stage/config"
      cp "$ZEEK_ROOT/config/campfire-filter.json" "$stage/config/"
      ;;
    relay)
      cp "$LINUX_BIN/zeek-verify" "$stage/"
      ;;
    keyholder-*)
      cp "$LINUX_BIN/zeek-keyholder" "$stage/"
      cp "$ZEEK_ROOT/config/game.testnet.json" "$stage/game.json"
      cp "$ZEEK_ROOT/deploy/railway/$svc.json" "$stage/"
      ;;
  esac
  (cd "$stage" && find . -type f ! -name Dockerfile | sort |
    sed 's|^\./||' | xargs shasum -a 256 >"$STAGE_ROOT/$svc.sums")
  mv "$STAGE_ROOT/$svc.sums" "$stage/SHA256SUMS"
  chmod -R go+rX "$stage"
  bin=$(grep ' zeek-' "$stage/SHA256SUMS")
  say "$svc: $bin"
done

for svc in $SERVICES; do
  say "Deploying ${svc}…"
  (cd "$LINK_DIR" && railway up "$STAGE_ROOT/$svc" --path-as-root --no-gitignore \
    --service "$svc" --detach) || fail "the upload of $svc failed"
done
say "Uploaded. Follow the builds with: (cd data/railway && railway logs --build --service SERVICE)"
