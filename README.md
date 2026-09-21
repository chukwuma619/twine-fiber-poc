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
