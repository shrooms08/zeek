#!/bin/sh
# Unpacks the operator's export into this service's volume (run as root
# through `railway ssh`, by scripts/railway-migrate.sh):
#
#   /app/import.sh SERVICE
#
# SERVICE must be the one this image was built for. The export is
# /data/import/SERVICE.tar, whose sha256 is in /data/import/SERVICE.tar.sha256;
# inside it, MANIFEST.sha256 lists every file's sha256. Prints only hashes
# checks and counts, never contents. Refuses to overwrite an imported volume.
set -eu
umask 077
svc=${1:?usage: import.sh SERVICE}
case $svc in
  engine) mine=/app/zeek-engine ;;
  keyholder-1 | keyholder-2 | keyholder-3) mine="/app/$svc.json" ;;
  *) mine= ;;
esac
if [ -z "$mine" ] || [ ! -e "$mine" ]; then
  echo "this is not the $svc image; refusing" >&2
  exit 1
fi
in=/data/import
if [ -e /data/.imported ]; then
  echo "this volume was already imported; refusing to overwrite it" >&2
  exit 1
fi
for d in secrets data; do
  if [ -e "/data/$d" ]; then
    echo "/data/$d exists; refusing to overwrite it" >&2
    exit 1
  fi
done
echo "$(cat "$in/$svc.tar.sha256")  $in/$svc.tar" | sha256sum -c --strict -
tar -x -f "$in/$svc.tar" -C /data --no-same-owner --no-same-permissions
cd /data
sha256sum -c --strict --quiet MANIFEST.sha256
echo "$(wc -l <MANIFEST.sha256) files match the manifest"
rm -f "$in/$svc.tar" "$in/$svc.tar.sha256"
rmdir "$in" 2>/dev/null || true
touch /data/.imported
echo "imported; the service starts now"
