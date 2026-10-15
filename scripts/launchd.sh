# Restarting launchd services reliably; sourced by the scripts that do it.
# `launchctl bootout` returns before launchd has finished tearing a service
# down, and a `launchctl bootstrap` issued meanwhile fails ("Bootstrap
# failed: 5: Input/output error"), so stopping waits until the service is
# gone and starting retries.
#
# Needs only launchctl and sleep, so it also works under the fixed PATH of
# isolate-services.sh (run as root).
# LAUNCHD_ERROR is read by the scripts that source this file.
# shellcheck shell=sh disable=SC2034

# Seconds launchd_stop waits for a service to go, and launchd_start's tries.
LAUNCHD_STOP_WAIT=20
LAUNCHD_START_TRIES=5
# Why the last launchd_start failed (launchctl's own message).
LAUNCHD_ERROR=""

launchd_present() {
  launchctl print "$1" >/dev/null 2>&1
}

# Waits up to LAUNCHD_STOP_WAIT seconds for TARGET (e.g. system/dev.zeek.engine)
# to disappear; fails if it is still there. Polls every half second.
launchd_wait_gone() {
  polls=0
  while launchd_present "$1"; do
    [ "$polls" -lt $((LAUNCHD_STOP_WAIT * 2)) ] || return 1
    sleep 0.5
    polls=$((polls + 1))
  done
}

# launchd_stop TARGET: unloads TARGET if loaded and waits until it is gone.
launchd_stop() {
  launchd_present "$1" || return 0
  launchctl bootout "$1" 2>/dev/null || true
  launchd_wait_gone "$1"
}

# launchd_start DOMAIN LABEL PLIST: loads PLIST into DOMAIN (system, gui/UID),
# first making sure no copy of LABEL is still loaded or going away. Retries a
# failed bootstrap after a short pause; on failure LAUNCHD_ERROR says why.
launchd_start() {
  tries=0
  LAUNCHD_ERROR=""
  while :; do
    if ! launchd_stop "$1/$2"; then
      LAUNCHD_ERROR="still loaded after ${LAUNCHD_STOP_WAIT}s"
    elif LAUNCHD_ERROR=$(launchctl bootstrap "$1" "$3" 2>&1); then
      LAUNCHD_ERROR=""
      return 0
    fi
    tries=$((tries + 1))
    [ "$tries" -lt "$LAUNCHD_START_TRIES" ] || return 1
    sleep 2
  done
}
