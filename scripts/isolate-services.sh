#!/bin/sh
# Runs the engine and each keyholder as its own macOS user (security review
# H2, N1, N2), so that:
#   * no other process can read a keyholder's key share;
#   * the internet-facing engine cannot read key shares, cannot write this
#     repository, and has neither your sudo rights nor your Cloudflare login.
#
#   sudo scripts/isolate-services.sh                 first run
#   sudo scripts/isolate-services.sh --mainnet-dkg   also replace the mainnet
#       vault with a fresh one whose ceremony runs inside the separated users
#   sudo scripts/isolate-services.sh --update-binary zeek-engine=<sha256> \
#       --update-binary zeek-keyholder=<sha256>      install rebuilt binaries
#   sudo scripts/isolate-services.sh --replace-config  install changed configs
#
# Run it before the site is public, from a clean checkout. It refuses to run
# with uncommitted changes, refuses any symbolic link in what it moves, and
# never replaces an installed binary or config silently: later changes need
# the binary's SHA-256 (from a build you ran yourself) or --replace-config.
#
# What it does:
#   * users and groups (cannot log in): _zeekengine, _zeekkh1-3 (testnet
#     keyholders), _zeekmkh1-3 (mainnet keyholders);
#   * moves each service's secrets and databases out of the repository into
#     /Library/Application Support/Zeek/<network>/<service> (0700, owned by
#     that user), with root-owned copies of the binaries in
#     /usr/local/libexec/zeek;
#   * runs the testnet engine and keyholders as LaunchDaemons from boot,
#     restarting on a crash, logging to /Library/Logs/Zeek (readable by you).
#
# The services still all run on this one Mac, operated by the Zeek team.
set -eu
umask 077

[ "$(id -u)" = 0 ] || {
  echo "run with sudo: sudo $0" >&2
  exit 1
}
OWNER=${SUDO_USER:?run through sudo from your own account}
OWNER_UID=$(id -u "$OWNER")
REPO=$(cd "$(dirname "$0")/.." && pwd)
BASE="/Library/Application Support/Zeek"
LIBEXEC=/usr/local/libexec/zeek
LOGS=/Library/Logs/Zeek
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH

say() { printf '%s\n' "$*"; }
fail() {
  printf 'isolate-services: %s\n' "$*" >&2
  exit 1
}

MAINNET_DKG=no
REPLACE_CONFIG=no
UPDATES=""
while [ $# -gt 0 ]; do
  case $1 in
    --mainnet-dkg) MAINNET_DKG=yes ;;
    --replace-config) REPLACE_CONFIG=yes ;;
    --update-binary)
      [ $# -ge 2 ] || fail "--update-binary needs NAME=SHA256"
      UPDATES="$UPDATES $2"
      shift
      ;;
    *) fail "unknown option $1" ;;
  esac
  shift
done

# ---- checks on what root is about to trust ----

[ -z "$(sudo -u "$OWNER" git -C "$REPO" status --porcelain --untracked-files=no)" ] ||
  fail "the repository has uncommitted changes; commit or stash them first"

# Refuses a path that is, or contains, a symbolic link.
no_links() {
  for p in "$@"; do
    if [ -L "$p" ]; then
      fail "$p is a symbolic link; refusing"
    fi
    if [ -d "$p" ] && [ -n "$(find "$p" -type l -print 2>/dev/null | head -n 1)" ]; then
      fail "$p contains a symbolic link; refusing"
    fi
  done
}

# Creates directory $1 (owner $2, mode $3) unless it exists as a real
# directory; refuses a symbolic link.
make_dir() {
  if [ -L "$1" ]; then
    fail "$1 is a symbolic link; refusing"
  fi
  if [ ! -d "$1" ]; then
    mkdir "$1"
  fi
  chown "$2" "$1"
  chmod "$3" "$1"
}

# Moves $1 to $2 if $1 exists (a re-run finds it already moved).
move_in() {
  [ -e "$1" ] || [ -L "$1" ] || return 0
  no_links "$1"
  if [ -e "$2" ] || [ -L "$2" ]; then
    fail "both $1 and $2 exist; resolve by hand"
  fi
  mv -n "$1" "$2"
  if [ -L "$2" ]; then
    fail "$2 became a symbolic link"
  fi
  say "moved $1"
}

# Installs file $1 as $2 (owner $3, mode 600) the first time; afterwards only
# if identical or with --replace-config.
install_config() {
  no_links "$1"
  if [ -e "$2" ] && ! cmp -s "$1" "$2"; then
    [ "$REPLACE_CONFIG" = yes ] || fail "$2 differs from $1; review the change and pass --replace-config"
  fi
  install -o "$3" -g "$3" -m 600 "$1" "$2"
}

# Installs a root-owned copy of target/release/$1 the first time; afterwards
# only when it is unchanged or its SHA-256 was given with --update-binary.
install_binary() {
  src="$REPO/target/release/$1"
  dst="$LIBEXEC/$1"
  [ -x "$src" ] || fail "build it first (as $OWNER): cargo build --release -p $1"
  no_links "$src"
  sum=$(shasum -a 256 "$src" | cut -d' ' -f1)
  if [ -e "$dst" ] && ! cmp -s "$src" "$dst"; then
    case " $UPDATES " in
      *" $1=$sum "*) ;;
      *) fail "target/release/$1 differs from the installed copy (sha256 $sum). If you built it yourself, re-run with --update-binary $1=$sum" ;;
    esac
  fi
  install -o root -g wheel -m 755 "$src" "$dst"
  say "$1 installed (sha256 $sum)"
}

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
  fail "no free id in 450-499"
}

ensure_user() {
  dscl . -read "/Users/$1" >/dev/null 2>&1 && return 0
  uid=$(free_id)
  dscl . -create "/Groups/$1"
  dscl . -create "/Groups/$1" PrimaryGroupID "$uid"
  dscl . -create "/Groups/$1" Password '*'
  dscl . -create "/Users/$1"
  dscl . -create "/Users/$1" UniqueID "$uid"
  dscl . -create "/Users/$1" PrimaryGroupID "$uid"
  dscl . -create "/Users/$1" UserShell /usr/bin/false
  dscl . -create "/Users/$1" NFSHomeDirectory /var/empty
  dscl . -create "/Users/$1" RealName "$2"
  dscl . -create "/Users/$1" Password '*'
  dscl . -create "/Users/$1" IsHidden 1
  say "created user $1 (uid $uid)"
}

# Writes and loads /Library/LaunchDaemons/$1.plist running as user $2 from
# directory $3: plist_daemon LABEL USER DIR LOGDIR ENV_PAIRS -- PROGRAM ARGS...
daemon() {
  label=$1
  user=$2
  dir=$3
  logdir=$4
  envpairs=$5
  shift 6
  plist="/Library/LaunchDaemons/$label.plist"
  {
    printf '<?xml version="1.0" encoding="UTF-8"?>\n'
    printf '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n'
    printf '<plist version="1.0">\n<dict>\n'
    printf '  <key>Label</key><string>%s</string>\n' "$label"
    printf '  <key>UserName</key><string>%s</string>\n' "$user"
    printf '  <key>GroupName</key><string>%s</string>\n' "$user"
    printf '  <key>ProgramArguments</key>\n  <array>\n'
    for arg in "$@"; do printf '    <string>%s</string>\n' "$arg"; done
    printf '  </array>\n'
    printf '  <key>WorkingDirectory</key><string>%s</string>\n' "$dir"
    printf '  <key>EnvironmentVariables</key>\n  <dict>\n'
    printf '    <key>HOME</key><string>/var/empty</string>\n'
    printf '    <key>RUST_LOG</key><string>info</string>\n'
    printf '    <key>ZEEK_LOG_DIR</key><string>%s</string>\n' "$logdir"
    for pair in $envpairs; do
      printf '    <key>%s</key><string>%s</string>\n' "${pair%%=*}" "${pair#*=}"
    done
    printf '  </dict>\n'
    printf '  <key>RunAtLoad</key><true/>\n'
    printf '  <key>KeepAlive</key><true/>\n'
    printf '  <key>ThrottleInterval</key><integer>10</integer>\n'
    printf '  <key>ProcessType</key><string>Interactive</string>\n'
    printf '  <key>Umask</key><integer>23</integer>\n'
    printf '  <key>StandardOutPath</key><string>%s/stdout.log</string>\n' "$logdir"
    printf '  <key>StandardErrorPath</key><string>%s/stdout.log</string>\n' "$logdir"
    printf '</dict>\n</plist>\n'
  } >"$plist"
  chown root:wheel "$plist"
  chmod 644 "$plist"
  launchctl bootout "system/$label" 2>/dev/null || true
  launchctl bootstrap system "$plist"
}

# Rewrites a node config's paths into its protected directory.
node_config() {
  sed -e "s|\"data_dir\": *\"[^\"]*\"|\"data_dir\": \"$2/data\"|" \
    -e "s|\"secrets_dir\": *\"[^\"]*\"|\"secrets_dir\": \"$2/secrets\"|" \
    -e "s|\"game_config\": *\"[^\"]*\"|\"game_config\": \"$2/game.json\"|" \
    "$1"
}

# ---- binaries and directories ----

make_dir /usr/local root:wheel 755
make_dir "$LIBEXEC" root:wheel 755
install_binary zeek-keyholder
install_binary zeek-engine
make_dir "$LOGS" root:wheel 755
make_dir "$BASE" root:wheel 755
make_dir "$BASE/testnet" root:wheel 755
make_dir "$BASE/mainnet" root:wheel 755

# ---- stop what runs as you before its files move ----

for label in dev.zeek.engine dev.zeek.keyholder-1 dev.zeek.keyholder-2 dev.zeek.keyholder-3; do
  launchctl bootout "gui/$OWNER_UID/$label" 2>/dev/null || true
  rm -f "/Users/$OWNER/Library/LaunchAgents/$label.plist"
done
for port in 8787 8788 9101 9102 9103; do
  for pid in $(lsof -nP -t -iTCP:"$port" -sTCP:LISTEN 2>/dev/null); do
    if [ "$(ps -o user= -p "$pid" | tr -d ' ')" = "$OWNER" ]; then
      kill "$pid"
    fi
  done
done
sleep 2

# ---- testnet keyholders ----

for n in 1 2 3; do
  user="_zeekkh$n"
  dir="$BASE/testnet/keyholder-$n"
  ensure_user "$user" "Zeek testnet keyholder $n"
  make_dir "$dir" "$user:$user" 700
  move_in "$REPO/.secrets/keyholder-$n" "$dir/secrets"
  move_in "$REPO/data/keyholder-$n" "$dir/data"
  make_dir "$dir/old-dealer" "$user:$user" 700
  if [ -d "$REPO/.secrets/old-dealer/node-$n" ]; then
    no_links "$REPO/.secrets/old-dealer/node-$n"
    for f in "$REPO/.secrets/old-dealer/node-$n"/*; do
      if [ -e "$f" ]; then
        move_in "$f" "$dir/old-dealer/$(basename "$f")"
      fi
    done
    rmdir "$REPO/.secrets/old-dealer/node-$n"
  fi
  move_in "$REPO/.secrets/old-dealer/keyholder-$n.json" "$dir/old-dealer/dealer-keyholder-$n.json"
  [ -d "$dir/data" ] || make_dir "$dir/data" "$user:$user" 700
  install_config "$REPO/config/game.testnet.json" "$dir/game.json" "$user"
  node_config "$REPO/config/keyholder-$n.json" "$dir" >"$dir/config.new"
  install_config "$dir/config.new" "$dir/config.json" "$user"
  rm -f "$dir/config.new"
  chown -R "$user:$user" "$dir"
  chmod -R go-rwx "$dir"
  logdir="$LOGS/testnet-keyholder-$n"
  make_dir "$logdir" "$user:staff" 750
  daemon "dev.zeek.keyholder-$n" "$user" "$dir" "$logdir" "" -- \
    "$LIBEXEC/zeek-keyholder" serve --config "$dir/config.json"
  say "testnet keyholder $n runs as $user"
done

# ---- testnet engine ----

DEPLOY="$REPO/apps/web/.zeek-deploy"
[ -f "$DEPLOY" ] || fail "run scripts/cloudflare-setup.sh first (the engine needs the public site's address)"
no_links "$DEPLOY" "$REPO/.env"
SUBDOMAIN=$(sed -n 's/^PROJECT=//p' "$DEPLOY" | head -n 1)
ORIGIN="https://$SUBDOMAIN.pages.dev"
KEYHOLDERS=$(sed -n 's/^ZEEK_KEYHOLDERS=//p' "$REPO/.env" | head -n 1)
THRESHOLD=$(sed -n 's/^ZEEK_VAULT_THRESHOLD=//p' "$REPO/.env" | head -n 1)
[ -n "$KEYHOLDERS" ] && [ -n "$THRESHOLD" ] || fail ".env has no ZEEK_KEYHOLDERS / ZEEK_VAULT_THRESHOLD"
user=_zeekengine
dir="$BASE/testnet/engine"
ensure_user "$user" "Zeek testnet engine"
make_dir "$dir" "$user:$user" 700
make_dir "$dir/secrets" "$user:$user" 700
make_dir "$dir/data" "$user:$user" 700
move_in "$REPO/.secrets/coordinator" "$dir/secrets/coordinator"
move_in "$REPO/.secrets/vault.json" "$dir/secrets/vault.json"
move_in "$REPO/.secrets/payout-sealing.json" "$dir/secrets/payout-sealing.json"
for f in zeek-engine.sqlite zeek-engine.sqlite-wal zeek-engine.sqlite-shm \
  zeek-vault-testnet.sqlite zeek-vault-testnet.sqlite-wal zeek-vault-testnet.sqlite-shm; do
  move_in "$REPO/data/$f" "$dir/data/$f"
done
install_config "$REPO/config/game.testnet.json" "$dir/game.json" "$user"
install_config "$REPO/.secrets/proxy-secret" "$dir/secrets/proxy-secret" "$user"
chown -R "$user:$user" "$dir"
chmod -R go-rwx "$dir"
logdir="$LOGS/testnet-engine"
make_dir "$logdir" "$user:staff" 750
DEMO=off
if [ -f "$REPO/data/run/demo-digger" ]; then
  DEMO=on
fi
daemon dev.zeek.engine "$user" "$dir" "$logdir" \
  "ZEEK_GAME_CONFIG=$dir/game.json ZEEK_ENGINE_DB=$dir/data/zeek-engine.sqlite ZEEK_ENGINE_BIND=127.0.0.1:8787 ZEEK_PUBLIC_BIND=127.0.0.1:8788 ZEEK_PROXY_SECRET_FILE=$dir/secrets/proxy-secret ZEEK_PUBLIC_URL=$ORIGIN ZEEK_APP_URL=$ORIGIN ZEEK_LIGHTWALLETD_URL=https://testnet.zec.rocks:443 ZEEK_KEYHOLDERS=$KEYHOLDERS ZEEK_VAULT_THRESHOLD=$THRESHOLD ZEEK_COORDINATOR_IDENTITY=$dir/secrets/coordinator/identity.json ZEEK_VAULT_SECRETS_DIR=$dir/secrets ZEEK_VAULT_DATA_DIR=$dir/data ZEEK_ENGINE_CORS=http://localhost:4174 ZEEK_DEMO_DIGS=$DEMO" -- \
  "$LIBEXEC/zeek-engine"
say "testnet engine runs as $user"

# ---- mainnet keyholders ----

if [ "$MAINNET_DKG" = yes ]; then
  coordinator="$REPO/.secrets/mainnet/coordinator/identity.json"
  no_links "$coordinator"
  COORD=$(sed -n 's/.*"public_key": *"\([0-9a-f]\{64\}\)".*/\1/p' "$coordinator")
  [ -n "$COORD" ] || fail "no mainnet coordinator identity at $coordinator"
  ids=""
  for n in 1 2 3; do
    user="_zeekmkh$n"
    dir="$BASE/mainnet/keyholder-$n"
    ensure_user "$user" "Zeek mainnet keyholder $n"
    # The previous mainnet vault was never funded and its address never
    # published; its shares, created under your account, are discarded.
    for old in "$REPO/.secrets/mainnet/keyholder-$n" "$REPO/data/mainnet/keyholder-$n" "$dir"; do
      if [ -e "$old" ]; then
        no_links "$old"
        rm -rf "$old"
      fi
    done
    make_dir "$dir" "$user:$user" 700
    make_dir "$dir/secrets" "$user:$user" 700
    make_dir "$dir/data" "$user:$user" 700
    id=$(sudo -u "$user" "$LIBEXEC/zeek-keyholder" identity-keygen --secrets-dir "$dir/secrets")
    ids="$ids $n=$id"
  done
  servers='"https://zec.rocks:443", "https://eu.zec.rocks:443", "https://na.zec.rocks:443", "https://zec.0xrpc.io:443", "https://zec-node.cakewallet.com:443", "https://zcash.mysideoftheweb.com:9067", "https://lwd.zcashexplorer.app:9067"'
  for n in 1 2 3; do
    user="_zeekmkh$n"
    dir="$BASE/mainnet/keyholder-$n"
    peers=""
    for pair in $ids; do
      m=${pair%%=*}
      if [ "$m" = "$n" ]; then
        continue
      fi
      if [ -n "$peers" ]; then
        peers="$peers, "
      fi
      peers="$peers{\"identifier\": $m, \"url\": \"http://127.0.0.1:920$m\", \"identity\": \"${pair#*=}\"}"
    done
    cat >"$dir/config.json" <<EOF
{
  "identifier": $n,
  "threshold": 2,
  "signers": 3,
  "bind": "127.0.0.1:920$n",
  "data_dir": "$dir/data",
  "secrets_dir": "$dir/secrets",
  "game_config": "$dir/game.json",
  "lightwalletd": [$servers],
  "peers": [$peers],
  "coordinator_identity": "$COORD",
  "poll_secs": 20
}
EOF
    chown "$user:$user" "$dir/config.json"
  done
  session=$(openssl rand -hex 32)
  pids=""
  for n in 1 2 3; do
    dir="$BASE/mainnet/keyholder-$n"
    sudo -u "_zeekmkh$n" "$LIBEXEC/zeek-keyholder" dkg --config "$dir/config.json" \
      --session "$session" --network main >"$LOGS/mainnet-dkg-$n.log" 2>&1 &
    pids="$pids $!"
  done
  for pid in $pids; do
    wait "$pid" || fail "the mainnet DKG failed (see $LOGS/mainnet-dkg-*.log)"
  done
  lead="$BASE/mainnet/keyholder-1/secrets/payout-sealing.json"
  for n in 2 3; do
    install -o "_zeekmkh$n" -g "_zeekmkh$n" -m 600 "$lead" "$BASE/mainnet/keyholder-$n/secrets/payout-sealing.json"
  done
  first=""
  for n in 1 2 3; do
    out=$(sudo -u "_zeekmkh$n" "$LIBEXEC/zeek-keyholder" address --config "$BASE/mainnet/keyholder-$n/config.json")
    if [ -z "$first" ]; then
      first=$out
    fi
    [ "$out" = "$first" ] || fail "mainnet keyholder $n derived a different vault"
  done
  ADDRESS=$(printf '%s\n' "$first" | sed -n 1p)
  UIVK=$(printf '%s\n' "$first" | sed -n 2p)
  SEALING=$(sed -n 's/.*"public_key": *"\([0-9a-f]\{64\}\)".*/\1/p' "$lead")
  # The new vault's public values into your (gitignored) mainnet game
  # config; the keyholders get a pinned copy.
  game="$REPO/config/game.mainnet.json"
  no_links "$game"
  sudo -u "$OWNER" sed -i '' \
    -e "s|\"vault_address\": *\"[^\"]*\"|\"vault_address\": \"$ADDRESS\"|" \
    -e "s|\"uivk\": *\"[^\"]*\"|\"uivk\": \"$UIVK\"|" \
    -e "s|\"sealing_public_key\": *\"[^\"]*\"|\"sealing_public_key\": \"$SEALING\"|" "$game"
  for n in 1 2 3; do
    install -o "_zeekmkh$n" -g "_zeekmkh$n" -m 600 "$game" "$BASE/mainnet/keyholder-$n/game.json"
    chmod -R go-rwx "$BASE/mainnet/keyholder-$n"
  done
  say "new mainnet vault created inside the separated users; its public values are in config/game.mainnet.json (do not publish it)"
fi

say ""
say "Done. Check with: scripts/zeek-status.sh"
