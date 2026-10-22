#!/bin/sh
# Cut-over to Railway, step 1 (docs/deployment.md): stops the testnet
# engine and keyholder daemons on this Mac, keeps them from starting again
# at boot, and exports each service's secrets and databases for upload.
#
#   sudo scripts/railway-export.sh
#
# Run scripts/zeek-down.sh first (the tunnels and the publisher). This
# script then:
#   1. stops the daemons dev.zeek.engine and dev.zeek.keyholder-1..3 and
#      disables them, so a reboot cannot start a stale copy that would pay
#      rounds Railway already paid (undo: scripts/railway-restore.sh);
#   2. writes data/migrate/SERVICE.tar (owned by you, mode 600) for engine,
#      keyholder-1, keyholder-2 and keyholder-3: the service's secrets/ and
#      data/ folders plus MANIFEST.sha256 (every file's sha256), and
#      SERVICE.tar.sha256. Keyholders' old-vault material stays here.
#
# Only /Library/Application Support/Zeek/testnet is read: the mainnet
# folders and keys are never touched. The originals stay in place as the
# backup. Prints names, counts, sizes and hashes only, never contents.
set -eu
umask 077
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH

REPO=$(cd "$(dirname "$0")/.." && pwd)
BASE="/Library/Application Support/Zeek/testnet"
SERVICES="engine keyholder-1 keyholder-2 keyholder-3"

say() { printf '%s\n' "$*"; }
fail() {
  printf 'railway-export: %s\n' "$*" >&2
  exit 1
}
# shellcheck source=scripts/launchd.sh
. "$REPO/scripts/launchd.sh"

[ "$(id -u)" = 0 ] || fail "run it with sudo"
OWNER=${SUDO_USER:?run it through sudo from your own account}
OUT="$REPO/data/migrate"

# ---- 1. stop and fence the daemons ----
for svc in $SERVICES; do
  label="dev.zeek.$svc"
  launchctl disable "system/$label"
  if launchd_present "system/$label"; then
    launchctl bootout "system/$label" 2>/dev/null || true
    launchd_wait_gone "system/$label" || fail "$label did not stop; see: sudo launchctl print system/$label"
  fi
  say "stopped and disabled $label"
done
for port in 8787 8788 9101 9102 9103; do
  if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
    fail "something still listens on port $port; stop it first"
  fi
done

# ---- 2. export ----
if [ -e "$OUT" ]; then
  [ ! -L "$OUT" ] || fail "$OUT is a symbolic link; refusing"
  [ -z "$(ls -A "$OUT")" ] || fail "$OUT is not empty: upload or remove the previous export first"
else
  mkdir "$OUT"
fi
chown "$OWNER" "$OUT"
chmod 700 "$OUT"
WORK=$(mktemp -d /tmp/zeek-export.XXXXXX)
trap 'rm -rf "$WORK"' EXIT

for svc in $SERVICES; do
  src="$BASE/$svc"
  [ -d "$src/secrets" ] && [ -d "$src/data" ] || fail "$src has no secrets/ and data/"
  if [ -n "$(find "$src/secrets" "$src/data" -type l -print | head -n 1)" ]; then
    fail "$src contains a symbolic link; refusing"
  fi
  (cd "$src" && find secrets data -type f | LC_ALL=C sort | while IFS= read -r f; do
    shasum -a 256 "$f"
  done) >"$WORK/$svc.manifest"
  mkdir "$WORK/$svc"
  cp "$WORK/$svc.manifest" "$WORK/$svc/MANIFEST.sha256"
  COPYFILE_DISABLE=1 tar --no-mac-metadata --no-xattrs --no-acls \
    -cf "$OUT/$svc.tar" -C "$WORK/$svc" MANIFEST.sha256 -C "$src" secrets data
  sum=$(shasum -a 256 "$OUT/$svc.tar" | cut -d' ' -f1)
  printf '%s\n' "$sum" >"$OUT/$svc.tar.sha256"
  chown "$OWNER" "$OUT/$svc.tar" "$OUT/$svc.tar.sha256"
  chmod 600 "$OUT/$svc.tar" "$OUT/$svc.tar.sha256"
  files=$(wc -l <"$WORK/$svc.manifest" | tr -d ' ')
  size=$(du -sk "$src/secrets" "$src/data" | awk '{s += $1} END {print s}')
  say "$svc: $files files, ${size} KiB, tar sha256 $sum"
done
say "Exported to $OUT. Next (as yourself, no sudo): scripts/railway-migrate.sh"
