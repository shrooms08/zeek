#!/bin/sh
# Starts one Zeek service on Railway (ZEEK_SERVICE) as an unprivileged user.
#
# Railway mounts a service's volume at /data as root. The stateful services
# (engine, keyholder-N) wait until the operator's import has written
# /data/.imported (scripts/railway-migrate.sh), so none of them ever starts
# with an empty state: no fresh database, no missing key share. Then /data
# is given to uid 10001 and the binary runs as that user with no
# capabilities. The relay keeps no state and has no volume.
#
# The service variable ZEEK_HOLD=1 keeps a stateful service from starting:
# the container runs and waits, so `railway ssh` can reach it even when the
# binary would crash, and its volume can be copied consistently
# (scripts/railway-pull.sh). Setting or removing it redeploys the service.
set -eu
umask 077
DATA=/data

run() {
  exec setpriv --reuid=10001 --regid=10001 --clear-groups --no-new-privs \
    --inh-caps=-all --bounding-set=-all -- "$@"
}

case "${ZEEK_SERVICE:?ZEEK_SERVICE is not set}" in
  relay)
    run /app/zeek-verify relay --lightwalletd "${ZEEK_LIGHTWALLETD_URL:?}" \
      --bind "[::]:${PORT:?}" --cors "${ZEEK_APP_URL:?}" \
      --client-ip-header "${ZEEK_CLIENT_IP_HEADER:?}"
    ;;
  engine | keyholder-1 | keyholder-2 | keyholder-3) ;;
  *)
    echo "unknown service $ZEEK_SERVICE" >&2
    exit 1
    ;;
esac

if [ "${RAILWAY_VOLUME_MOUNT_PATH:-}" != "$DATA" ]; then
  echo "the volume must be mounted at $DATA" >&2
  exit 1
fi
if [ "${ZEEK_HOLD:-}" = 1 ]; then
  echo "on hold (ZEEK_HOLD=1): the binary does not start"
  while :; do sleep 3600; done
fi
if [ ! -e "$DATA/.imported" ]; then
  echo "waiting for the import (scripts/railway-migrate.sh)"
  while [ ! -e "$DATA/.imported" ]; do sleep 5; done
  echo "import found"
fi
chown -R 10001:10001 "$DATA"
chmod 700 "$DATA"

case "$ZEEK_SERVICE" in
  engine)
    ZEEK_PUBLIC_BIND="[::]:${PORT:?}"
    export ZEEK_PUBLIC_BIND
    run /app/zeek-engine
    ;;
  keyholder-*)
    run /app/zeek-keyholder serve --config "/app/$ZEEK_SERVICE.json"
    ;;
esac
