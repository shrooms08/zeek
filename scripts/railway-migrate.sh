#!/bin/sh
# Cut-over to Railway, step 2 (docs/deployment.md): uploads each export of
# scripts/railway-export.sh to its own service's volume and imports it there.
#
#   scripts/railway-migrate.sh [SERVICE...]   default: the keyholders, then the engine
#
# For each service:
#   1. checks data/migrate/SERVICE.tar against SERVICE.tar.sha256;
#   2. uploads both to /import/ on the volume SERVICE-data (Railway's API,
#      over TLS): never into a variable, never printed;
#   3. runs /app/import.sh in the container (railway ssh), which checks the
#      tar's sha256 and every file against MANIFEST.sha256, unpacks it and
#      lets the waiting service start;
#   4. deletes the local tar.
# The service's own files on this Mac stay in place as the backup.
set -eu
umask 077
# shellcheck source=scripts/zeek-env.sh
. "$(dirname "$0")/zeek-env.sh"

IN="$ZEEK_ROOT/data/migrate"
LINK_DIR="$ZEEK_ROOT/data/railway"
KEY="$HOME/.ssh/zeek_railway_ed25519"
SERVICES=${*:-"keyholder-1 keyholder-2 keyholder-3 engine"}

[ -d "$LINK_DIR" ] || fail "no Railway project linked in $LINK_DIR"
[ -f "$KEY" ] || fail "missing $KEY (docs/deployment.md, Railway setup)"

for svc in $SERVICES; do
  case $svc in
    engine | keyholder-1 | keyholder-2 | keyholder-3) ;;
    *) fail "unknown service $svc" ;;
  esac
  tar="$IN/$svc.tar"
  [ -f "$tar" ] && [ -f "$tar.sha256" ] || fail "no export for $svc in $IN (sudo scripts/railway-export.sh)"
  want=$(cat "$tar.sha256")
  have=$(shasum -a 256 "$tar" | cut -d' ' -f1)
  [ "$want" = "$have" ] || fail "$svc.tar does not match its sha256"
  say "$svc: uploading (sha256 $have)…"
  (cd "$LINK_DIR" &&
    railway volume --service "$svc" files --volume "$svc-data" upload "$tar" "/import/$svc.tar" >/dev/null &&
    railway volume --service "$svc" files --volume "$svc-data" upload "$tar.sha256" "/import/$svc.tar.sha256" >/dev/null) ||
    fail "the upload of $svc failed"
  say "$svc: importing…"
  (cd "$LINK_DIR" && railway ssh --service "$svc" --identity-file "$KEY" -- /app/import.sh "$svc") ||
    fail "the import of $svc failed; its volume was left as it is"
  rm -f "$tar" "$tar.sha256"
  say "$svc: imported; local copy of the export deleted"
done
rmdir "$IN" 2>/dev/null || true
say "Done. Check: (cd data/railway && railway logs --service SERVICE)"
