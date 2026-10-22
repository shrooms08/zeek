#!/bin/sh
# Rollback, step 2 (docs/deployment.md): makes this Mac the server again.
#
#   sudo scripts/railway-restore.sh --railway-stopped
#       before Railway paid any round: the Mac's own copies are current
#   sudo scripts/railway-restore.sh --railway-stopped --from data/rollback
#       after it did: first installs the state scripts/railway-pull.sh saved
#
# --railway-stopped is your statement that the Railway engine and keyholders
# are stopped (railway-pull.sh leaves them on hold; or remove their
# deployments). Two engines with the same vault must never run at once.
#
# With --from, for engine and keyholder-1..3:
#   1. checks DIR/SERVICE.tar against SERVICE.tar.sha256;
#   2. moves the service's current secrets/ and data/ aside into
#      before-restore-STAMP/ (kept, not deleted);
#   3. unpacks the tar, checks every file against its MANIFEST.sha256 and
#      gives the files to the service's own user.
# Then it re-enables and starts the four daemons that
# scripts/railway-export.sh disabled. Only the testnet folders are touched.
set -eu
umask 077
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH

REPO=$(cd "$(dirname "$0")/.." && pwd)
BASE="/Library/Application Support/Zeek/testnet"
SERVICES="keyholder-1 keyholder-2 keyholder-3 engine"

say() { printf '%s\n' "$*"; }
fail() {
  printf 'railway-restore: %s\n' "$*" >&2
  exit 1
}
# shellcheck source=scripts/launchd.sh
. "$REPO/scripts/launchd.sh"

STOPPED=no
FROM=""
while [ $# -gt 0 ]; do
  case $1 in
    --railway-stopped) STOPPED=yes ;;
    --from)
      [ $# -ge 2 ] || fail "--from needs a folder"
      FROM=$2
      shift
      ;;
    *) fail "unknown option $1" ;;
  esac
  shift
done
[ "$(id -u)" = 0 ] || fail "run it with sudo"
[ "$STOPPED" = yes ] || fail "pass --railway-stopped once the Railway engine and keyholders are stopped"

user_of() {
  case $1 in
    engine) echo _zeekengine ;;
    keyholder-1) echo _zeekkh1 ;;
    keyholder-2) echo _zeekkh2 ;;
    keyholder-3) echo _zeekkh3 ;;
  esac
}

if [ -n "$FROM" ]; then
  case $FROM in /*) ;; *) FROM="$REPO/$FROM" ;; esac
  STAMP=$(date -u +%Y%m%dT%H%M%SZ)
  for svc in $SERVICES; do
    tar="$FROM/$svc.tar"
    [ -f "$tar" ] && [ -f "$tar.sha256" ] || fail "missing $tar or its .sha256"
    [ "$(cat "$tar.sha256")" = "$(shasum -a 256 "$tar" | cut -d' ' -f1)" ] ||
      fail "$svc.tar does not match its sha256"
  done
  for svc in $SERVICES; do
    dir="$BASE/$svc"
    user=$(user_of "$svc")
    [ -d "$dir" ] && [ ! -L "$dir" ] || fail "$dir is missing"
    aside="$dir/before-restore-$STAMP"
    mkdir "$aside"
    mv "$dir/secrets" "$dir/data" "$aside/"
    tar -xf "$FROM/$svc.tar" -C "$dir"
    (cd "$dir" && shasum -a 256 -c --quiet MANIFEST.sha256) || fail "$svc: files do not match the manifest (the previous state is in $aside)"
    mv "$dir/MANIFEST.sha256" "$aside/railway-MANIFEST.sha256"
    chown -R "$user:$user" "$dir"
    chmod -R go-rwx "$dir"
    say "$svc: restored from Railway ($(wc -l <"$aside/railway-MANIFEST.sha256" | tr -d ' ') files); the Mac's previous state is in $aside"
  done
fi

for svc in $SERVICES; do
  label="dev.zeek.$svc"
  launchctl enable "system/$label"
  launchd_start system "$label" "/Library/LaunchDaemons/$label.plist" ||
    fail "could not start $label: $LAUNCHD_ERROR"
  say "started $label"
done
say "Next (as yourself): scripts/zeek-up.sh, then remove ENGINE_ORIGIN and RELAY_ORIGIN from apps/web/.zeek-deploy and the [vars] from apps/web/wrangler.toml, and run scripts/deploy-web.sh"
