#!/usr/bin/env bash
# Connect the three nodes and open seller -> twine and twine -> buyer.
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

FUNDING_SHANNONS="${FUNDING_SHANNONS:-50000000000}"
# Fiber auto-accepts by adding about 99 CKB on the counterparty side.
SELLER_MIN_SHANNONS=60000000000
TWINE_MIN_SHANNONS=70000000000
BUYER_MIN_SHANNONS=20000000000
SIGHASH_CODE_HASH="0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8"

pubkey_of() {
  local body
  body="$(fiber_rpc "$(rpc_url "$1")" node_info '[]')"
  if ! jq -e '.result.pubkey' <<<"$body" >/dev/null; then
    echo "$1 node_info failed: $body" >&2
    exit 1
  fi
  jq -r '.result.pubkey' <<<"$body"
}

connect_to() {
  local from="$1"
  local to="$2"
  local to_pubkey="$3"
  local body
  body="$(fiber_rpc "$(rpc_url "$from")" connect_peer \
    "$(jq -n --arg pubkey "$to_pubkey" --arg address "$(p2p_addr "$to")" \
      '[{pubkey:$pubkey, address:$address, save:true}]')")"
  if jq -e '.error' <<<"$body" >/dev/null; then
    echo "connect_peer $from -> $to failed: $(jq -r '.error.message' <<<"$body")" >&2
    exit 1
  fi
  echo "connected $from -> $to"
}

list_channels() {
  local from="$1"
  local params="$2"
  local body
  body="$(fiber_rpc "$(rpc_url "$from")" list_channels "$params")"
  if ! jq -e '.result.channels' <<<"$body" >/dev/null; then
    echo "list_channels on $from failed: $body" >&2
    exit 1
  fi
  printf '%s' "$body"
}

channels_with_peer() {
  local body="$1"
  local peer_pubkey="$2"
  jq -c --arg pubkey "$peer_pubkey" '
    .result.channels
    | map(select(
        (.funding_udt_type_script == null)
        and (((.pubkey // "") | ascii_downcase | ltrimstr("0x"))
          == ($pubkey | ascii_downcase | ltrimstr("0x")))
      ))
  ' <<<"$body"
}

channel_to_peer() {
  local from="$1"
  local peer_pubkey="$2"
  local body matches
  body="$(list_channels "$from" '[{}]')"
  matches="$(channels_with_peer "$body" "$peer_pubkey")"
  jq -c '.[0] // empty' <<<"$matches"
}

open_one() {
  local from="$1"
  local to="$2"
  local to_pubkey="$3"
  local existing funding_hex body
  existing="$(channel_to_peer "$from" "$to_pubkey")"
  if [[ -n "$existing" ]]; then
    echo "$from -> $to already has a channel ($(jq -r '.state.state_name' <<<"$existing"))"
    return 0
  fi
  funding_hex="$(printf '0x%x' "$FUNDING_SHANNONS")"
  echo "opening $from -> $to for $FUNDING_SHANNONS shannon ($funding_hex)"
  body="$(fiber_rpc "$(rpc_url "$from")" open_channel \
    "$(jq -n --arg pubkey "$to_pubkey" --arg amount "$funding_hex" \
      '[{pubkey:$pubkey, funding_amount:$amount, public:true}]')")"
  if jq -e '.error' <<<"$body" >/dev/null; then
    echo "open_channel $from -> $to failed: $(jq -r '.error.message' <<<"$body")" >&2
    echo "Fund $(jq -r --arg role "$from" '.[$role].address' "$ADDRESSES_FILE") at https://faucet.nervos.org" >&2
    exit 1
  fi
  echo "submitted $from -> $to ($(jq -r '.result.temporary_channel_id' <<<"$body"))"
}

pending_ids() {
  local from="$1"
  local peer_pubkey="$2"
  local body
  body="$(list_channels "$from" '[{"only_pending": true}]')"
  channels_with_peer "$body" "$peer_pubkey" | jq -r '.[].channel_id'
}

wait_ready() {
  local from="$1"
  local to="$2"
  local to_pubkey="$3"
  local known="$4"
  local i channel state failed detail
  for i in $(seq 1 60); do
    channel="$(channel_to_peer "$from" "$to_pubkey")"
    if [[ -n "$channel" ]]; then
      state="$(jq -r '.state.state_name' <<<"$channel")"
      echo "$from -> $to state=$state"
      if [[ "$state" == "ChannelReady" || "$state" == "CHANNEL_READY" ]]; then
        jq -r '"balances local=\(.local_balance) remote=\(.remote_balance)"' <<<"$channel"
        return 0
      fi
    else
      echo "$from -> $to has no live channel yet"
    fi
    failed="$(list_channels "$from" '[{"only_pending": true}]')"
    detail="$(channels_with_peer "$failed" "$to_pubkey" | jq -r --arg known "$known" '
      map(select(.failure_detail != null and (.channel_id as $id | ($known | split(" ") | index($id) | not))))
      | .[0].failure_detail // empty
    ')"
    if [[ -n "$detail" ]]; then
      echo "$from -> $to failed: $detail" >&2
      exit 1
    fi
    sleep 10
  done
  echo "$from -> $to did not reach ChannelReady" >&2
  exit 1
}

wallet_shannon() {
  local role="$1"
  local lock_arg body
  lock_arg="$(jq -r --arg role "$role" '.[$role].lock_arg' "$ADDRESSES_FILE")"
  body="$(curl -sS -m 20 -H "Content-Type: application/json" \
    --data "$(jq -n --arg args "$lock_arg" --arg code "$SIGHASH_CODE_HASH" '{
      jsonrpc:"2.0",
      id:1,
      method:"get_cells_capacity",
      params:[{
        script:{code_hash:$code, hash_type:"type", args:$args},
        script_type:"lock"
      }]
    }')" \
    "$CKB_RPC")"
  jq -r '.result.capacity // "0x0"' <<<"$body"
}

require_funded() {
  local role="$1"
  local minimum="$2"
  local raw shannon
  raw="$(wallet_shannon "$role")"
  shannon="$(python3 -c 'print(int("'"$raw"'", 16))')"
  echo "$role wallet capacity: $shannon shannon"
  if [[ "$shannon" -lt "$minimum" ]]; then
    echo "$role needs at least $minimum shannon before open_channel." >&2
    echo "Send testnet CKB to $(jq -r --arg role "$role" '.[$role].address' "$ADDRESSES_FILE")" >&2
    echo "Faucet: https://faucet.nervos.org" >&2
    exit 1
  fi
}

if [[ ! -f "$ADDRESSES_FILE" ]]; then
  echo "missing $ADDRESSES_FILE. Run ./scripts/setup-nodes.sh first." >&2
  exit 1
fi

require_funded seller "$SELLER_MIN_SHANNONS"
require_funded twine "$TWINE_MIN_SHANNONS"
require_funded buyer "$BUYER_MIN_SHANNONS"

seller_pk="$(pubkey_of seller)"
twine_pk="$(pubkey_of twine)"
buyer_pk="$(pubkey_of buyer)"

connect_to seller twine "$twine_pk"
connect_to twine seller "$seller_pk"
connect_to twine buyer "$buyer_pk"
connect_to buyer twine "$twine_pk"

seller_known="$(pending_ids seller "$twine_pk" | tr '\n' ' ')"
open_one seller twine "$twine_pk"
wait_ready seller twine "$twine_pk" "$seller_known"

twine_known="$(pending_ids twine "$buyer_pk" | tr '\n' ' ')"
open_one twine buyer "$buyer_pk"
wait_ready twine buyer "$buyer_pk" "$twine_known"

echo "both channels are ChannelReady"
