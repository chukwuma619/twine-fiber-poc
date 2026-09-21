#!/usr/bin/env bash
# Shared paths and Fiber RPC helper for the three local testnet nodes.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="$ROOT/bin"
NODES_DIR="$ROOT/nodes"
PASSWORD_FILE="$NODES_DIR/password"
ADDRESSES_FILE="$NODES_DIR/addresses.json"
FNN_VERSION="0.9.1"
CKB_RPC="https://testnet.ckbapp.dev/"

rpc_port() {
  case "$1" in
    seller) echo 8227 ;;
    twine) echo 8237 ;;
    buyer) echo 8247 ;;
    *)
      echo "unknown role: $1" >&2
      return 1
      ;;
  esac
}

p2p_port() {
  case "$1" in
    seller) echo 8228 ;;
    twine) echo 8238 ;;
    buyer) echo 8248 ;;
    *)
      echo "unknown role: $1" >&2
      return 1
      ;;
  esac
}

rpc_url() {
  echo "http://127.0.0.1:$(rpc_port "$1")"
}

p2p_addr() {
  echo "/ip4/127.0.0.1/tcp/$(p2p_port "$1")"
}

fiber_rpc() {
  local url="$1"
  local method="$2"
  local params="$3"
  curl -sS -m 20 \
    -H "Content-Type: application/json" \
    --data "$(jq -n --arg method "$method" --argjson params "$params" \
      '{jsonrpc:"2.0", id:1, method:$method, params:$params}')" \
    "$url"
}

fnn_bin() {
  echo "$BIN_DIR/fnn"
}

require_fnn() {
  if [[ ! -x "$(fnn_bin)" ]]; then
    echo "fnn is not installed. Run ./scripts/setup-nodes.sh first." >&2
    exit 1
  fi
}

pid_file() {
  echo "$NODES_DIR/$1.pid"
}

log_file() {
  echo "$NODES_DIR/$1.log"
}

node_dir() {
  echo "$NODES_DIR/$1"
}
