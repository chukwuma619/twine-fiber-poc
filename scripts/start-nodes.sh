#!/usr/bin/env bash
# Start seller, twine, and buyer, then print their RPC URLs and pubkeys.
set -euo pipefail
set -m

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

require_fnn

if [[ ! -f "$PASSWORD_FILE" ]]; then
  echo "missing $PASSWORD_FILE. Run ./scripts/setup-nodes.sh first." >&2
  exit 1
fi

password="$(cat "$PASSWORD_FILE")"
export NO_PROXY="${NO_PROXY:-127.0.0.1,localhost}"

start_one() {
  local role="$1"
  local pidfile logfile dir
  pidfile="$(pid_file "$role")"
  logfile="$(log_file "$role")"
  dir="$(node_dir "$role")"
  if [[ ! -f "$dir/config.yml" || ! -f "$dir/ckb/key" ]]; then
    echo "$role is not set up. Run ./scripts/setup-nodes.sh first." >&2
    exit 1
  fi
  if [[ -f "$pidfile" ]] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
    echo "$role already running (pid $(cat "$pidfile"))"
    return 0
  fi
  echo "starting $role"
  nohup env FIBER_SECRET_KEY_PASSWORD="$password" RUST_LOG=info \
    "$(fnn_bin)" -c "$dir/config.yml" -d "$dir" \
    >"$logfile" 2>&1 &
  echo $! >"$pidfile"
  disown
}

wait_for_rpc() {
  local role="$1"
  local url i body
  url="$(rpc_url "$role")"
  for i in $(seq 1 90); do
    body="$(fiber_rpc "$url" node_info '[]' 2>/dev/null || true)"
    if [[ -n "$body" ]] && jq -e '.result.pubkey' <<<"$body" >/dev/null 2>&1; then
      jq -r --arg role "$role" --arg url "$url" \
        '"\($role)\t\($url)\t\(.result.pubkey)"' <<<"$body"
      return 0
    fi
    sleep 1
  done
  echo "$role did not answer node_info at $url" >&2
  echo "last log lines:" >&2
  tail -n 40 "$(log_file "$role")" >&2 || true
  exit 1
}

for role in seller twine buyer; do
  start_one "$role"
done

echo
echo "role    rpc                      pubkey"
for role in seller twine buyer; do
  wait_for_rpc "$role"
done

if [[ -f "$ADDRESSES_FILE" ]]; then
  echo
  echo "CKB testnet addresses (faucet https://faucet.nervos.org):"
  jq -r 'to_entries[] | "\(.key)\t\(.value.address)"' "$ADDRESSES_FILE"
fi

echo
echo "Daemon defaults: SELLER_RPC=$(rpc_url seller) TWINE_RPC=$(rpc_url twine) BUYER_RPC=$(rpc_url buyer)"
echo "Start it with: cd daemon && cargo run"
echo "Then: curl -s http://127.0.0.1:8080/health"
