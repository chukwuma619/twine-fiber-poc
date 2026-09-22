# Twine

Twine is a testnet proof of concept for a peer-to-peer CKB market settled on [Fiber](https://github.com/nervosnetwork/fiber). A person posts an offer priced in ordinary money (naira, dollars, and the rest). Another person takes a slice of that offer, pays that money outside the app, and receives CKB on Fiber. The CKB is locked in a hold invoice for the whole fiat window, then released, or returned when the hold expires.

One Flutter app is the trader client. One Rust daemon is the coordinator. Fiber nodes move real testnet CKB (`Fibt`). The app never holds a Fiber key.

This repository is the lab that proves that loop on testnet. It is a finished proof of concept, and it is the whole product surface that exists today: market, order room, channels, and a host-side solver for disputes.

## What it shows

- A Fiber pubkey is the user. Anyone who can reach a node can list or take. There is no account, display name, or login.
- Listing and taking are the same person wearing two hats. The lab scripts name two nodes `seller` and `buyer` only so two phones have somewhere to point.
- Fiat never enters the protocol. The buyer uploads a JPEG or PNG receipt. The seller looks at it and releases, or either side files a dispute.
- CKB moves only through Fiber: `new_invoice`, `send_payment`, `settle_invoice`, and TLC expiry. Currency is `Fibt`. Amounts are CKB. One CKB is `100_000_000` shannon (`0x5f5e100`).
- The coordinator keeps the hold preimage. The public trade view never includes it.
- Four endings are real on testnet: release, a failed payout that can be retried, a dispute awarded from the host, and a 16-hour expiry that refunds the lister.

## Architecture

```text
 Phone A (Flutter)                         Phone B (Flutter)
 talks to its own fnn                      talks to its own fnn
        |                                         |
        |  HTTP :8080 (ads, trades, chat)         |
        +------------------+----------------------+
                           |
                    Twine daemon (Rust)
                    market.json + proofs/
                    JSON-RPC only to the Twine fnn
                           |
              +------------+------------+
              |            |            |
         seller fnn    twine fnn    buyer fnn
         :8227/:8228   :8237/:8238  :8247/:8248
              \            |            /
               \____ Fiber testnet ____/
```

| Piece | Role |
| --- | --- |
| `app/` | Flutter client. Market, post an offer, order room, settings. Each install talks to one `fnn` and to the daemon. |
| `daemon/` | Axum HTTP API. Persists ads and trades. The only process that calls the Twine `fnn`. Holds preimage `S`. |
| `scripts/` | Download `fnn` v0.9.1, start the three lab nodes, open channels, stop them. |
| Solver | Whoever can reach the daemon on the host. `curl` is the tool. There is no admin screen and no admin key. |

The daemon talks only to Twine. Locking a hold (`send_payment`) runs on the lister’s node, from the lister’s app. The payout invoice (`new_invoice`) is created on the taker’s node, from the taker’s app. Twine pays that invoice from its own outbound channel, because the lister’s coins stay locked until `settle_invoice`.

Node data, keys, the `fnn` binary, `daemon/market.json`, `daemon/order.json`, and `daemon/proofs/` stay out of git.

## Repository

```text
twine-fiber-poc/
  app/        Flutter. Dart 3.11.
  daemon/     Rust. Axum, Tokio, a JSON-RPC client for fnn.
  scripts/    setup-nodes.sh, start-nodes.sh, open-channels.sh, stop-nodes.sh
  bin/        fnn and fnn-cli after setup (gitignored)
  nodes/      three testnet node directories (gitignored)
```

## Identity and inventory

A person is one Fiber node. Settings loads that node’s pubkey; that string is the user id on every ad and trade. Pubkeys are compared after trimming and lowercasing, with or without a `0x` prefix. They are not authenticated. Anyone who can reach the daemon can post an ad under a pubkey they can type.

An ad is one offer:

| Field | Meaning |
| --- | --- |
| `available` | CKB still for sale, in whole CKB |
| `currency` | Fiat code the price is quoted in (`NGN`, `USD`, …) |
| `price` | Units of that currency per 1 CKB, shown as `2000 NGN/CKB` |
| `min` / `max` | Smallest and largest take, in that currency |
| `payment_method` | Rail and handle, for example `Opay · 0803…` |

Taking an ad reserves `ckb = pay_amount / price` and hides the ad until the trade ends. One open trade per ad. A second take gets `409`. The reserved CKB returns to the book when the trade is `Cancelled` or `Expired`. A `Settled` trade keeps the slice subtracted. Leftover stays listed when it still covers the minimum take.

Supported currency codes live in `app/lib/currencies.dart`: NGN, GHS, KES, UGX, TZS, RWF, ZAR, XOF, XAF, EGP, MAD, USD, EUR, GBP, CAD, AUD, INR, PHP, IDR, BRL, MXN, CNY, JPY. The code is a label. There is no rate oracle. The lister types the price.

## A trade

```text
Lister phone                 Twine daemon                      Taker phone

Post ad                      stored in market.json
                             GET /ads shows it
                                                          Take: POST /trades
                             new_invoice on Twine fnn
                             state WaitingHold
                             accept window = 15 minutes
Accept order
send_payment on lister fnn
POST /locked                 polls until invoice Received
                             state WaitingFiat
                             pay window = 15 minutes
                                                          Pay fiat outside the app
                                                          Upload JPEG/PNG
                                                          new_invoice on taker fnn
                                                          POST /fiat_sent
                             state FiatSent
Release CKB
                             send_payment of the taker invoice
                             then settle_invoice(H, S)
                             state Settled, invoice Paid
```

Two clocks sit on top of Fiber’s own timelock:

| Window | Length | Starts | If it passes |
| --- | --- | --- | --- |
| Accept | 15 minutes | Trade created, invoice `Open` | Daemon cancels the unpaid invoice. State `Cancelled`. Reserved CKB returns. |
| Pay | 15 minutes | Hold becomes `Received` | State `PayWindowClosed`. The hold stays `Received` until the 16-hour Fiber expiry. |
| Fiber TLC | 16 hours | `new_invoice` (`final_expiry_delta`) | Invoice `Expired`. Lister is refunded. State `Expired`. |

The daemon polls `get_invoice` every 15 seconds. The app polls `GET /trades/:id` every 5 seconds while a hold can still expire. There is no virtual clock.

### Order states

| State | Meaning |
| --- | --- |
| `WaitingHold` | Hold invoice is `Open`. Lister can accept. Taker can cancel. |
| `WaitingFiat` | Hold is `Received`. Taker has 15 minutes to pay and upload a receipt. |
| `PayWindowClosed` | Those 15 minutes passed. Hold stays locked. Either side can dispute. |
| `FiatSent` | Receipt and taker payout invoice are stored. Lister can release. |
| `Releasing` | Daemon is paying the taker, then settling. |
| `Leg2Failed` | Taker invoice could not be paid. Hold stays `Received`. Retry with a new invoice. |
| `Disputed` | Someone asked Twine to step in. Funds stay locked until the solver awards. |
| `Settled` | Path A completed. Invoice `Paid`. Slice stays off the book. |
| `Cancelled` | Unpaid hold was cancelled, or the accept window elapsed. Slice returns. |
| `Expired` | Fiber marked the invoice `Expired`. Slice returns. |

A new trade is briefly `Pending` while the hold invoice is created, then `WaitingHold`. `Idle`, `Held`, and `Paid` remain on the enum so older rows still load. Live trades use the states in the table.

Chat is allowed from `WaitingHold` through `Disputed`, including `Leg2Failed`. A dispute can be filed from `WaitingFiat`, `PayWindowClosed`, `FiatSent`, or `Leg2Failed`. Chat is plain text stored on the daemon.

## Four endings

| Path | What people do | What Fiber does |
| --- | --- | --- |
| A | Taker is reachable. Lister releases, or the solver awards the taker | Daemon pays the taker’s invoice, then `settle_invoice(H, S)`. Invoice `Paid`. State `Settled` |
| B | Payment to the taker cannot route | `send_payment` fails. No `settle_invoice`. Hold stays `Received`. State `Leg2Failed`. A new invoice retries path A |
| C | Either side files a dispute with a reason. The solver reads chat and the receipt, then calls the daemon | Taker wins runs path A. If that payment fails, the order stays `Disputed` and is not settled. Lister wins leaves the hold `Received` until expiry |
| D | Nobody settles before the timelock | Fiber marks the invoice `Expired` and the lister is refunded. A later `settle_invoice` fails. `cancel_invoice` is not called |

Leg 2 comes first. The daemon calls `settle_invoice` only after `send_payment` to the taker reports `Success`.

A lister-wins award leaves that trade open on the ad, so a second take is blocked until the hold expires.

## How the hold works

The daemon generates preimage `S`, sets `H = SHA256(S)`, and creates the hold invoice with `payment_hash` and `hash_algorithm: sha256`. `payment_preimage` is omitted from the RPC call. `S` stays in `market.json` and is never returned by `GET /trades`.

| Moment | RPC | Invoice status |
| --- | --- | --- |
| Create hold | Twine `new_invoice` | `Open` |
| Lock | Lister `send_payment` (from the app) | `Received`. Coins sit in the lister’s channel |
| Unpaid cancel | `cancel_invoice` | `Cancelled`. Legal only while `Open` |
| Path A | Twine `send_payment` to the stored taker invoice, then `settle_invoice(H, S)` | `Paid` |
| Path B | Taker payment fails | stays `Received`. No `settle_invoice` |
| Path C, lister wins | No `settle_invoice` | stays `Received` until expiry |
| Path D | Nobody settles. `final_expiry_delta` passes | `Expired`. Lister payment returns |

Fiber refuses `cancel_invoice` on a `Received` invoice. Lister-wins and “taker never comes back” share path D’s refund: the TLC expires. The shortest `final_expiry_delta` this `fnn` v0.9.1 accepts is 16 hours (`57_600_000` ms, `0x36ee800`). Every hold uses that.

Confirm a hold directly (replace `H`):

```bash
curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"get_invoice","params":[{"payment_hash":"H"}]}'
```

## Requirements

- macOS or Linux
- Two phones to run the app: two iOS Simulators (Xcode), two Android emulators (Android Studio), or one of each
- `curl`, `jq`, `python3`, `openssl`
- [ckb-cli](https://github.com/nervosnetwork/ckb-cli) on `PATH` (used once, to print testnet addresses)
- Rust (stable) and Cargo
- Flutter. The app targets Dart 3.11 (`sdk: ^3.11.4`)

`./scripts/setup-nodes.sh` downloads a Fiber v0.9.1 portable bundle for the host (macOS or Linux, arm64 or x86_64) and checks its SHA-256.

## Run the lab

```bash
./scripts/setup-nodes.sh
```

That writes a random node password to `nodes/password` and prints a testnet address for the seller node, the Twine node, and the buyer node. Send CKB from [https://faucet.nervos.org](https://faucet.nervos.org) to all three before opening channels:

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

The Settings screen can open the same channels without the script. **Open channel to Twine** funds that user’s outbound channel (needed to sell). **Ask Twine to open a return channel** calls `POST /connect` so Twine can pay them (needed to buy). Both directions are required for a full trade.

```bash
curl -s http://127.0.0.1:8080/health
```

`ready` is true when the Twine node answers. Defaults are `TWINE_RPC=http://127.0.0.1:8237` and `TWINE_P2P=/ip4/127.0.0.1/tcp/8238`. Start the daemon from `daemon/` so the market file is `daemon/market.json`. Then launch two phones.

When you are done:

```bash
./scripts/stop-nodes.sh
```

That stops the daemon (if `nodes/daemon.pid` exists) and the three nodes. Quit each `flutter run` with `q`.

## Two phones

One phone is one user: that install talks to one `fnn`, and the Fiber pubkey is their id. The same user can list or take. A trade still needs two phones, because lock/`send_payment` runs on the lister’s node and the payout invoice comes from the taker’s node.

Two iOS Simulators, two Android emulators, or one of each all work. Leave both `flutter run` terminals open.

Plain `flutter run` (no dart-defines) assigns the two default iOS simulators: **iPhone 17 Pro** is User A (`8227`), **iPhone 18 Pro** is User B (`8247`). The title bar shows `Twine · User A` or `Twine · User B`. Each install then reads that node’s pubkey. Dart-defines still override this. Any other device pair is assigned from the device id, or you set Fiber RPC and P2P in Settings and tap **Read this phone**.

The Android emulator reaches the host as `10.0.2.2`, and the app rewrites `127.0.0.1` in the Fiber RPC when it detects Android. P2P stays `127.0.0.1` because Twine (on the host) dials the user `fnn` on the host.

| Phone | Lab node | Fiber RPC (iOS / desktop) | Fiber RPC (Android emulator) | P2P |
| --- | --- | --- | --- | --- |
| User A | `seller` | `http://127.0.0.1:8227` | `http://10.0.2.2:8227` | `/ip4/127.0.0.1/tcp/8228` |
| Twine | `twine` | `http://127.0.0.1:8237` | — | `/ip4/127.0.0.1/tcp/8238` |
| User B | `buyer` | `http://127.0.0.1:8247` | `http://10.0.2.2:8247` | `/ip4/127.0.0.1/tcp/8248` |

Daemon URL is `http://127.0.0.1:8080` on iOS and desktop, and `http://10.0.2.2:8080` on an Android emulator. A physical phone on the same network needs `LISTEN=0.0.0.0:8080` and the machine’s LAN address for RPC, P2P, and the daemon.

Those script names are two machines, not roles. Either side can post or take.

1. Boot two devices and confirm both show up:

```bash
# iOS: Xcode → Window → Devices and Simulators, or File → Open Simulator
# Android: Android Studio Device Manager, or:
flutter emulators
flutter emulators --launch EMULATOR_A
flutter emulators --launch EMULATOR_B

flutter devices
```

2. Fetch the two user pubkeys (optional; **Read this phone** does the same thing):

```bash
A_PUB=$(curl -s http://127.0.0.1:8227 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"node_info","params":[]}' \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['pubkey'])")
B_PUB=$(curl -s http://127.0.0.1:8247 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"node_info","params":[]}' \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['pubkey'])")
echo "A=$A_PUB"
echo "B=$B_PUB"
```

3. Start User A (replace `DEVICE_A`). On Android, use the `10.0.2.2` RPC:

```bash
cd app
flutter run -d DEVICE_A \
  --dart-define=TWINE_FIBER_RPC=http://127.0.0.1:8227 \
  --dart-define=TWINE_P2P=/ip4/127.0.0.1/tcp/8228 \
  --dart-define=TWINE_PUBKEY="$A_PUB"
```

4. In a second terminal, start User B:

```bash
cd app
flutter run -d DEVICE_B \
  --dart-define=TWINE_FIBER_RPC=http://127.0.0.1:8247 \
  --dart-define=TWINE_P2P=/ip4/127.0.0.1/tcp/8248 \
  --dart-define=TWINE_PUBKEY="$B_PUB"
```

On an Android emulator, set `TWINE_FIBER_RPC` to `http://10.0.2.2:8227` (User A) or `http://10.0.2.2:8247` (User B). The daemon URL is already `http://10.0.2.2:8080` unless you override `TWINE_DAEMON_URL`.

## Walk it in the app

Settings stores this phone’s `fnn` RPC, P2P address, and the Twine daemon URL. **Read this phone** loads the Fiber pubkey.

| Screen | What you do |
| --- | --- |
| Market | **BUY CKB** is other people’s ads. **SELL CKB** is yours (badge **YOUR OFFER**). Tap **+** to post. Tap someone else’s offer to take. A lister with a new take sees **New order — accept** even while the ad is hidden. The home shell polls every 5 seconds. |
| Post ad | Available (CKB), currency, price (that currency per CKB), min–max per take, payment rail and handle |
| Order | Accept / Pay / Release. Lister: Accept order (Fiber lock), Reject, Release CKB. Taker: pay card, upload receipt, I have paid, Retry. Either side: cancel while the hold is `Open`, chat, File dispute |
| Settings | Channel status. Open channel to Twine. Ask Twine for a return channel |

### Path A

1. On User A, open **SELL CKB** → **+**. Post an offer, for example available `2` CKB, currency `NGN`, price `2000` (2000 NGN/CKB), min `2000`, max `4000`, payment `Opay`.
2. On User A, BUY CKB stays empty for that ad. On **SELL CKB** it shows **YOUR OFFER**.
3. On User B, **BUY CKB** shows the same card. Tap it, pay an amount inside the limit (for example `2000` NGN → 1 CKB hold), **Start trade**. B lands on the order room: waiting for the lister, with **Cancel order** while the hold is still `Open`.
4. On User A, the book shows **New order — accept**. **Accept order** runs `send_payment` on A’s `fnn`, then `POST /locked`. The taker then has 15 minutes to pay.
5. On User B, pay fiat outside the app using the copyable handle, **Upload receipt**, then **I have paid** (B’s `fnn` creates the payout invoice). The receipt is a JPEG or PNG, at most 1.5 MB, stored on the daemon.
6. On User A, open the order, check the screenshot, then **Release CKB**. The daemon pays B, then `settle_invoice`. State `Settled`, invoice `Paid`. Leftover 1 CKB stays listed when it still meets the minimum.

Use a small amount. 1 CKB is enough.

### Path B

After the taker notifies the lister, disconnect the taker on the Fiber graph so Twine cannot route, then **Release CKB**. State becomes `Leg2Failed`. The hold stays `Received`. The log has no `settle_invoice`. Reconnect the taker and press **Retry** with a new invoice. That retry is path A.

```bash
BUYER_PUB=$(curl -s http://127.0.0.1:8247 -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"node_info","params":[]}' \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['pubkey'])")

curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"disconnect_peer\",\"params\":[{\"pubkey\":\"$BUYER_PUB\"}]}"

# after Leg2Failed, bring the taker back, then Retry in the app
curl -s http://127.0.0.1:8237 -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"connect_peer\",\"params\":[{\"pubkey\":\"$BUYER_PUB\",\"address\":\"/ip4/127.0.0.1/tcp/8248\",\"save\":true}]}"
```

`TWINE_RELEASE_PAUSE_MS` (milliseconds) pauses the daemon after it has the taker invoice and before `send_payment`, if you would rather stop the taker process than disconnect the peer.

### Path C

Either side can chat while the hold is still `Received`. **File dispute** with a reason is the only request for Twine to step in. The app does not award the trade. Award from the host; see [Solver](#solver).

### Path D

Lock a hold and leave it. Leave the daemon running. After 16 hours `get_invoice` is `Expired`, the trade state is `Expired`, the reserved CKB returns to the ad, and the log records the expiry for that payment hash.

## Solver

Traders use the Flutter app. The operator awards from another tool. This POC has no admin app and no admin key. The tool is HTTP against the daemon on the host. Anyone who can reach `:8080` can award. That is acceptable on localhost. It is not acceptable if the daemon is reachable from the network.

1. In the app, either side files a dispute. State becomes `Disputed`. Funds stay locked (`Received`).
2. On the host, read the trade: chat, `dispute_reason`, and the receipt.
3. Award the taker or award the lister.

```bash
TRADE_ID=t1   # from the order screen, or GET /trades?pubkey=

curl -s "http://127.0.0.1:8080/trades/$TRADE_ID"
curl -s "http://127.0.0.1:8080/trades/$TRADE_ID/proof" -o /tmp/receipt.bin
```

**Taker wins** pays the stored taker invoice, then `settle_invoice` (path A). If that payment fails, the order stays `Disputed`.

```bash
INVOICE=$(curl -s "http://127.0.0.1:8080/trades/$TRADE_ID" \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['buyer_invoice'])")

curl -s -X POST "http://127.0.0.1:8080/trades/$TRADE_ID/award_buyer" \
  -H 'content-type: application/json' \
  -d "{\"invoice\":\"$INVOICE\"}"
```

**Lister wins** does not settle and does not cancel. The log says the lister is refunded when the TLC expires (path D).

```bash
curl -s -X POST "http://127.0.0.1:8080/trades/$TRADE_ID/award_seller"
```

After the hold is `Expired`, both awards fail.

## HTTP API

The daemon is the only process that talks to the Twine `fnn`. User nodes are called from the app. Bodies are JSON. CORS allows origins whose host is `localhost` or `127.0.0.1`.

| Method | Path | Body |
| --- | --- | --- |
| `GET` | `/health` | Twine pubkey. `ready` when that node answers |
| `GET` | `/twine` | Twine pubkey and P2P address |
| `POST` | `/connect` | `{"pubkey":"…","address":"/ip4/…"}`. Twine `connect_peer` + `open_channel` |
| `GET` | `/ads` | Takeable ads (no open trade, leftover still covers min) |
| `POST` | `/ads` | `{"pubkey","available","currency","price","min","max","payment_method"}` |
| `POST` | `/ads/:id/cancel` | Cancel a listing |
| `GET` | `/trades?pubkey=` | Trades for that node |
| `POST` | `/trades` | `{"ad_id","taker","pay_amount"}`. Creates the hold |
| `GET` | `/trades/:id` | Current trade. Never includes `S`. Proof is `{content_type, bytes}` only. Includes `accept_by` and `pay_by` |
| `GET` | `/trades/:id/proof` | JPEG or PNG bytes |
| `POST` | `/trades/:id/demo_cancel` | Creates a throwaway unpaid invoice and `cancel_invoice`s it. The live trade state is unchanged |
| `POST` | `/trades/:id/locked` | Poll Twine until the hold is `Received`, then start the 15-minute pay window |
| `POST` | `/trades/:id/try_cancel` | Refuses to cancel a `Received` hold |
| `POST` | `/trades/:id/cancel` | `{"from":"lister"\|"taker"}`. `WaitingHold` + `Open` runs `cancel_invoice`. Taker in `WaitingFiat` closes the pay window; the hold stays `Received` |
| `POST` | `/trades/:id/fiat_sent` | `{"invoice","proof_b64","content_type"}` JPEG/PNG, 1.5 MB max |
| `POST` | `/trades/:id/release` | Path A, or `Leg2Failed` if the taker payment fails |
| `POST` | `/trades/:id/retry` | `{"invoice":"…"}` path A again, from `Leg2Failed` |
| `POST` | `/trades/:id/dispute` | `{"from":"lister"\|"taker","reason":"…"}` |
| `POST` | `/trades/:id/chat` | `{"from":"lister","text":"…"}` or `"from":"taker"` on an open trade |
| `POST` | `/trades/:id/award_buyer` | Solver. `{"invoice":"…"}`. Path A. A failed pay leaves `Disputed` |
| `POST` | `/trades/:id/award_seller` | Solver. Hold stays `Received` until expiry |

`from` accepts `lister` or `seller`, and `taker` or `buyer`. Stored values are `lister` and `taker`.

Errors are `{"error":"…"}` with 400 (bad amount or state), 404 (unknown ad or trade), 409 (an ad already has an open trade), or 502 (Fiber).

## Configuration

Daemon (process environment):

| Variable | Default | |
| --- | --- | --- |
| `LISTEN` | `127.0.0.1:8080` | Bind address |
| `TWINE_RPC` | `http://127.0.0.1:8237` | Twine `fnn` |
| `TWINE_P2P` | `/ip4/127.0.0.1/tcp/8238` | Twine P2P address returned by `GET /twine` |
| `MARKET_FILE` | `market.json` in the working directory | Persisted ads and trades, including `S` |
| `ORDER_FILE` | unused except to pick the directory for `market.json` | |
| `TWINE_RELEASE_PAUSE_MS` | unset | Pause before the taker `send_payment` |
| `FUNDING_SHANNONS` | `50000000000` (500 CKB) | Channel funding in `open-channels.sh` and `POST /connect` |

App launch (`--dart-define`, compile-time; used when SharedPreferences has no value yet):

| Define | Default | |
| --- | --- | --- |
| `TWINE_FIBER_RPC` | `http://127.0.0.1:8227` | This phone’s `fnn` |
| `TWINE_P2P` | `/ip4/127.0.0.1/tcp/8228` | This phone’s P2P address |
| `TWINE_PUBKEY` | unset | Fiber pubkey (user id). Empty until **Read this phone** if omitted |
| `TWINE_DAEMON_URL` | `http://127.0.0.1:8080` on iOS and desktop | Twine daemon. Android emulator default is `http://10.0.2.2:8080` |

## Tests

```bash
cd daemon && cargo test
cd app && flutter test
```

The daemon tests drive the order state machine, including lock, fiat, path B, dispute, windows, and expiry, against a stand-in for Fiber. The Flutter tests cover amounts, the market screen, and lab-user assignment.

## Out of scope

Nostr, encrypted DMs, key rotation, bonds, fees, UDT, accounts, buy-side ads, rate oracles, partial fills, and app-store builds. Chat is plain text on the daemon. Payment screenshots live in `daemon/proofs/`. The award routes are unauthenticated. Fiber’s shortest hold is 16 hours, so path D is a real wait, not a shortened lab clock.
