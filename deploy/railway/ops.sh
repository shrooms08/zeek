#!/bin/sh
# Operator steps inside a Zeek container on Railway, run as root through
# `railway ssh` (plain words only, so nothing depends on remote quoting):
#
#   ops.sh held          exit 0 when the service's binary is not running
#                        (the service variable ZEEK_HOLD=1 keeps it stopped)
#   ops.sh pack SERVICE  /data/export/SERVICE.tar (+ .sha256): secrets/,
#                        data/ and MANIFEST.sha256 (scripts/railway-pull.sh)
#   ops.sh backup STAMP  consistent copies of /data/data/*.sqlite in
#                        /data/backups/STAMP with SHA256SUMS
#                        (scripts/railway-backup.sh)
#   ops.sh clean NAME    removes /data/export or /data/backups/NAME
#
# Prints hashes and counts only, never file contents.
set -eu
umask 077
cmd=${1:?usage: ops.sh held|pack|backup|clean ...}
case $cmd in
  held)
    if grep -qsxE "zeek-(engine|keyholder)" /proc/[0-9]*/comm; then exit 1; fi
    ;;
  pack)
    svc=${2:?usage: ops.sh pack SERVICE}
    case $svc in engine | keyholder-1 | keyholder-2 | keyholder-3) ;; *) exit 2 ;; esac
    if grep -qsxE "zeek-(engine|keyholder)" /proc/[0-9]*/comm; then
      echo "the service is running; hold it first" >&2
      exit 1
    fi
    work=$(mktemp -d)
    mkdir -p /data/export
    cd /data
    find secrets data -type f | LC_ALL=C sort | xargs sha256sum >"$work/MANIFEST.sha256"
    tar -cf "/data/export/$svc.tar" -C "$work" MANIFEST.sha256 -C /data secrets data
    sha256sum "/data/export/$svc.tar" | cut -d' ' -f1 >"/data/export/$svc.tar.sha256"
    echo "$(wc -l <"$work/MANIFEST.sha256") files packed, sha256 $(cat "/data/export/$svc.tar.sha256")"
    rm -rf "$work"
    ;;
  backup)
    stamp=${2:?usage: ops.sh backup STAMP}
    case $stamp in *[!0-9TZ]* | "") exit 2 ;; esac
    d="/data/backups/$stamp"
    mkdir -p "$d"
    for f in /data/data/*.sqlite; do
      out="$d/$(basename "$f")"
      sqlite3 "$f" ".backup '$out'"
      [ "$(sqlite3 "$out" 'PRAGMA integrity_check;')" = ok ]
    done
    cd "$d"
    sha256sum -- *.sqlite >SHA256SUMS
    echo "$(wc -l <SHA256SUMS) databases copied and checked"
    ;;
  clean)
    name=${2:?usage: ops.sh clean export|STAMP}
    case $name in
      export) rm -rf /data/export ;;
      *[!0-9TZ]* | "") exit 2 ;;
      *) rm -rf "/data/backups/$name" ;;
    esac
    ;;
  *)
    echo "unknown step $cmd" >&2
    exit 2
    ;;
esac
