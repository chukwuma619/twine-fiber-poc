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

You need **two phones** (two users): two iOS Simulators, two Android emulators, or one of each. One `flutter run` is not enough.

1. Fund and start the three lab nodes, open channels, start the daemon (see [Run it](#run-it)).
2. Launch the app on two devices, each pointed at a different Fiber RPC (see [Two phones](#two-phones-required-for-a-real-trade)).
3. User A posts a sell ad. User B takes it from BUY CKB.
4. A locks the hold. B marks fiat sent. A releases.
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

- macOS or Linux
- Two phones to run the app: two iOS Simulators (Xcode), two Android emulators (Android Studio), or one of each
- `curl`, `jq`, `python3`, `openssl`
- [ckb-cli](https://github.com/nervosnetwork/ckb-cli) on `PATH` (used once, to print testnet addresses)
- Rust (stable) and Cargo
- Flutter (the app targets Dart 3.11). The setup script downloads a Fiber v0.9.1 portable bundle for the host.

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

`ready` is true when the Twine node answers. Defaults are `TWINE_RPC=http://127.0.0.1:8237` and `TWINE_P2P=/ip4/127.0.0.1/tcp/8238`. Start the daemon from `daemon/` so the market file is `daemon/market.json`. Then launch **two** phones.

## Two phones (required for a real trade)

One phone is one user: that install talks to one `fnn`, and the Fiber pubkey is their id. The same user can list or take. A trade still needs **two phones**, because Lock/`send_payment` runs on the lister’s node and the payout invoice comes from the taker’s node.

Two iOS Simulators, two Android emulators, or one of each all work. Leave both `flutter run` terminals open. If one quits, that device no longer has a running app (you will not find Twine on the home screen until you launch it again).

The Android emulator reaches the host as `10.0.2.2`, not `127.0.0.1`. P2P stays `127.0.0.1` because Twine (on the host) dials the user `fnn` on the host.

| Phone | Lab node | Fiber RPC (iOS) | Fiber RPC (Android emulator) | P2P |
| --- | --- | --- | --- | --- |
| User A | `seller` in the scripts | `http://127.0.0.1:8227` | `http://10.0.2.2:8227` | `/ip4/127.0.0.1/tcp/8228` |
| User B | `buyer` in the scripts | `http://127.0.0.1:8247` | `http://10.0.2.2:8247` | `/ip4/127.0.0.1/tcp/8248` |

Daemon URL is `http://127.0.0.1:8080` on iOS / desktop, and `http://10.0.2.2:8080` on an Android emulator (the app fills that in). A physical phone on the same network needs `LISTEN=0.0.0.0:8080` and the machine’s LAN address for RPC, P2P, and the daemon.

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

Copy each device id (`iPhone 17 Pro` and `iPhone 18 Pro`, or two Android emulators).

2. Fetch the two user pubkeys (optional but avoids a Settings tap):

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

3. In one terminal, start User A (replace `DEVICE_A` with the first device id). On Android, use the `10.0.2.2` RPC:

```bash
cd app
flutter run -d DEVICE_A \
  --dart-define=TWINE_FIBER_RPC=http://127.0.0.1:8227 \
  --dart-define=TWINE_P2P=/ip4/127.0.0.1/tcp/8228 \
  --dart-define=TWINE_PUBKEY="$A_PUB"
```

4. In a **second** terminal, start User B. Same idea, port `8247`:

```bash
cd app
flutter run -d DEVICE_B \
  --dart-define=TWINE_FIBER_RPC=http://127.0.0.1:8247 \
  --dart-define=TWINE_P2P=/ip4/127.0.0.1/tcp/8248 \
  --dart-define=TWINE_PUBKEY="$B_PUB"
```

On an Android emulator, set `TWINE_FIBER_RPC` to `http://10.0.2.2:8227` (User A) or `http://10.0.2.2:8247` (User B). The daemon URL is already `http://10.0.2.2:8080` unless you override `TWINE_DAEMON_URL`.

Without dart-defines, open **Settings** on each phone, set that user’s RPC and P2P from the table, then **Read node info**.

## Walk it in the app

Settings stores this user’s `fnn` RPC, P2P address, and the Twine daemon URL. **Read node info** treats the Fiber pubkey as the user id.

| Screen | Actions |
| --- | --- |
| Market | BUY CKB is other people’s ads. SELL CKB is yours (badge **YOUR OFFER**). Tap + to post. Tap someone else’s offer to take |
| Post ad | Available (CKB), currency, price (that currency per CKB, shown as `50 NGN/CKB`), min–max per take, payment rail and handle |
| Trade | Lister: Lock, Release. Taker: Fiat sent, Retry. Either side: dispute and chat |
| Settings | Channel status. Open channel to Twine. Ask Twine for a return channel. Operator tools toggle |

**Path A (two phones).**

1. On User A, open **SELL CKB** → **+**. Post an offer, e.g. available 2 CKB, currency `NGN`, price `2000` (2000 NGN/CKB), min `2000`, max `4000`, payment `Opay`.
2. On User A, BUY CKB stays empty (that ad is theirs). On **SELL CKB** it shows **YOUR OFFER**.
3. On User B, **BUY CKB** shows the same card without YOUR OFFER. Tap it, pay an amount inside the limit (e.g. `2000` NGN → 1 CKB hold), **Start trade**.
4. On User A, **My trades** → the trade → **Lock** (`send_payment` on A’s `fnn`).
5. On User B, pay fiat outside the app, then **Fiat sent** (B’s `fnn` creates the payout invoice).
6. On User A, **Release**. The daemon pays B, then `settle_invoice`. State `Settled`, invoice `Paid`. Leftover 1 CKB stays listed if it still meets the minimum.

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

When you are done:

```bash
./scripts/stop-nodes.sh
```

That stops the daemon (if `nodes/daemon.pid` exists) and the three nodes. Quit each `flutter run` with `q`.

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

App launch (`--dart-define`, compile-time; used when SharedPreferences has no value yet):

| Define | Default | |
| --- | --- | --- |
| `TWINE_FIBER_RPC` | `http://127.0.0.1:8227` | this phone’s `fnn` |
| `TWINE_P2P` | `/ip4/127.0.0.1/tcp/8228` | this phone’s P2P address |
| `TWINE_PUBKEY` | unset | Fiber pubkey (user id). Empty until **Read node info** if omitted |
| `TWINE_DAEMON_URL` | `http://127.0.0.1:8080` (iOS / desktop) | Twine daemon. Android emulator default is `http://10.0.2.2:8080` |

P2P ports are User A `8228`, Twine `8238`, User B `8248`.

## Out of scope

Nostr, encrypted DMs, key rotation, bonds, fees, UDT, accounts, buy-side ads, rate oracles, partial fills, and app-store builds. Dispute chat is plain text stored on the daemon. Pubkeys on ads and trades are not authenticated.
