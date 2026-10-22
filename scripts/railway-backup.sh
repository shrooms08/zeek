#!/bin/sh
# Daily backup of the Railway engine's databases to this Mac (Railway's
# Hobby plan has no volume backups). Database files only: no secrets.
#
#   scripts/railway-backup.sh             take one backup now
#   scripts/railway-backup.sh --install   run it every day at 04:17 (launchd
#                                         agent dev.zeek.railway-backup)
#   scripts/railway-backup.sh --uninstall
#
# 1. In the engine's container, `sqlite3 .backup` copies each database in
#    /data/data consistently (while the engine runs) to /data/backups/STAMP,
#    with SHA256SUMS.
# 2. That folder is downloaded to data/backups/railway-engine/STAMP (yours,
#    folder 700, files 600) and checked against SHA256SUMS here.
# 3. The copy on the volume is removed; the 7 newest local backups are kept.
set -eu
umask 077
# shellcheck source=scripts/zeek-env.sh
. "$(dirname "$0")/zeek-env.sh"

LINK_DIR="$ZEEK_ROOT/data/railway"
KEY="$HOME/.ssh/zeek_railway_ed25519"
DEST="$ZEEK_ROOT/data/backups/railway-engine"
KEEP=7
LABEL=dev.zeek.railway-backup
PLIST="$AGENTS_DIR/$LABEL.plist"

case "${1:-}" in
  --install)
    mkdir -p "$AGENTS_DIR" "$LOG_DIR"
    cat >"$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array><string>/bin/sh</string><string>$ZEEK_ROOT/scripts/railway-backup.sh</string></array>
  <key>WorkingDirectory</key><string>$ZEEK_ROOT</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>$PATH</string>
    <key>HOME</key><string>$HOME</string>
  </dict>
  <key>StartCalendarInterval</key>
  <dict><key>Hour</key><integer>4</integer><key>Minute</key><integer>17</integer></dict>
  <key>Umask</key><integer>63</integer>
  <key>StandardOutPath</key><string>$LOG_DIR/railway-backup.log</string>
  <key>StandardErrorPath</key><string>$LOG_DIR/railway-backup.log</string>
</dict>
</plist>
EOF
    launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
    launchctl bootstrap "$DOMAIN" "$PLIST" || fail "could not load $LABEL"
    say "installed: daily at 04:17; log $LOG_DIR/railway-backup.log"
    exit 0
    ;;
  --uninstall)
    launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
    rm -f "$PLIST"
    say "uninstalled"
    exit 0
    ;;
  "") ;;
  *) fail "usage: railway-backup.sh [--install | --uninstall]" ;;
esac

[ -d "$LINK_DIR" ] || fail "no Railway project linked in $LINK_DIR"
mkdir -p "$DEST"
chmod 700 "$ZEEK_ROOT/data/backups" "$DEST"
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
say "$(date -u '+%F %T') backup $STAMP"

(cd "$LINK_DIR" && railway ssh --service engine --identity-file "$KEY" -- \
  /app/ops.sh backup "$STAMP") || fail "the backup in the container failed"

local_dir="$DEST/$STAMP"
(cd "$LINK_DIR" && railway volume --service engine files --volume engine-data \
  download "/backups/$STAMP" "$local_dir" >/dev/null) || fail "the download failed"
chmod 700 "$local_dir"
chmod 600 "$local_dir"/*
(cd "$local_dir" && shasum -a 256 -c --quiet SHA256SUMS) || fail "the downloaded files do not match SHA256SUMS"
(cd "$LINK_DIR" && railway ssh --service engine --identity-file "$KEY" -- \
  /app/ops.sh clean "$STAMP") || say "note: could not remove /data/backups/$STAMP from the volume"
say "saved $local_dir ($(du -sh "$local_dir" | cut -f1)): $(grep -c . "$local_dir/SHA256SUMS") databases, checked"

# Keep the newest $KEEP.
# shellcheck disable=SC2012 # names are UTC stamps
ls -1 "$DEST" | sort -r | tail -n +$((KEEP + 1)) | while IFS= read -r old; do
  rm -rf "${DEST:?}/$old"
  say "removed the old backup $old"
done
