#!/bin/sh
# Rollback, step 1 (docs/deployment.md): stops the stateful Railway services
# and copies their state back to this Mac, for scripts/railway-restore.sh.
#
#   scripts/railway-pull.sh
#
# Needed only once Railway has paid a round: the Mac's own copies are then
# stale, and starting them would pay those rounds again.
#
# For the engine first, then each keyholder:
#   1. sets the service variable ZEEK_HOLD=1, which redeploys it with its
#      binary stopped and the container waiting (deploy/railway/entrypoint.sh),
#      reachable over `railway ssh` even if the binary was crashing;
#   2. packs /data/secrets and /data/data with MANIFEST.sha256 into
#      /data/export/SERVICE.tar, downloads it to data/rollback/ (yours, 600)
#      and checks its sha256 here;
#   3. removes the copy from the volume.
# The services stay on hold: nothing on Railway signs or pays any more (to
# resume one: railway variable delete ZEEK_HOLD --service SERVICE). The
# relay is stopped too. Prints hashes and counts only.
set -eu
umask 077
# shellcheck source=scripts/zeek-env.sh
. "$(dirname "$0")/zeek-env.sh"

LINK_DIR="$ZEEK_ROOT/data/railway"
KEY="$HOME/.ssh/zeek_railway_ed25519"
OUT="$ZEEK_ROOT/data/rollback"

[ -d "$LINK_DIR" ] || fail "no Railway project linked in $LINK_DIR"
if [ -e "$OUT" ] && [ -n "$(ls -A "$OUT")" ]; then
  fail "$OUT is not empty: restore or remove the previous pull first"
fi
mkdir -p "$OUT"
chmod 700 "$OUT"

rw() { (cd "$LINK_DIR" && railway "$@"); }
in_container() {
  svc=$1
  shift
  rw ssh --service "$svc" --identity-file "$KEY" -- "$@"
}

for svc in engine keyholder-1 keyholder-2 keyholder-3; do
  say "$svc: holding…"
  rw variable set ZEEK_HOLD=1 --service "$svc" --json >/dev/null || fail "could not put $svc on hold"
  i=0
  until in_container "$svc" /app/ops.sh held 2>/dev/null; do
    [ "$i" -lt 180 ] || fail "$svc did not reach its hold"
    sleep 5
    i=$((i + 5))
  done
  in_container "$svc" /app/ops.sh pack "$svc" || fail "could not pack $svc"
  for f in "$svc.tar" "$svc.tar.sha256"; do
    rw volume --service "$svc" files --volume "$svc-data" download "/export/$f" "$OUT/$f" >/dev/null ||
      fail "could not download $f"
  done
  chmod 600 "$OUT/$svc.tar" "$OUT/$svc.tar.sha256"
  want=$(cat "$OUT/$svc.tar.sha256")
  have=$(shasum -a 256 "$OUT/$svc.tar" | cut -d' ' -f1)
  [ "$want" = "$have" ] || fail "$svc.tar does not match its sha256 after the download"
  in_container "$svc" /app/ops.sh clean export || say "note: could not remove /data/export from $svc"
  say "$svc: pulled, sha256 $have; it stays on hold"
done
rw down --service relay --yes >/dev/null || say "note: could not stop the relay"
say "Next: sudo scripts/railway-restore.sh --from $OUT"
