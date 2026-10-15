# Shared settings for zeek-up.sh, zeek-down.sh, zeek-status.sh and
# zeek-publish.sh. Sourced, not run.
#
# Everything runs as macOS launchd agents of the logged-in user (labels
# dev.zeek.*), except the keyholders once scripts/keyholder-users.sh has made
# them system daemons running as their own users.

ZEEK_ROOT=$(cd "$(dirname "$0")/.." && pwd)
export PATH="/usr/local/bin:/opt/homebrew/bin:$HOME/.npm-global/bin:/usr/bin:/bin:/usr/sbin:/sbin"

# Cloudflare deployment written by scripts/cloudflare-setup.sh.
DEPLOY_FILE="$ZEEK_ROOT/apps/web/.zeek-deploy"
WRANGLER="$ZEEK_ROOT/apps/web/node_modules/.bin/wrangler"

AGENTS_DIR="$HOME/Library/LaunchAgents"
LOG_DIR="$ZEEK_ROOT/data/logs"
RUN_DIR="$ZEEK_ROOT/data/run"
UID_NUM=$(id -u)
DOMAIN="gui/$UID_NUM"

ENGINE_LOCAL=127.0.0.1:8787
ENGINE_PUBLIC=127.0.0.1:8788
RELAY_BIND=127.0.0.1:8790
ENGINE_METRICS=127.0.0.1:20241
RELAY_METRICS=127.0.0.1:20242
KEYHOLDER_PORTS="9101 9102 9103"

# The testnet lightwalletd servers (docs/lightwalletd-servers.md).
TESTNET_LIGHTWALLETD="https://testnet.zec.rocks:443"

# User agents, in start order.
AGENTS="caffeinate keyholder-1 keyholder-2 keyholder-3 engine relay tunnel-engine tunnel-relay publish"

say() { printf '%s\n' "$*"; }
fail() {
  printf 'zeek: %s\n' "$*" >&2
  exit 1
}

# Reads a key of the deployment file (KEY=value lines).
deploy_value() {
  [ -f "$DEPLOY_FILE" ] || return 1
  sed -n "s/^$1=//p" "$DEPLOY_FILE" | head -n 1
}

public_origin() {
  name=$(deploy_value PROJECT) || return 1
  [ -n "$name" ] || return 1
  printf 'https://%s.pages.dev' "$name"
}

# Whether keyholder N runs as a system daemon (after keyholder-users.sh).
keyholder_is_daemon() {
  [ -f "/Library/LaunchDaemons/dev.zeek.keyholder-$1.plist" ]
}

agent_loaded() {
  launchctl print "$DOMAIN/dev.zeek.$1" >/dev/null 2>&1
}

# Writes ~/Library/LaunchAgents/dev.zeek.NAME.plist.
#   write_agent NAME PROCESS_TYPE LOGFILE ENV_PAIRS -- PROGRAM ARGS...
# ENV_PAIRS is a space-separated list of KEY=VALUE (values without spaces).
write_agent() {
  name=$1
  ptype=$2
  logfile=$3
  envpairs=$4
  shift 5
  plist="$AGENTS_DIR/dev.zeek.$name.plist"
  {
    printf '<?xml version="1.0" encoding="UTF-8"?>\n'
    printf '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n'
    printf '<plist version="1.0">\n<dict>\n'
    printf '  <key>Label</key><string>dev.zeek.%s</string>\n' "$name"
    printf '  <key>ProgramArguments</key>\n  <array>\n'
    for arg in "$@"; do
      printf '    <string>%s</string>\n' "$arg"
    done
    printf '  </array>\n'
    printf '  <key>WorkingDirectory</key><string>%s</string>\n' "$ZEEK_ROOT"
    printf '  <key>EnvironmentVariables</key>\n  <dict>\n'
    printf '    <key>PATH</key><string>%s</string>\n' "$PATH"
    printf '    <key>HOME</key><string>%s</string>\n' "$HOME"
    for pair in $envpairs; do
      printf '    <key>%s</key><string>%s</string>\n' "${pair%%=*}" "${pair#*=}"
    done
    printf '  </dict>\n'
    printf '  <key>RunAtLoad</key><true/>\n'
    printf '  <key>KeepAlive</key><true/>\n'
    printf '  <key>ThrottleInterval</key><integer>10</integer>\n'
    printf '  <key>ProcessType</key><string>%s</string>\n' "$ptype"
    printf '  <key>Umask</key><integer>63</integer>\n'
    printf '  <key>StandardOutPath</key><string>%s</string>\n' "$logfile"
    printf '  <key>StandardErrorPath</key><string>%s</string>\n' "$logfile"
    printf '</dict>\n</plist>\n'
  } >"$plist"
}

# The quick tunnel hostname cloudflared reports on its metrics port.
tunnel_host() {
  curl -fsS -m 3 "http://$1/quicktunnel" 2>/dev/null |
    sed -n 's/.*"hostname":"\([a-z0-9-]*\.trycloudflare\.com\)".*/\1/p'
}

# Waits up to $2 seconds for URL $1 to answer 2xx.
wait_http() {
  i=0
  while [ "$i" -lt "$2" ]; do
    if curl -fsS -m 5 -o /dev/null "$1" 2>/dev/null; then
      return 0
    fi
    sleep 2
    i=$((i + 2))
  done
  return 1
}
