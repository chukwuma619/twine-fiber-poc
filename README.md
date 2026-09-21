# Twine Fiber POC

A testnet proof of concept. One Rust daemon, one Flutter app, real Fiber testnet CKB. Rough on purpose. It exists so someone can create an order and walk the four payment paths.

Build order: [PLAN.md](PLAN.md).

```text
twine-fiber-poc/
  daemon/     Rust coordinator. Talks to fnn over JSON-RPC. Holds preimage S.
  app/        Flutter client. Seller, Buyer, and Solver roles.
```

Three Fiber testnet nodes sit beside the daemon: seller, Twine, and buyer. The app never holds a Fiber key. It calls the daemon. The daemon calls `fnn`.

Fiat is a button. The CKB movement is real: `new_invoice`, `send_payment`, `settle_invoice`, and TLC expiry on testnet (`Fibt`).

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
