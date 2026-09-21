#!/usr/bin/env bash
# Download fnn v0.9.1 and create seller, twine, and buyer testnet node dirs.
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

asset_name() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os-$arch" in
    Darwin-arm64) echo "fnn_v${FNN_VERSION}-aarch64-darwin-portable.tar.gz" ;;
    Darwin-x86_64) echo "fnn_v${FNN_VERSION}-x86_64-darwin-portable.tar.gz" ;;
    Linux-aarch64 | Linux-arm64) echo "fnn_v${FNN_VERSION}-aarch64-linux-portable.tar.gz" ;;
    Linux-x86_64) echo "fnn_v${FNN_VERSION}-x86_64-linux-portable.tar.gz" ;;
    *)
      echo "no fnn v${FNN_VERSION} bundle for $os $arch" >&2
      return 1
      ;;
  esac
}

asset_sha256() {
  case "$1" in
    fnn_v0.9.1-aarch64-darwin-portable.tar.gz)
      echo "4a330134fe71053c3d85b56e1003aa94c53e147836cd013f5a6415fdac84f63d"
      ;;
    fnn_v0.9.1-x86_64-darwin-portable.tar.gz)
      echo "2936136bcd310055dc111c3f46b2c98f062d062e704073e4b7fe4a142e20f203"
      ;;
    fnn_v0.9.1-aarch64-linux-portable.tar.gz)
      echo "c52c99b06bd954244b3fb91634d5b5031c23170208e46c7738ee308cb0bd3f0c"
      ;;
    fnn_v0.9.1-x86_64-linux-portable.tar.gz)
      echo "6425bc90ce971a5b8f10bd93667227cc932ee2e50a70f584e2c07590ce446d0d"
      ;;
    *)
      echo "no checksum for $1" >&2
      return 1
      ;;
  esac
}

install_fnn() {
  local asset archive expected actual extracted
  asset="$(asset_name)"
  expected="$(asset_sha256 "$asset")"
  mkdir -p "$BIN_DIR"
  archive="$BIN_DIR/$asset"
  if [[ ! -f "$archive" ]]; then
    echo "downloading $asset"
    curl --fail --location --retry 5 --output "$archive" \
      "https://github.com/nervosnetwork/fiber/releases/download/v${FNN_VERSION}/$asset"
  fi
  actual="$(shasum -a 256 "$archive" | awk '{print $1}')"
  if [[ "$actual" != "$expected" ]]; then
    echo "checksum mismatch for $asset" >&2
    echo "expected $expected" >&2
    echo "actual   $actual" >&2
    exit 1
  fi
  rm -rf "$BIN_DIR/unpack"
  mkdir -p "$BIN_DIR/unpack"
  tar -xzf "$archive" -C "$BIN_DIR/unpack"
  extracted="$(find "$BIN_DIR/unpack" -type f -name fnn -print -quit)"
  if [[ -z "$extracted" ]]; then
    echo "archive has no fnn binary" >&2
    exit 1
  fi
  cp "$extracted" "$BIN_DIR/fnn"
  cp "$(dirname "$extracted")/fnn-cli" "$BIN_DIR/fnn-cli"
  chmod +x "$BIN_DIR/fnn" "$BIN_DIR/fnn-cli"
  if [[ -d "$(dirname "$extracted")/config/testnet" ]]; then
    rm -rf "$BIN_DIR/config"
    cp -R "$(dirname "$extracted")/config" "$BIN_DIR/config"
  elif [[ -d "$BIN_DIR/unpack/config/testnet" ]]; then
    rm -rf "$BIN_DIR/config"
    cp -R "$BIN_DIR/unpack/config" "$BIN_DIR/config"
  fi
  if [[ ! -f "$BIN_DIR/config/testnet/config.yml" ]]; then
    echo "archive has no config/testnet/config.yml" >&2
    exit 1
  fi
  xattr -d com.apple.quarantine "$BIN_DIR/fnn" "$BIN_DIR/fnn-cli" 2>/dev/null || true
  echo "fnn $("$BIN_DIR/fnn" --version)"
}

write_node_config() {
  local role="$1"
  local rpc p2p
  rpc="$(rpc_port "$role")"
  p2p="$(p2p_port "$role")"
  python3 - "$BIN_DIR/config/testnet/config.yml" "$(node_dir "$role")/config.yml" "$role" "$rpc" "$p2p" <<'PY'
import pathlib, sys
src, dest, role, rpc, p2p = sys.argv[1:]
text = pathlib.Path(src).read_text()
old_p2p = '"/ip4/0.0.0.0/tcp/8228"'
new_p2p = f'"/ip4/127.0.0.1/tcp/{p2p}"'
if old_p2p not in text:
    raise SystemExit(f"testnet config is missing {old_p2p}")
text = text.replace(old_p2p, new_p2p, 1)
old_rpc = 'listening_addr: "127.0.0.1:8227"'
new_rpc = f'listening_addr: "127.0.0.1:{rpc}"'
if old_rpc not in text:
    raise SystemExit(f"testnet config is missing {old_rpc}")
text = text.replace(old_rpc, new_rpc, 1)
text = text.replace("announce_listening_addr: true", "announce_listening_addr: false", 1)
needle = f"listening_addr: {new_p2p}\n"
insert = needle + f'  announced_node_name: "twine-poc-{role}"\n'
if needle not in text:
    raise SystemExit("could not insert announced_node_name")
text = text.replace(needle, insert, 1)
path = pathlib.Path(dest)
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(text)
PY
}

ensure_password() {
  mkdir -p "$NODES_DIR"
  if [[ ! -f "$PASSWORD_FILE" ]]; then
    openssl rand -hex 24 > "$PASSWORD_FILE"
    chmod 600 "$PASSWORD_FILE"
  fi
}

ensure_key() {
  local role="$1"
  local key_dir key_file
  key_dir="$(node_dir "$role")/ckb"
  key_file="$key_dir/key"
  mkdir -p "$key_dir"
  if [[ -f "$key_file" ]]; then
    return 0
  fi
  openssl rand -hex 32 > "$key_file"
  chmod 600 "$key_file"
}

record_address() {
  local role="$1"
  local key_file info
  key_file="$(node_dir "$role")/ckb/key"
  if [[ -f "$ADDRESSES_FILE" ]] && jq -e --arg role "$role" '.[$role].address' "$ADDRESSES_FILE" >/dev/null; then
    return 0
  fi
  info="$(ckb-cli util key-info --local-only --privkey-path "$key_file" --output-format json 2>/dev/null)"
  INFO="$info" python3 - "$ADDRESSES_FILE" "$role" "$(rpc_url "$role")" "$(p2p_addr "$role")" <<'PY'
import json, os, pathlib, sys
dest, role, rpc, p2p = sys.argv[1:]
raw = os.environ["INFO"]
data = json.loads(raw[raw.index("{") :])
path = pathlib.Path(dest)
book = {}
if path.exists():
    book = json.loads(path.read_text())
book[role] = {
    "address": data["address"]["testnet"],
    "lock_arg": data["lock_arg"],
    "rpc": rpc,
    "p2p": p2p,
}
path.write_text(json.dumps(book, indent=2) + "\n")
PY
  chmod 600 "$ADDRESSES_FILE"
}

install_fnn
ensure_password
for role in seller twine buyer; do
  ensure_key "$role"
  write_node_config "$role"
  record_address "$role"
done

echo
echo "Fiber testnet nodes are ready. Fund seller and twine at https://faucet.nervos.org"
echo "Each needs at least 600 CKB: 500 CKB for the channel, plus a change cell and fee."
echo
jq -r 'to_entries[] | "\(.key)\t\(.value.address)\t\(.value.rpc)"' "$ADDRESSES_FILE"
echo
echo "Next: ./scripts/start-nodes.sh"
