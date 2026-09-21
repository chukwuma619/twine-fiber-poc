# Twine Fiber POC

A testnet proof of concept for a Fiber hold-invoice trade. One Flutter app is the client. One Rust daemon is the coordinator. Three Fiber nodes (seller, Twine, buyer) move real testnet CKB.

The app never holds a Fiber key. Fiat is a button. The CKB movement is `new_invoice`, `send_payment`, `settle_invoice`, and TLC expiry on testnet (`Fibt`).

```text
twine-fiber-poc/
  daemon/     Rust. HTTP API for the app. JSON-RPC to three fnn processes. Holds preimage S.
  app/        Flutter. One screen. Role switch: Seller, Buyer, Solver.
  scripts/    Download fnn, start the three testnet nodes, open the two channels.
```

Node data, keys, the `fnn` binary, and `daemon/order.json` stay out of git.

## What a reviewer can do

1. Create a sell order for a small amount of testnet CKB.
2. Lock it in a Fiber hold invoice. The coins stay in the seller’s channel. Twine’s spendable balance does not increase.
3. Mark fiat sent and release, or open a dispute.
4. Watch one of the four endings on testnet.

```text
[ Seller node ]              [ Twine daemon ]                    [ Buyer node ]

1. Create hold invoice (H)          daemon calls twine fnn new_invoice
2. Lock HTLC, status Received       seller fnn send_payment
3. Order accepted                   buyer, in the app
4. Fiat window                      a button. Off-chain. Three minutes in the app
5. Fiat sent                        buyer, in the app
6. Seller releases                  seller, in the app
```

| Path | What they do | What Fiber does |
| --- | --- | --- |
| A | Buyer is reachable. Seller releases | Daemon pays the buyer’s invoice, then `settle_invoice(H, S)`. Invoice `Paid`. State `Settled` |
| B | Payment to the buyer cannot route | `send_payment` fails. No `settle_invoice`. Hold stays `Received`. State `Leg2Failed`. A new invoice retries path A |
| C | Either side disputes. Solver reads the chat | Buyer wins runs path A. If that payment fails, the order stays `Disputed` and is not settled. Seller wins leaves the hold `Received` until expiry |
| D | Nobody settles before the timelock | Fiber marks the invoice `Expired` and the seller is refunded. A later `settle_invoice` fails. `cancel_invoice` is not called |

One order at a time. Create is allowed again when the order is `Idle`, `Cancelled`, `Paid`, `Settled`, or `Expired`.

## How the hold works

The daemon generates preimage `S`, sets `H = SHA256(S)`, and creates the hold invoice with `payment_hash` and `hash_algorithm: sha256`. `payment_preimage` is omitted. `S` stays in `order.json` and is never returned by `GET /order`.

Currency is `Fibt`. The app amount is CKB. One CKB is `0x5f5e100` shannon. Use a small amount (1 CKB is enough).

| Moment | RPC | Invoice status |
| --- | --- | --- |
| Create hold | Twine `new_invoice` | `Open` |
| Lock | Seller `send_payment` | `Received`. Coins are held in the seller’s channel |
| Unpaid cancel | `cancel_invoice` | `Cancelled`. Only legal while `Open` |
| Path A | Twine `send_payment` to the buyer invoice, then `settle_invoice(H, S)` | `Paid` |
| Path B | Buyer payment fails | stays `Received`. No `settle_invoice` |
| Path C, seller wins | No `settle_invoice` | stays `Received` until expiry |
| Path D | Nobody settles. `final_expiry_delta` passes | `Expired`. Seller payment returns |

`cancel_invoice` on a `Received` invoice is refused. The app’s “Try cancel” button after lock only writes that to the log. Seller-wins and “buyer never returns” share path D’s refund: the TLC expires. The shortest `final_expiry_delta` Fiber accepts is 16 hours (`57_600_000` ms, `0x36ee800`). Every hold uses that. There is no virtual clock. The daemon polls `get_invoice` every 15 seconds; the app polls `GET /order` every 5 seconds while a hold can still expire.

Leg 2 comes first. The daemon calls `settle_invoice` only after `send_payment` to the buyer reports `Success`. Twine pays that invoice from its own outbound channel, because the seller’s coins are not spendable until settle.

## Requirements

- macOS or Linux (the setup script downloads a Fiber v0.9.1 portable bundle for that host)
- `curl`, `jq`, `python3`, `openssl`
- [ckb-cli](https://github.com/nervosnetwork/ckb-cli) on `PATH` (used once, to print testnet addresses)
- Rust (stable) and Cargo
- Flutter (the app targets Dart 3.11)

## Run it

```bash
./scripts/setup-nodes.sh
```

That downloads `fnn` v0.9.1, writes a random node password to `nodes/password`, and prints a testnet address for seller, Twine, and buyer. Send CKB from [https://faucet.nervos.org](https://faucet.nervos.org) to all three before opening channels:

| Node | Minimum | Why |
| --- | --- | --- |
| Seller | 600 CKB | 500 CKB channel to Twine, plus a change cell and fee |
| Twine | 700 CKB | Accepts the seller channel and funds a 500 CKB channel to the buyer |
| Buyer | 200 CKB | Fiber auto-accepts an incoming channel by locking about 99 CKB on that side |

```bash
./scripts/start-nodes.sh
./scripts/open-channels.sh
cd daemon && cargo run
```

`start-nodes.sh` starts the three `fnn` processes and prints their RPC URLs and pubkeys. `open-channels.sh` connects them and opens seller → Twine and Twine → buyer, 500 CKB each (`FUNDING_SHANNONS` overrides the amount), then waits until both are `ChannelReady`.

```bash
curl -s http://127.0.0.1:8080/health
```

`ready` is true when all three pubkeys answer and both channels are open. Defaults are `SELLER_RPC=http://127.0.0.1:8227`, `TWINE_RPC=http://127.0.0.1:8237`, `BUYER_RPC=http://127.0.0.1:8247`. Start the daemon from `daemon/` so the order file is `daemon/order.json`.

```bash
cd app && flutter run
```

On the iOS simulator the daemon URL is `http://127.0.0.1:8080`. On the Android emulator use `http://10.0.2.2:8080`. The app fills that in. A phone on the same network needs `LISTEN=0.0.0.0:8080` and the machine’s LAN address.

```bash
./scripts/stop-nodes.sh
```

That stops the daemon (if `nodes/daemon.pid` exists) and the three nodes.

## Walk it in the app

Switch role on the same screen. A second device pointed at the same daemon sees the same order.

| Role | Actions |
| --- | --- |
| Any | Create order → Demo cancel unpaid invoice → Create hold invoice |
| Seller | Lock → Try cancel (skipped after Received) → Release |
| Buyer | Accept (starts a 3 minute fiat timer) → Fiat sent |
| Seller or Buyer | Open dispute, from `WaitingFiat`, `FiatSent`, or `Leg2Failed` → Post chat line |
| Seller or Buyer, after path B | Retry with new invoice |
| Solver | Award buyer / Award seller |

**Path A.** Buyer node stays up. Seller presses Release from `FiatSent`. The log lists the buyer payment, then settle, then `Settled`. The invoice shows `Paid`. Twine’s channel toward the buyer dips for the outbound payment and recovers on the seller channel when the hold settles.

**Path B.** After Fiat sent, disconnect the buyer on the Fiber graph so Twine cannot route, then Release. State becomes `Leg2Failed`. The hold stays `Received`. The log has no `settle_invoice`. Reconnect the buyer and press Retry with new invoice. That retry is path A.

```bash
BUYER_PUB=$(curl -s http://127.0.0.1:8247 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"node_info","params":[]}' \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['pubkey'])")

curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"disconnect_peer\",\"params\":[{\"pubkey\":\"$BUYER_PUB\"}]}"

# after Leg2Failed, bring the buyer back, then Retry in the app
curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"connect_peer\",\"params\":[{\"pubkey\":\"$BUYER_PUB\",\"address\":\"/ip4/127.0.0.1/tcp/8248\",\"save\":true}]}"
```

`TWINE_RELEASE_PAUSE_MS` (milliseconds) pauses the daemon after the buyer invoice is created and before `send_payment`, if you would rather stop the buyer process than disconnect the peer.

**Path C.** Open dispute while the hold is still `Received`. Both sides post a line. Switch to Solver. Award buyer runs path A (pay, then settle). If that route fails, the order stays `Disputed` and is not settled. Award seller does not settle and does not cancel; the log says the seller is refunded when the TLC expires. After the invoice is `Expired`, both awards fail.

A seller-wins order stays open, so create is blocked until that hold expires. Run a buyer-wins trade first if you want both endings without waiting.

**Path D.** Lock a hold and do not settle it. Leave the daemon running. After 16 hours `get_invoice` is `Expired`, the order state is `Expired`, the seller’s channel balance is restored, and the log records a failed `settle_invoice` for that payment hash. It does not record a successful settle or a cancel.

## HTTP API

The daemon is the only process that talks to `fnn`. Bodies are JSON. Empty actions take `{}`.

| Method | Path | Body |
| --- | --- | --- |
| `GET` | `/health` | three pubkeys and both channel balances |
| `GET` | `/order` | current order. Never includes `S` |
| `POST` | `/order` | `{"amount":"1"}` |
| `POST` | `/order/demo_cancel` | throwaway unpaid invoice, then `cancel_invoice` |
| `POST` | `/order/hold` | hold invoice on the Twine node |
| `POST` | `/order/lock` | seller `send_payment` |
| `POST` | `/order/try_cancel` | refuses to cancel a `Received` hold |
| `POST` | `/order/accept` | |
| `POST` | `/order/fiat_sent` | |
| `POST` | `/order/release` | path A, or `Leg2Failed` if the buyer payment fails |
| `POST` | `/order/retry` | path A again, from `Leg2Failed` |
| `POST` | `/order/dispute` | |
| `POST` | `/order/chat` | `{"from":"buyer","text":"…"}` or `"from":"seller"` |
| `POST` | `/order/award_buyer` | |
| `POST` | `/order/award_seller` | |

Errors are `{"error":"…"}` with 400 (bad amount or state), 409 (an order is already open), or 502 (Fiber).

Confirm a hold directly (replace `H`):

```bash
curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"get_invoice","params":[{"payment_hash":"H"}]}'
```

## Configuration

| Variable | Default | |
| --- | --- | --- |
| `LISTEN` | `127.0.0.1:8080` | daemon bind address |
| `SELLER_RPC` | `http://127.0.0.1:8227` | seller `fnn` |
| `TWINE_RPC` | `http://127.0.0.1:8237` | Twine `fnn` |
| `BUYER_RPC` | `http://127.0.0.1:8247` | buyer `fnn` |
| `ORDER_FILE` | `order.json` in the working directory | persisted order, including `S` |
| `TWINE_RELEASE_PAUSE_MS` | unset | pause before the buyer `send_payment` |
| `FUNDING_SHANNONS` | `50000000000` (500 CKB) | channel funding in `open-channels.sh` |

P2P ports are seller `8228`, Twine `8238`, buyer `8248`.

## Out of scope

Nostr, encrypted DMs, key rotation, bonds, fees, UDT, accounts, a public order book, and app-store builds. Dispute chat is plain text stored on the daemon.
