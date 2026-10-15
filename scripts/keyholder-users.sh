#!/bin/sh
# Gives each keyholder its own macOS user, so no other process on this Mac
# (in particular the internet-facing engine, which runs as you) can read a
# key share (security review H2). Run once, and again after the keyholder
# code changes:
#
#   sudo scripts/keyholder-users.sh
#
# For keyholders 1-3 of the testnet vault, and of the mainnet vault, it:
#   * creates a hidden service user and group (_zeekkhN testnet, _zeekmkhN
#     mainnet) that cannot log in;
#   * moves the node's secrets and databases out of this repository into
#     /Library/Application Support/Zeek/<network>/keyholder-N (owned by that
#     user, mode 0700) and rewrites the node's config with those paths;
#   * moves the retired dealer vault's material of node N there too;
#   * installs a root-owned copy of the keyholder binary (so your user cannot
#     swap the program a keyholder runs) in /usr/local/libexec/zeek;
#   * for testnet, installs /Library/LaunchDaemons/dev.zeek.keyholder-N.plist,
#     which runs the node as its user from boot, restarts it on a crash and
#     logs to /Library/Logs/Zeek/testnet-keyholder-N (readable by you).
#
# Mainnet keyholders get users and protected directories but no daemon: the
# mainnet vault does not sign anything until docs/mainnet-runbook.md opens it.
#
# The keyholders still run on this one Mac and are operated by the Zeek team.
set -eu

[ "$(id -u)" = 0 ] || {
  echo "run with sudo: sudo $0" >&2
  exit 1
}
OWNER=${SUDO_USER:?run through sudo from your own account}
REPO=$(cd "$(dirname "$0")/.." && pwd)
BASE="/Library/Application Support/Zeek"
LIBEXEC=/usr/local/libexec/zeek
LOGS=/Library/Logs/Zeek
BINARY="$REPO/target/release/zeek-keyholder"
[ -x "$BINARY" ] || {
  echo "build it first (as $OWNER): cargo build --release -p zeek-keyholder" >&2
  exit 1
}

say() { printf '%s\n' "$*"; }

# First id in 450-499 free as both a uid and a gid.
free_id() {
  id=450
  while [ "$id" -lt 500 ]; do
    if ! dscl . -search /Users UniqueID "$id" | grep -q . &&
      ! dscl . -search /Groups PrimaryGroupID "$id" | grep -q .; then
      echo "$id"
      return 0
    fi
    id=$((id + 1))
  done
  echo "no free id in 450-499" >&2
  return 1
}

ensure_user() {
  name=$1
  real=$2
  if dscl . -read "/Users/$name" >/dev/null 2>&1; then
    return 0
  fi
  uid=$(free_id)
  dscl . -create "/Groups/$name"
  dscl . -create "/Groups/$name" PrimaryGroupID "$uid"
  dscl . -create "/Groups/$name" Password '*'
  dscl . -create "/Users/$name"
  dscl . -create "/Users/$name" UniqueID "$uid"
  dscl . -create "/Users/$name" PrimaryGroupID "$uid"
  dscl . -create "/Users/$name" UserShell /usr/bin/false
  dscl . -create "/Users/$name" NFSHomeDirectory /var/empty
  dscl . -create "/Users/$name" RealName "$real"
  dscl . -create "/Users/$name" Password '*'
  dscl . -create "/Users/$name" IsHidden 1
  say "created user $name (uid $uid)"
}

# Moves $1 to $2 if $1 still exists (a re-run finds it already moved).
move_if_present() {
  if [ -e "$1" ]; then
    mkdir -p "$(dirname "$2")"
    if [ -e "$2" ]; then
      echo "both $1 and $2 exist; resolve by hand" >&2
      exit 1
    fi
    mv "$1" "$2"
    say "moved $1"
  fi
}

install -d -o root -g wheel -m 755 "$LIBEXEC"
install -o root -g wheel -m 755 "$BINARY" "$LIBEXEC/zeek-keyholder"
install -d -o root -g wheel -m 755 "$LOGS"

setup() {
  network=$1
  n=$2
  user=$3
  config=$4
  secrets=$5
  data=$6
  game=$7
  dir="$BASE/$network/keyholder-$n"

  ensure_user "$user" "Zeek $network keyholder $n"
  install -d -o root -g wheel -m 755 "$BASE" "$BASE/$network"
  install -d -o "$user" -g "$user" -m 700 "$dir"
  move_if_present "$secrets" "$dir/secrets"
  move_if_present "$data" "$dir/data"
  if [ "$network" = testnet ]; then
    move_if_present "$REPO/.secrets/old-dealer/node-$n" "$dir/old-dealer"
    move_if_present "$REPO/.secrets/old-dealer/keyholder-$n.json" "$dir/old-dealer/dealer-keyholder-$n.json"
  fi
  mkdir -p "$dir/data"
  cp "$game" "$dir/game.json"
  # The node's config, with its paths moved here.
  sed -e "s|\"data_dir\": *\"[^\"]*\"|\"data_dir\": \"$dir/data\"|" \
    -e "s|\"secrets_dir\": *\"[^\"]*\"|\"secrets_dir\": \"$dir/secrets\"|" \
    -e "s|\"game_config\": *\"[^\"]*\"|\"game_config\": \"$dir/game.json\"|" \
    "$REPO/$config" >"$dir/config.json"
  chown -R "$user:$user" "$dir"
  chmod -R go-rwx "$dir"
}

# Stop the testnet nodes before their files move.
owner_uid=$(id -u "$OWNER")
for n in 1 2 3; do
  label="dev.zeek.keyholder-$n"
  launchctl bootout "gui/$owner_uid/$label" 2>/dev/null || true
  rm -f "/Users/$OWNER/Library/LaunchAgents/$label.plist"
  pkill -f "zeek-keyholder serve --config config/keyholder-$n.json" 2>/dev/null || true
  launchctl bootout "system/$label" 2>/dev/null || true
done
sleep 2

for n in 1 2 3; do
  setup testnet "$n" "_zeekkh$n" "config/keyholder-$n.json" \
    "$REPO/.secrets/keyholder-$n" "$REPO/data/keyholder-$n" "$REPO/config/game.testnet.json"
  if [ -f "$REPO/config/game.mainnet.json" ]; then
    setup mainnet "$n" "_zeekmkh$n" "config/mainnet/keyholder-$n.json" \
      "$REPO/.secrets/mainnet/keyholder-$n" "$REPO/data/mainnet/keyholder-$n" "$REPO/config/game.mainnet.json"
  fi
done

# Testnet keyholders as daemons (replacing the user agents zeek-up installs).
for n in 1 2 3; do
  label="dev.zeek.keyholder-$n"
  user="_zeekkh$n"
  dir="$BASE/testnet/keyholder-$n"
  logs="$LOGS/testnet-keyholder-$n"
  install -d -o "$user" -g staff -m 750 "$logs"
  plist="/Library/LaunchDaemons/$label.plist"
  cat >"$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$label</string>
  <key>UserName</key><string>$user</string>
  <key>GroupName</key><string>$user</string>
  <key>ProgramArguments</key>
  <array>
    <string>$LIBEXEC/zeek-keyholder</string>
    <string>serve</string>
    <string>--config</string>
    <string>$dir/config.json</string>
  </array>
  <key>WorkingDirectory</key><string>$dir</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>ZEEK_LOG_DIR</key><string>$logs</string>
    <key>RUST_LOG</key><string>info</string>
    <key>HOME</key><string>/var/empty</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>ProcessType</key><string>Standard</string>
  <key>Umask</key><integer>23</integer>
  <key>StandardOutPath</key><string>$logs/stdout.log</string>
  <key>StandardErrorPath</key><string>$logs/stdout.log</string>
</dict>
</plist>
EOF
  chown root:wheel "$plist"
  chmod 644 "$plist"
  launchctl bootstrap system "$plist"
  say "keyholder $n runs as $user (logs: $logs)"
done

say ""
say "Done. Check with: scripts/zeek-status.sh"
