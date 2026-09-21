# Twine Fiber POC

A testnet proof of concept. One Rust daemon, one Flutter app, real Fiber testnet CKB. Rough on purpose. It exists so someone can create an order and walk the four payment paths.

Build order: [PLAN.md](PLAN.md).

```text
twine-fiber-poc/
  daemon/     Rust coordinator. Talks to fnn over JSON-RPC. Holds preimage S.
  app/        Flutter client. Seller, Buyer, and Solver roles.
  scripts/    Download fnn, start the three testnet nodes, open the two channels.
```

Three Fiber testnet nodes sit beside the daemon: seller, Twine, and buyer. The app never holds a Fiber key. It calls the daemon. The daemon calls `fnn`.

Fiat is a button. The CKB movement is real: `new_invoice`, `send_payment`, `settle_invoice`, and TLC expiry on testnet (`Fibt`).

## Stage 0

Three Fiber v0.9.1 testnet nodes (seller, Twine, buyer) and the daemon. Node data, keys, and the fnn binary stay out of git.

```bash
./scripts/setup-nodes.sh
./scripts/start-nodes.sh
```

`setup-nodes.sh` prints a testnet address for each node. Send CKB from [https://faucet.nervos.org](https://faucet.nervos.org) to all three. Seller needs at least 600 CKB and Twine at least 700 CKB, for a 500 CKB channel plus a change cell. The buyer needs at least 200 CKB: Fiber auto-accepts an incoming channel by locking about 99 CKB on that side.

```bash
./scripts/open-channels.sh
cd daemon && cargo run
curl -s http://127.0.0.1:8080/health
```

`open-channels.sh` connects the nodes and opens seller → Twine and Twine → buyer, 500 CKB each, then waits until both are `ChannelReady`.

`GET /health` returns the three pubkeys and both channel balances. `ready` is true when those channels are `ChannelReady`. The daemon reads `SELLER_RPC`, `TWINE_RPC`, and `BUYER_RPC` (defaults `http://127.0.0.1:8227`, `:8237`, and `:8247`).

`./scripts/stop-nodes.sh` stops the daemon and the three nodes. The node password is `nodes/password`.

## Stage 1

The app creates one order. No Fiber payment yet. The daemon stores it in `order.json` in the daemon working directory (`ORDER_FILE` overrides that path).

```bash
cd daemon && cargo run
cd app && flutter run
```

The screen has an amount field, Create order, a Seller / Buyer / Solver switch, and the order log. Create moves the order from `Idle` to `Pending`. Restart the app and it loads the same order from `GET /order`.

On the iOS simulator the daemon URL is `http://127.0.0.1:8080`. On the Android emulator use `http://10.0.2.2:8080`. The app fills that in. A phone on the same network needs `LISTEN=0.0.0.0:8080` and the Mac's LAN address.

## Stage 2

Seller’s testnet CKB is held in a Fiber hold invoice. Preimage `S` is generated in the daemon; `H = SHA256(S)`. The Twine node gets `new_invoice` with `payment_hash` and `hash_algorithm: sha256` (no `payment_preimage`). `S` never leaves the daemon.

Restart only the daemon after code changes (leave the three `fnn` processes alone):

```bash
# if an old daemon is still bound to :8080
kill "$(cat nodes/daemon.pid)" 2>/dev/null || true
# or: lsof -tiTCP:8080 -sTCP:LISTEN | xargs kill

cd daemon && cargo build && ./target/debug/twine-daemon
# log: nodes/daemon.log when started via nohup; ORDER_FILE defaults to daemon/order.json
```

App on the iPhone simulator (the device that worked for stage 1):

```bash
cd app && flutter run -d 242B4280-AADB-418E-A0E9-F3C400EA7D57
```

Buttons by role:

| Role | Actions |
| --- | --- |
| Any | Create order → Demo cancel unpaid invoice → Create hold invoice |
| Seller | Lock → Try cancel (skipped after Received) → Release |
| Buyer | Accept (starts a 3 minute fiat timer) → Fiat sent |

HTTP shape (daemon only talks to `fnn`):

```bash
curl -s http://127.0.0.1:8080/order
curl -s -X POST http://127.0.0.1:8080/order/demo_cancel -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/hold -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/lock -H 'content-type: application/json' -d '{}'
# confirm hold (replace H):
curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"get_invoice","params":[{"payment_hash":"H"}]}'
curl -s -X POST http://127.0.0.1:8080/order/try_cancel -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/accept -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/fiat_sent -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/release -H 'content-type: application/json' -d '{}'
```

Acceptance for this stage: `get_invoice` is `Received`, seller `get_payment` stays `Inflight`, Twine’s spendable outbound balance (Twine → buyer) has not gained the trade amount, and the app / order log shows `H=…` with `S sealed in daemon`. Stage 2 `Release` sets state `Releasing` and does **not** call `settle_invoice`.

`POST /order/demo_cancel` creates a throwaway unpaid invoice and cancels it so the live order can stay lockable. After `Received`, `POST /order/try_cancel` refuses to call `cancel_invoice` on the live hold (calling it can destroy the trade); the log says refund is TLC expiry.

Relaunch the app: `GET /order` still returns `Held` or later (`WaitingFiat` / `FiatSent` / `Releasing`).

## What a reviewer can do

1. Create a sell order for a small amount of testnet CKB.
2. Lock it in a Fiber hold invoice. The coins stay in the seller’s channel. Twine’s spendable balance does not increase.
3. Mark fiat sent and release, or open a dispute.
4. Watch one of the four endings on testnet.

| Path | What they do | What Fiber does |
| --- | --- | --- |
| A | Buyer is reachable. Seller releases | Daemon pays the buyer’s invoice, then `settle_invoice(H, S)` |
| B | Buyer is offline | Payment to the buyer fails. Daemon does not settle. Hold stays `Received`. A new invoice can retry path A |
| C | Either side disputes. Solver reads the chat | Buyer wins runs path A. Seller wins leaves the hold unsettled |
| D | Nobody settles before the timelock | Fiber expires the TLC and the seller is refunded. Shortest `final_expiry_delta` is 16 hours |

`cancel_invoice` only succeeds while the invoice is still `Open`. After the seller pays, the status is `Received`, and the refund is expiry.
