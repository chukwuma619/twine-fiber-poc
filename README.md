# Twine Fiber POC

A testnet proof of concept for a Fiber hold-invoice P2P market. One Flutter app is the client. One Rust daemon is the coordinator. Fiber nodes (any user, plus Twine) move real testnet CKB.

A person is one Fiber node. The Fiber pubkey is the user id — anyone can list or take. The app never holds a Fiber key. Fiat is a button. The CKB movement is `new_invoice`, `send_payment`, `settle_invoice`, and TLC expiry on testnet (`Fibt`).

```text
twine-fiber-poc/
  daemon/     Rust. HTTP API for ads and trades. JSON-RPC only to the Twine fnn. Holds preimage S.
  app/        Flutter. Market, post-ad, trade, and settings. Each phone talks to its own fnn.
  scripts/    Download fnn, start the lab nodes, open channels.
```

Node data, keys, the `fnn` binary, `daemon/order.json`, and `daemon/market.json` stay out of git.

## What a reviewer can do

1. Open Settings, point the app at a Fiber RPC, and open both channel directions with Twine.
2. Post a sell ad: CKB amount, currency, price, min–max per take, payment rail and handle.
3. From a second node, pick that ad, enter how much they pay inside the limit, and start a trade. The daemon creates a hold invoice for that slice only.
4. Seller locks the hold from their own node. Buyer marks fiat sent and supplies a payout invoice. Seller releases.
5. Watch one of the four endings on testnet.

```text
[ Seller node ]              [ Twine daemon ]                    [ Buyer node ]

1. Create hold invoice (H)          daemon calls twine fnn new_invoice
2. Lock HTLC, status Received       seller app send_payment on the seller fnn
3. Fiat window                      starts when the hold is Received
4. Fiat sent                        buyer app new_invoice, then a button
5. Seller releases                  daemon pays the buyer invoice, then settle
```

| Path | What they do | What Fiber does |
| --- | --- | --- |
| A | Buyer is reachable. Seller releases | Daemon pays the buyer’s invoice, then `settle_invoice(H, S)`. Invoice `Paid`. State `Settled` |
| B | Payment to the buyer cannot route | `send_payment` fails. No `settle_invoice`. Hold stays `Received`. State `Leg2Failed`. A new invoice retries path A |
| C | Either side disputes. Operator reads the chat | Buyer wins runs path A. If that payment fails, the order stays `Disputed` and is not settled. Seller wins leaves the hold `Received` until expiry |
| D | Nobody settles before the timelock | Fiber marks the invoice `Expired` and the seller is refunded. A later `settle_invoice` fails. `cancel_invoice` is not called |

One open trade per ad. The listing is not locked. Creating a trade reserves that slice (`ckb = pay / price`) and hides the ad until the trade ends. The reserved CKB returns if the trade is `Cancelled` or `Expired`. A `Settled` trade keeps the slice subtracted; leftover stays on the book if it still covers the minimum take.

## How the hold works

The daemon generates preimage `S`, sets `H = SHA256(S)`, and creates the hold invoice with `payment_hash` and `hash_algorithm: sha256`. `payment_preimage` is omitted. `S` stays in `market.json` and is never returned by `GET /trades`.

Currency is `Fibt`. Trade amounts are CKB. One CKB is `0x5f5e100` shannon. Use a small amount (1 CKB is enough).

The seller app calls `send_payment` on the seller node. The buyer app calls `new_invoice` on the buyer node. The daemon talks only to Twine.

| Moment | RPC | Invoice status |
| --- | --- | --- |
| Create hold | Twine `new_invoice` | `Open` |
| Lock | Seller `send_payment` (from the app) | `Received`. Coins are held in the seller’s channel |
| Unpaid cancel | `cancel_invoice` | `Cancelled`. Only legal while `Open` |
| Path A | Twine `send_payment` to the stored buyer invoice, then `settle_invoice(H, S)` | `Paid` |
| Path B | Buyer payment fails | stays `Received`. No `settle_invoice` |
| Path C, seller wins | No `settle_invoice` | stays `Received` until expiry |
| Path D | Nobody settles. `final_expiry_delta` passes | `Expired`. Seller payment returns |

`cancel_invoice` on a `Received` invoice is refused. Seller-wins and “buyer never returns” share path D’s refund: the TLC expires. The shortest `final_expiry_delta` Fiber accepts is 16 hours (`57_600_000` ms, `0x36ee800`). Every hold uses that. There is no virtual clock. The daemon polls `get_invoice` every 15 seconds; the app polls `GET /trades/:id` every 5 seconds while a hold can still expire.

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

That downloads `fnn` v0.9.1, writes a random node password to `nodes/password`, and prints a testnet address for seller, Twine, and buyer. Those three lab nodes are still the easiest two-user demo: treat “seller” and “buyer” as two people. Send CKB from [https://faucet.nervos.org](https://faucet.nervos.org) to all three before opening channels:

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

The Settings screen can do the same job without the script: **Open channel to Twine** funds that user’s outbound channel (needed to sell). **Ask Twine to open a return channel** calls `POST /connect` so Twine can pay them (needed to buy). Both directions are required.

```bash
curl -s http://127.0.0.1:8080/health
```

`ready` is true when the Twine node answers. Defaults are `TWINE_RPC=http://127.0.0.1:8237` and `TWINE_P2P=/ip4/127.0.0.1/tcp/8238`. Start the daemon from `daemon/` so the market file is `daemon/market.json`.

```bash
cd app && flutter run
```

On the iOS simulator the daemon URL is `http://127.0.0.1:8080`. On the Android emulator use `http://10.0.2.2:8080`. The app fills that in. A phone on the same network needs `LISTEN=0.0.0.0:8080` and the machine’s LAN address.

Point one app at the seller RPC (`http://127.0.0.1:8227`, P2P `/ip4/127.0.0.1/tcp/8228`) and a second app at the buyer RPC (`http://127.0.0.1:8247`, P2P `/ip4/127.0.0.1/tcp/8248`).

```bash
./scripts/stop-nodes.sh
```

That stops the daemon (if `nodes/daemon.pid` exists) and the three nodes.

## Walk it in the app

Settings stores the display name, this user’s `fnn` RPC, P2P address, and the Twine daemon URL. **Read node info** treats the Fiber pubkey as the user id. The same node can post an ad or take someone else’s.

| Screen | Actions |
| --- | --- |
| Market | Browse open ads. Tap an offer, pick how much to pay in the limit, start a trade |
| Post ad | Available, currency, price, min–max, payment rail and handle |
| Trade | Seller: Lock, Release. Buyer: Fiat sent, Retry. Either side: dispute and chat |
| Settings | Channel status. Open channel to Twine. Ask Twine for a return channel. Operator tools toggle |

**Path A.** Buyer node stays up. Seller posts 2 CKB at 2000 NGN, limit 2000–4000 NGN. Buyer takes 2000 NGN (hold is 1 CKB). Seller presses Lock, buyer pays fiat outside and presses Fiat sent, seller presses Release. Leftover 1 CKB stays listed if it still meets the minimum. The log lists the buyer payment, then settle, then `Settled`. The invoice shows `Paid`.

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

`TWINE_RELEASE_PAUSE_MS` (milliseconds) pauses the daemon after it has the buyer invoice and before `send_payment`, if you would rather stop the buyer process than disconnect the peer.

**Path C.** Open dispute while the hold is still `Received`. Both sides post a line. Turn on Operator tools in Settings. Award buyer runs path A (pay, then settle). If that route fails, the order stays `Disputed` and is not settled. Award seller does not settle and does not cancel; the log says the seller is refunded when the TLC expires. After the invoice is `Expired`, both awards fail.

A seller-wins trade stays open on that ad, so a second take is blocked until the hold expires.

**Path D.** Lock a hold and do not settle it. Leave the daemon running. After 16 hours `get_invoice` is `Expired`, the trade state is `Expired`, the reserved CKB returns to the ad, and the log records a failed `settle_invoice` for that payment hash.

## HTTP API

The daemon is the only process that talks to the Twine `fnn`. User nodes are called from the app. Bodies are JSON.

| Method | Path | Body |
| --- | --- | --- |
| `GET` | `/health` | Twine pubkey |
| `GET` | `/twine` | Twine pubkey and P2P address |
| `POST` | `/connect` | `{"pubkey":"…","address":"/ip4/…"}`. Twine `connect_peer` + `open_channel` |
| `GET` | `/ads` | takeable ads (no open trade, leftover still covers min) |
| `POST` | `/ads` | `{"pubkey","available","currency","price","min","max","payment_method"}` |
| `POST` | `/ads/:id/cancel` | |
| `GET` | `/trades?pubkey=` | trades for that node |
| `POST` | `/trades` | `{"ad_id","taker","pay_amount"}`. Creates the hold |
| `GET` | `/trades/:id` | current trade. Never includes `S` |
| `POST` | `/trades/:id/demo_cancel` | throwaway unpaid invoice, then `cancel_invoice` |
| `POST` | `/trades/:id/locked` | poll Twine until the hold is `Received`, then start the fiat window |
| `POST` | `/trades/:id/try_cancel` | refuses to cancel a `Received` hold |
| `POST` | `/trades/:id/fiat_sent` | `{"invoice":"…"}` buyer invoice from the buyer node |
| `POST` | `/trades/:id/release` | path A, or `Leg2Failed` if the buyer payment fails |
| `POST` | `/trades/:id/retry` | `{"invoice":"…"}` path A again, from `Leg2Failed` |
| `POST` | `/trades/:id/dispute` | |
| `POST` | `/trades/:id/chat` | `{"from":"lister","text":"…"}` or `"from":"taker"` |
| `POST` | `/trades/:id/award_buyer` | `{"invoice":"…"}` |
| `POST` | `/trades/:id/award_seller` | |

Errors are `{"error":"…"}` with 400 (bad amount or state), 404 (unknown ad or trade), 409 (an ad already has an open trade), or 502 (Fiber).

Confirm a hold directly (replace `H`):

```bash
curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"get_invoice","params":[{"payment_hash":"H"}]}'
```

## Configuration

| Variable | Default | |
| --- | --- | --- |
| `LISTEN` | `127.0.0.1:8080` | daemon bind address |
| `TWINE_RPC` | `http://127.0.0.1:8237` | Twine `fnn` |
| `TWINE_P2P` | `/ip4/127.0.0.1/tcp/8238` | Twine P2P address returned by `GET /twine` |
| `MARKET_FILE` | `market.json` in the working directory | persisted ads and trades, including `S` |
| `ORDER_FILE` | unused except to pick the directory for `market.json` | |
| `TWINE_RELEASE_PAUSE_MS` | unset | pause before the buyer `send_payment` |
| `FUNDING_SHANNONS` | `50000000000` (500 CKB) | channel funding in `open-channels.sh` and `POST /connect` |

P2P ports are seller `8228`, Twine `8238`, buyer `8248`.

## Out of scope

Nostr, encrypted DMs, key rotation, bonds, fees, UDT, accounts, buy-side ads, rate oracles, partial fills, and app-store builds. Dispute chat is plain text stored on the daemon. Pubkeys on ads and trades are not authenticated.
