#!/usr/bin/env bash
# Runs only in the GitHub runner's disposable graphical session.
set -euo pipefail
test "$#" -eq 1
exe=$(realpath "$1")
profile=$(mktemp -d)
trap 'rm -rf "$profile"' EXIT
export XDG_DATA_HOME="$profile/data" XDG_CONFIG_HOME="$profile/config" XDG_CACHE_HOME="$profile/cache"
mkdir -p "$XDG_DATA_HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME"
timeout --signal=TERM --kill-after=5s 45s xvfb-run -a dbus-run-session -- bash -euo pipefail -c '
  "$1" >"$2/launch.log" 2>&1 &
  app=$!
  trap '"'"'kill "$app" 2>/dev/null || true; wait "$app" 2>/dev/null || true'"'"' EXIT
  for _ in $(seq 1 30); do
    if ! kill -0 "$app" 2>/dev/null; then
      cat "$2/launch.log"
      exit 1
    fi
    if xdotool search --onlyvisible --name "Drifting Next" >/dev/null 2>&1; then
      sleep 2
      kill -0 "$app"
      exit 0
    fi
    sleep 1
  done
  cat "$2/launch.log"
  echo "No visible Next Alpha window" >&2
  exit 1
' launch "$exe" "$profile"
