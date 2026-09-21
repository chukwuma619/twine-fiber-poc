#!/usr/bin/env bash
# Stop the three local fnn processes started by start-nodes.sh.
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

if [[ -f "$NODES_DIR/daemon.pid" ]]; then
  pid="$(cat "$NODES_DIR/daemon.pid")"
  if kill -0 "$pid" 2>/dev/null; then
    kill "$pid"
    echo "stopped daemon (pid $pid)"
  fi
  rm -f "$NODES_DIR/daemon.pid"
fi

for role in seller twine buyer; do
  pidfile="$(pid_file "$role")"
  if [[ ! -f "$pidfile" ]]; then
    echo "$role is not running"
    continue
  fi
  pid="$(cat "$pidfile")"
  if kill -0 "$pid" 2>/dev/null; then
    kill "$pid"
    echo "stopped $role (pid $pid)"
  else
    echo "$role pid $pid is already gone"
  fi
  rm -f "$pidfile"
done
