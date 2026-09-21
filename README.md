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

## Stage 3 — Path A

With the buyer node up, seller Release pays the buyer from Twine, then settles the hold. `S` stays in `daemon/order.json` and is never returned by `OrderView`.

Restart only the daemon after code changes (leave the three `fnn` processes alone), from the `daemon/` directory so `ORDER_FILE` stays `daemon/order.json`:

```bash
kill "$(cat nodes/daemon.pid)" 2>/dev/null || true
# or: lsof -tiTCP:8080 -sTCP:LISTEN | xargs kill

cd daemon && cargo build && ./target/debug/twine-daemon
# log: nodes/daemon.log when redirected; ORDER_FILE defaults to daemon/order.json
```

If the live order is already `Releasing` from stage 2, Release (or `POST /order/release`) continues Path A from that state. Sequence:

1. Buyer node `new_invoice` for the trade amount (normal invoice; Fiber generates the preimage on the buyer node).
2. Twine `send_payment` to that invoice.
3. Poll Twine `get_payment` until `Success` (not `Created` / `Inflight`).
4. Twine `settle_invoice(payment_hash, payment_preimage)` with hold `H` and sealed `S`.
5. Poll `get_invoice` until `Paid`. Order state `Settled`.

If step 2/3 fails, the daemon does **not** call `settle_invoice`.

```bash
# balances before / after (Twine → buyer local should dip by ~1 CKB; buyer local should rise by ~1 CKB)
curl -s http://127.0.0.1:8080/health | python3 -m json.tool

# Path A release (works from FiatSent or Releasing)
curl -s -X POST http://127.0.0.1:8080/order/release -H 'content-type: application/json' -d '{}'

# hold invoice must be Paid (replace H)
curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"get_invoice","params":[{"payment_hash":"H"}]}'

curl -s http://127.0.0.1:8080/order
```

App on the iPhone simulator:

```bash
cd app && flutter run -d 242B4280-AADB-418E-A0E9-F3C400EA7D57
```

Seller: **Release** (or **Release (continue path A)** when already `Releasing`). The log lists `pay:…` lines, then `settle:…`, then `path A complete: state Settled`. Invoice shows `Paid`. Relaunch the app: state stays `Settled`.

Acceptance: buyer channel local balance increased by the trade; hold `get_invoice` is `Paid`; Twine’s Twine→buyer local dipped for the outbound payment and Twine recovered on the seller channel when the hold settled; app log is pay then settle; `S` never appears in `GET /order`.

## Stage 4 — Path B

A failed payment to the buyer leaves the seller hold untouched. State becomes `Leg2Failed`. Do **not** call `cancel_invoice` on the Received hold. Retry is a separate action (`POST /order/retry`) that runs Path A.

Restart only the daemon after code changes. For the failure demo, set `TWINE_RELEASE_PAUSE_MS` so you can stop the buyer after `new_invoice` and before `send_payment` (otherwise the payment can finish before the kill lands):

```bash
kill "$(cat nodes/daemon.pid)" 2>/dev/null || true
# or: lsof -tiTCP:8080 -sTCP:LISTEN | xargs kill

cd daemon
TWINE_RELEASE_PAUSE_MS=4000 ./target/debug/twine-daemon
# log: nodes/daemon.log when redirected; ORDER_FILE defaults to daemon/order.json
```

Exact Path B sequence used on this machine (1 CKB / `0x5f5e100` shannon, currency `Fibt`):

```bash
# walk to FiatSent (buyer up; hold ends Received)
curl -s -X POST http://127.0.0.1:8080/order -H 'content-type: application/json' -d '{"amount":"1"}'
curl -s -X POST http://127.0.0.1:8080/order/demo_cancel -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/hold -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/lock -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/accept -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/fiat_sent -H 'content-type: application/json' -d '{}'

# balances before failure
curl -s http://127.0.0.1:8080/health | python3 -m json.tool

# release: buyer RPC stays up for new_invoice; disconnect P2P so send_payment cannot route
BUYER_PUB=$(curl -s http://127.0.0.1:8247 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"node_info","params":[]}' | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['pubkey'])")
curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"disconnect_peer\",\"params\":[{\"pubkey\":\"$BUYER_PUB\"}]}"
# optional alternative: TWINE_RELEASE_PAUSE_MS=4000 on the daemon, then kill only nodes/buyer.pid after "buyer invoice created"
curl -s -X POST http://127.0.0.1:8080/order/release -H 'content-type: application/json' -d '{}'

curl -s http://127.0.0.1:8080/order
# hold still Received (replace H):
curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"get_invoice","params":[{"payment_hash":"H"}]}'
curl -s http://127.0.0.1:8080/health | python3 -m json.tool

# bring buyer back on the graph, then retry (Path A)
curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"connect_peer\",\"params\":[{\"pubkey\":\"$BUYER_PUB\",\"address\":\"/ip4/127.0.0.1/tcp/8248\",\"save\":true}]}"
curl -s -X POST http://127.0.0.1:8080/order/retry -H 'content-type: application/json' -d '{}'
```

App on the iPhone simulator (`242B4280-AADB-418E-A0E9-F3C400EA7D57`):

```bash
cd app && flutter run -d 242B4280-AADB-418E-A0E9-F3C400EA7D57
```

On `Leg2Failed` the app shows the “submit a new invoice” message and **Retry with new invoice** (Seller or Buyer). Release stays on `FiatSent` / `Releasing` only. After retry the order is `Settled` / invoice `Paid`; terminate + launch again keeps that order. `S` never appears in `GET /order`.

Acceptance: during the failure, Twine’s balance does not gain the trade and the failure log has no `settle_invoice`. After retry, Path A’s checks pass (buyer channel up by the trade, hold `Paid`, Twine dipped then recovered, log is pay then settle). The failure log also says that if the buyer never returns, the seller is refunded when the TLC expires.

## Stage 5 — Path C

The solver picks a side from the app before the 16 hour timelock. Chat is plain text on the daemon (no Nostr). `OrderView` / `GET /order` never includes preimage `S`.

Restart only the daemon after code changes (leave the three `fnn` processes alone):

```bash
kill "$(cat nodes/daemon.pid)" 2>/dev/null || true
# or: lsof -tiTCP:8080 -sTCP:LISTEN | xargs kill

cd daemon && cargo build
# start from daemon/ so ORDER_FILE stays daemon/order.json; write pid with an absolute path
( cd daemon && ./target/debug/twine-daemon >> ../nodes/daemon.log 2>&1 & echo $! > "$(pwd)/../nodes/daemon.pid" )
# confirm: curl -s http://127.0.0.1:8080/order  (must include "chat")
```

Open dispute only from `WaitingFiat`, `FiatSent`, or `Leg2Failed`, and only while `get_invoice` is still `Received`. Buyer and seller post chat lines. Solver awards one side.

| Action | HTTP |
| --- | --- |
| Open dispute | `POST /order/dispute` |
| Chat line | `POST /order/chat` body `{"from":"buyer"|"seller","text":"…"}` |
| Award buyer | `POST /order/award_buyer` (Path A: pay then `settle_invoice`; route fail → stay `Disputed`, no settle) |
| Award seller | `POST /order/award_seller` (no `settle_invoice`, no `cancel_invoice`; log says refund at TLC expiry) |

If the hold is already `Expired`, both awards return 400 and the app shows that. Do not wait 16 hours here (that is stage 6).

**Order of the two live trades (1 CKB / `0x5f5e100` shannon, `Fibt`):** a seller-wins order stays `Disputed` / `Received`, so `Order::is_open()` stays true and blocks create. Run **buyer-wins first**, then seller-wins. Confirm `GET /order` is closed (`Settled`) before creating.

Exact commands used on this machine:

```bash
# confirm create is allowed (previous stage-4 order Settled)
curl -s http://127.0.0.1:8080/order
curl -s http://127.0.0.1:8080/health | python3 -m json.tool

# --- buyer-wins FIRST ---
curl -s -X POST http://127.0.0.1:8080/order -H 'content-type: application/json' -d '{"amount":"1"}'
curl -s -X POST http://127.0.0.1:8080/order/hold -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/lock -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/accept -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/fiat_sent -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/dispute -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/chat -H 'content-type: application/json' \
  -d '{"from":"buyer","text":"I sent the fiat already"}'
curl -s -X POST http://127.0.0.1:8080/order/chat -H 'content-type: application/json' \
  -d '{"from":"seller","text":"I never got it"}'
curl -s http://127.0.0.1:8080/health | python3 -m json.tool
curl -s -X POST http://127.0.0.1:8080/order/award_buyer -H 'content-type: application/json' -d '{}'
# hold must be Paid (replace H):
curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"get_invoice","params":[{"payment_hash":"H"}]}'
curl -s http://127.0.0.1:8080/order
curl -s http://127.0.0.1:8080/health | python3 -m json.tool

# --- seller-wins SECOND (leave Disputed / Received; do not expire) ---
curl -s -X POST http://127.0.0.1:8080/order -H 'content-type: application/json' -d '{"amount":"1"}'
curl -s -X POST http://127.0.0.1:8080/order/hold -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/lock -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/accept -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/dispute -H 'content-type: application/json' -d '{}'
curl -s -X POST http://127.0.0.1:8080/order/chat -H 'content-type: application/json' \
  -d '{"from":"seller","text":"buyer ghosted me"}'
curl -s -X POST http://127.0.0.1:8080/order/chat -H 'content-type: application/json' \
  -d '{"from":"buyer","text":"that is not true"}'
curl -s -X POST http://127.0.0.1:8080/order/award_seller -H 'content-type: application/json' -d '{}'
curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"get_invoice","params":[{"payment_hash":"H"}]}'
curl -s http://127.0.0.1:8080/order
```

Live results on this machine:

- Buyer-wins hold `0x1dd17b11d2f8424239277fdfb60b290b8d21de180dcf98601153ec247cdd4dd5` → state `Settled`, `get_invoice` `Paid`. Log is pay then settle (`path C buyer wins complete`). Twine→buyer local dipped 396→395 CKB (buyer remote 5→6); seller→Twine local 395→394 after settle recovered Twine’s side of the hold.
- Seller-wins hold `0x9e5df1a4bb73f69965e6aefb1e7b15e83542d6994162f6a4934f52cabe021ce1` → state `Disputed`, `get_invoice` still `Received`. Log: `settle_invoice not called; cancel_invoice not called; seller refund at TLC expiry`. Daemon stderr: `(no settle, no cancel)`.
- Chat lines visible on Solver in the app. Route-fail stay-disputed covered by Rust unit test `award_buyer_pay_failure_stays_disputed`. Expired award refusal covered by `expired_invoice_refuses_both_awards` and the app error path.

App on the iPhone simulator (`242B4280-AADB-418E-A0E9-F3C400EA7D57`):

```bash
cd app && flutter run -d 242B4280-AADB-418E-A0E9-F3C400EA7D57
```

| Role | Path C actions |
| --- | --- |
| Seller / Buyer | Open dispute (from WaitingFiat / FiatSent / Leg2Failed) → Post chat line |
| Solver | Sees chat → Award buyer / Award seller |

On seller-wins the app shows the TLC-expiry line. Terminate + launch again: same `Disputed` / `Received` order and chat. `S` never appears in `GET /order`.

## Stage 6 — Path D

A held invoice that nobody settles expires on testnet. Fiber refunds the seller. There is no virtual clock.

**`final_expiry_delta` probed on this fnn v0.9.1 Twine node:** `57_600_000` ms (`0x36ee800`, Fiber’s documented 16-hour minimum). Throwaway `new_invoice` with that value was accepted; attribute `final_htlc_minimum_expiry_delta` came back as `0x36ee800`. The probe invoice was then `cancel_invoice`’d while still `Open`. `create_hold` now always sends that delta.

Restart only the daemon after code changes (leave the three `fnn` processes alone):

```bash
kill "$(cat nodes/daemon.pid)" 2>/dev/null || true
# or: lsof -tiTCP:8080 -sTCP:LISTEN | xargs kill

cd daemon && cargo build
# start from daemon/ so ORDER_FILE stays daemon/order.json; write pid with an absolute path
( cd daemon && unset CARGO_TARGET_DIR && ./target/debug/twine-daemon >> ../nodes/daemon.log 2>&1 & echo $! > "$(pwd)/../nodes/daemon.pid" )
```

The daemon polls Twine `get_invoice` every 15s while the order is open and the hold is still watchable (`Held` / `WaitingFiat` / `FiatSent` / `Leg2Failed` / `Disputed` / `Releasing`). When status becomes `Expired`:

1. Order state → `Expired` (create is allowed again).
2. Log: seller payment failed back; seller refunded because the TLC expired.
3. `cancel_invoice` is **not** called.
4. One `settle_invoice(H, S)` attempt is made and logged as a **failed** settle (not a success).
5. Seller→twine local balance while in flight vs after expiry is logged.

The Flutter app polls `GET /order` every 5s while those states are live, so `Expired` and the TLC-expiry line appear without a manual refresh.

### Live seller-wins hold from Stage 5 (node default delta, not 16h)

This hold was created **before** `final_expiry_delta` was passed (fnn default `0x927c00` = 9_600_000 ms = 160 minutes).

| Field | Value |
| --- | --- |
| H | `0x9e5df1a4bb73f69965e6aefb1e7b15e83542d6994162f6a4934f52cabe021ce1` |
| Invoice timestamp | `0x1a0c5f4cc21` → `2026-09-21T21:52:29.729Z` |
| Expected expiry | ~`2026-09-22T00:32:29Z` |
| State at Stage 6 start | `Disputed` / `Received` |

Do **not** settle or cancel this invoice. Leave the daemon polling. After it expires, re-check:

```bash
H=0x9e5df1a4bb73f69965e6aefb1e7b15e83542d6994162f6a4934f52cabe021ce1

curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"get_invoice\",\"params\":[{\"payment_hash\":\"$H\"}]}"
# expect status Expired

curl -s http://127.0.0.1:8080/order
# expect state Expired; log has path D lines; no successful settle/cancel for this H

# settle must fail (replace S from daemon/order.json if you probe by hand; the daemon already tries once)
curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"settle_invoice\",\"params\":[{\"payment_hash\":\"$H\",\"payment_preimage\":\"S\"}]}"

curl -s http://127.0.0.1:8080/health | python3 -m json.tool
# seller→twine local should be restored vs the in-flight value in the order log

grep -E 'path D|settle|cancel' nodes/daemon.log | tail -40
```

### Explicit 16-hour Path D hold (blocked until the live order expires)

**Not started yet.** The Stage 5 seller-wins order is still open (`Disputed` / `Received`), so `Order::is_open()` refuses create. Expected Fiber expiry for that hold is ~`2026-09-22T00:32:29Z`. Do not sleep-wait; leave the daemon polling. After `GET /order` shows `Expired`, run:

```bash
curl -s -X POST http://127.0.0.1:8080/order -H 'content-type: application/json' -d '{"amount":"1"}'
curl -s -X POST http://127.0.0.1:8080/order/hold -H 'content-type: application/json' -d '{}'
# log should include final_expiry_delta=57600000ms (0x36ee800)
curl -s -X POST http://127.0.0.1:8080/order/lock -H 'content-type: application/json' -d '{}'
# optional: accept / dispute; do not settle; do not cancel
curl -s http://127.0.0.1:8080/order
# record H and expiry = invoice timestamp + 16h

curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"get_invoice","params":[{"payment_hash":"H"}]}'
```

Leave the daemon and app running; check again after the 16-hour wait (next day). Acceptance: `Expired`, later `settle_invoice` fails, seller channel balance restored, log has no successful settle or cancel for that H.

App on the iPhone simulator (`242B4280-AADB-418E-A0E9-F3C400EA7D57`):

```bash
cd app && flutter run -d 242B4280-AADB-418E-A0E9-F3C400EA7D57
```

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
