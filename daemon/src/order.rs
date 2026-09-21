use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::health::Config;
use crate::rpc::FiberRpc;

const SHANNONS_PER_CKB: u128 = 100_000_000;
const LOCK_POLL: Duration = Duration::from_millis(500);
const LOCK_TIMEOUT: Duration = Duration::from_secs(90);
const PAY_POLL: Duration = Duration::from_millis(500);
const PAY_TIMEOUT: Duration = Duration::from_secs(90);

/// Fiber `new_invoice` `final_expiry_delta` in milliseconds.
/// Documented minimum is 16 hours. Probed on this fnn v0.9.1 Twine node: accepted;
/// created invoice attribute `final_htlc_minimum_expiry_delta` = `0x36ee800`.
pub const FINAL_EXPIRY_DELTA_MS: u64 = 57_600_000;

/// How often the daemon polls Twine `get_invoice` for Path D expiry.
pub const HOLD_EXPIRY_POLL: Duration = Duration::from_secs(15);

#[derive(Clone, Debug)]
pub struct OrderStore {
    path: PathBuf,
    inner: Arc<Mutex<Order>>,
}

/// Persisted order. `payment_preimage` stays on disk / in the daemon only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    pub state: OrderState,
    pub amount: Option<String>,
    #[serde(default)]
    pub payment_hash: Option<String>,
    #[serde(default)]
    pub payment_preimage: Option<String>,
    #[serde(default)]
    pub invoice_address: Option<String>,
    #[serde(default)]
    pub invoice_status: Option<String>,
    /// Seller local balance on seller→twine while the hold is Received (payment in flight).
    /// Used for Path D comparison after TLC expiry. Never returned in `OrderView`.
    #[serde(default)]
    pub seller_local_while_held: Option<String>,
    pub log: Vec<LogLine>,
    #[serde(default)]
    pub chat: Vec<ChatLine>,
}

/// Public order view. Never includes `payment_preimage`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OrderView {
    pub state: OrderState,
    pub amount: Option<String>,
    pub payment_hash: Option<String>,
    pub invoice_address: Option<String>,
    pub invoice_status: Option<String>,
    pub log: Vec<LogLine>,
    pub chat: Vec<ChatLine>,
}

impl Order {
    fn idle() -> Self {
        Self {
            state: OrderState::Idle,
            amount: None,
            payment_hash: None,
            payment_preimage: None,
            invoice_address: None,
            invoice_status: None,
            seller_local_while_held: None,
            log: Vec::new(),
            chat: Vec::new(),
        }
    }

    /// States where a Received hold may still expire on Fiber (Path D).
    fn watches_hold_invoice(state: OrderState) -> bool {
        matches!(
            state,
            OrderState::Held
                | OrderState::WaitingFiat
                | OrderState::FiatSent
                | OrderState::Leg2Failed
                | OrderState::Disputed
                | OrderState::Releasing
        )
    }

    pub fn view(&self) -> OrderView {
        OrderView {
            state: self.state,
            amount: self.amount.clone(),
            payment_hash: self.payment_hash.clone(),
            invoice_address: self.invoice_address.clone(),
            invoice_status: self.invoice_status.clone(),
            log: self.log.clone(),
            chat: self.chat.clone(),
        }
    }

    fn is_open(&self) -> bool {
        !matches!(
            self.state,
            OrderState::Idle
                | OrderState::Cancelled
                | OrderState::Paid
                | OrderState::Settled
                | OrderState::Expired
        )
    }

    fn push_log(&mut self, text: impl Into<String>) {
        self.log.push(LogLine {
            at: timestamp(),
            text: text.into(),
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum OrderState {
    Idle,
    Pending,
    WaitingHold,
    Held,
    WaitingFiat,
    FiatSent,
    Releasing,
    Leg2Failed,
    Disputed,
    Cancelled,
    Paid,
    Settled,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub at: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatLine {
    pub at: String,
    pub from: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct PostChatBody {
    pub from: String,
    pub text: String,
}

/// How Path A payment failure updates order state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PayFailureMode {
    /// Stage 3/4: move to Leg2Failed.
    Leg2Failed,
    /// Stage 5 Path C buyer-wins: stay Disputed, do not settle.
    StayDisputed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OrderError {
    BadAmount(String),
    AlreadyOpen,
    BadState(String),
    Fiber(String),
    Save(String),
}

#[derive(Debug, Deserialize)]
pub struct CreateOrderBody {
    pub amount: String,
}

impl OrderStore {
    pub fn from_env() -> Result<Self, String> {
        let path = match std::env::var("ORDER_FILE") {
            Ok(value) if !value.is_empty() => value,
            _ => "order.json".to_string(),
        };
        Self::open(PathBuf::from(path))
    }

    pub fn open(path: PathBuf) -> Result<Self, String> {
        let order = if path.exists() {
            let text = fs::read_to_string(&path).map_err(|err| err.to_string())?;
            serde_json::from_str(&text)
                .map_err(|err| format!("order file {}: {err}", path.display()))?
        } else {
            Order::idle()
        };
        Ok(Self {
            path,
            inner: Arc::new(Mutex::new(order)),
        })
    }

    pub fn snapshot(&self) -> OrderView {
        self.guard().view()
    }

    pub fn create(&self, amount: &str) -> Result<OrderView, OrderError> {
        let amount = normalize_amount(amount).map_err(OrderError::BadAmount)?;
        let mut order = self.guard();
        if order.is_open() {
            return Err(OrderError::AlreadyOpen);
        }
        *order = Order {
            state: OrderState::Pending,
            amount: Some(amount.clone()),
            payment_hash: None,
            payment_preimage: None,
            invoice_address: None,
            invoice_status: None,
            seller_local_while_held: None,
            log: vec![LogLine {
                at: timestamp(),
                text: format!("created order for {amount} CKB"),
            }],
            chat: Vec::new(),
        };
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!("order created amount={amount} CKB state=Pending");
        Ok(order.view())
    }

    /// Create a throwaway Open invoice and cancel it. Does not change the live order state.
    pub async fn demo_cancel(
        &self,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<OrderView, OrderError> {
        let amount = {
            let order = self.guard();
            order
                .amount
                .clone()
                .ok_or_else(|| OrderError::BadState("create an order first".into()))?
        };
        let shannon = amount_to_shannon(&amount).map_err(OrderError::BadAmount)?;
        let (_s, payment_hash) = generate_preimage();
        let created = rpc
            .call(
                &config.twine_rpc,
                "new_invoice",
                json!([{
                    "amount": hex_u128(shannon),
                    "currency": "Fibt",
                    "description": "twine demo cancel",
                    "payment_hash": payment_hash,
                    "hash_algorithm": "sha256",
                }]),
            )
            .await
            .map_err(OrderError::Fiber)?;
        let address = invoice_address(&created).unwrap_or_default();
        let cancelled = rpc
            .call(
                &config.twine_rpc,
                "cancel_invoice",
                json!([{ "payment_hash": payment_hash }]),
            )
            .await
            .map_err(OrderError::Fiber)?;
        let status = invoice_status(&cancelled).unwrap_or_else(|| "Cancelled".into());

        let mut order = self.guard();
        let live_state = order.state;
        order.push_log(format!(
            "demo cancel: unpaid invoice {status} H={payment_hash} (live order unchanged, still {live_state:?})"
        ));
        if !address.is_empty() {
            order.push_log(format!("demo cancel invoice was {address}"));
        }
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!("demo cancel H={payment_hash} status={status}");
        Ok(order.view())
    }

    pub async fn create_hold(
        &self,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<OrderView, OrderError> {
        let (amount, state) = {
            let order = self.guard();
            (order.amount.clone(), order.state)
        };
        if state != OrderState::Pending {
            return Err(OrderError::BadState(format!(
                "create hold needs Pending, got {state:?}"
            )));
        }
        let amount = amount.ok_or_else(|| OrderError::BadState("order has no amount".into()))?;
        let shannon = amount_to_shannon(&amount).map_err(OrderError::BadAmount)?;
        let (preimage, payment_hash) = generate_preimage();

        let created = rpc
            .call(
                &config.twine_rpc,
                "new_invoice",
                json!([{
                    "amount": hex_u128(shannon),
                    "currency": "Fibt",
                    "description": format!("twine order {amount} CKB"),
                    "payment_hash": payment_hash,
                    "hash_algorithm": "sha256",
                    "final_expiry_delta": hex_u64(FINAL_EXPIRY_DELTA_MS),
                }]),
            )
            .await
            .map_err(OrderError::Fiber)?;
        let address = invoice_address(&created)
            .ok_or_else(|| OrderError::Fiber("new_invoice missing invoice_address".into()))?;
        let status = invoice_status(&created).unwrap_or_else(|| "Open".into());
        let attr_delta = invoice_final_expiry_delta(&created);
        if let Some(attr) = attr_delta.as_deref() {
            let expected = hex_u64(FINAL_EXPIRY_DELTA_MS);
            if normalize_hex(attr) != normalize_hex(&expected) {
                return Err(OrderError::Fiber(format!(
                    "new_invoice final_htlc_minimum_expiry_delta={attr} did not match requested {expected}"
                )));
            }
        }

        let mut order = self.guard();
        if order.state != OrderState::Pending {
            return Err(OrderError::BadState(
                "order changed while creating hold".into(),
            ));
        }
        order.state = OrderState::WaitingHold;
        order.payment_hash = Some(payment_hash.clone());
        order.payment_preimage = Some(preimage);
        order.invoice_address = Some(address.clone());
        order.invoice_status = Some(status.clone());
        order.push_log(format!(
            "hold invoice created H={payment_hash} final_expiry_delta={FINAL_EXPIRY_DELTA_MS}ms ({}) S sealed in daemon (never sent to app)",
            hex_u64(FINAL_EXPIRY_DELTA_MS)
        ));
        order.push_log(format!("invoice address {address}"));
        order.push_log(format!("invoice status {status}"));
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!(
            "hold created H={payment_hash} final_expiry_delta={FINAL_EXPIRY_DELTA_MS}ms state=WaitingHold S sealed"
        );
        Ok(order.view())
    }

    pub async fn lock_payment(
        &self,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<OrderView, OrderError> {
        let (state, address, payment_hash) = {
            let order = self.guard();
            (
                order.state,
                order.invoice_address.clone(),
                order.payment_hash.clone(),
            )
        };
        if state != OrderState::WaitingHold {
            return Err(OrderError::BadState(format!(
                "lock needs WaitingHold, got {state:?}"
            )));
        }
        let address =
            address.ok_or_else(|| OrderError::BadState("missing invoice address".into()))?;
        let payment_hash =
            payment_hash.ok_or_else(|| OrderError::BadState("missing payment hash".into()))?;

        rpc.call(
            &config.seller_rpc,
            "send_payment",
            json!([{
                "invoice": address,
                "max_fee_amount": "0x5f5e100",
            }]),
        )
        .await
        .map_err(OrderError::Fiber)?;

        let status = poll_invoice_received(rpc, &config.twine_rpc, &payment_hash).await?;
        let seller_local = seller_local_balance(rpc, config).await.ok();

        let mut order = self.guard();
        if order.state != OrderState::WaitingHold {
            return Err(OrderError::BadState(
                "order changed while locking".into(),
            ));
        }
        order.state = OrderState::Held;
        order.invoice_status = Some(status.clone());
        if let Some(local) = seller_local.clone() {
            order.seller_local_while_held = Some(local);
        }
        order.push_log(format!(
            "seller locked: send_payment submitted, get_invoice={status}"
        ));
        order.push_log(format!(
            "H={payment_hash} S still sealed in daemon; Twine spendable balance unchanged until settle"
        ));
        if let Some(local) = seller_local {
            order.push_log(format!(
                "seller→twine local while payment in flight={local}"
            ));
        }
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!("locked H={payment_hash} invoice={status} state=Held");
        Ok(order.view())
    }

    /// After Received, do not call cancel_invoice on the live hold — it would destroy
    /// the trade. Fiber docs say cancel is only legal while Open; we refuse locally.
    pub async fn try_cancel(
        &self,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<OrderView, OrderError> {
        let (state, payment_hash) = {
            let order = self.guard();
            (order.state, order.payment_hash.clone())
        };
        let payment_hash =
            payment_hash.ok_or_else(|| OrderError::BadState("no hold invoice yet".into()))?;

        let current = rpc
            .call(
                &config.twine_rpc,
                "get_invoice",
                json!([{ "payment_hash": payment_hash }]),
            )
            .await
            .map_err(OrderError::Fiber)?;
        let status = invoice_status(&current).unwrap_or_else(|| "unknown".into());

        if status != "Open" {
            let mut order = self.guard();
            order.invoice_status = Some(status.clone());
            order.push_log(format!(
                "cancel_invoice not applied (state={state:?}, invoice={status}): only legal while Open; after Received the seller refund is TLC expiry, not cancel"
            ));
            save(&self.path, &order).map_err(OrderError::Save)?;
            return Ok(order.view());
        }

        match rpc
            .call(
                &config.twine_rpc,
                "cancel_invoice",
                json!([{ "payment_hash": payment_hash }]),
            )
            .await
        {
            Ok(result) => {
                let status = invoice_status(&result).unwrap_or_else(|| "Cancelled".into());
                let mut order = self.guard();
                order.state = OrderState::Cancelled;
                order.invoice_status = Some(status.clone());
                order.push_log(format!(
                    "cancel_invoice succeeded: invoice {status} H={payment_hash}"
                ));
                save(&self.path, &order).map_err(OrderError::Save)?;
                Ok(order.view())
            }
            Err(err) => {
                let mut order = self.guard();
                order.push_log(format!(
                    "cancel_invoice refused (state={state:?}, invoice={status}): {err}"
                ));
                save(&self.path, &order).map_err(OrderError::Save)?;
                Ok(order.view())
            }
        }
    }

    pub fn accept(&self) -> Result<OrderView, OrderError> {
        let mut order = self.guard();
        if order.state != OrderState::Held {
            return Err(OrderError::BadState(format!(
                "accept needs Held, got {:?}",
                order.state
            )));
        }
        order.state = OrderState::WaitingFiat;
        order.push_log("buyer accepted; start fiat timer in the app");
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!("state=WaitingFiat");
        Ok(order.view())
    }

    pub fn fiat_sent(&self) -> Result<OrderView, OrderError> {
        let mut order = self.guard();
        if order.state != OrderState::WaitingFiat {
            return Err(OrderError::BadState(format!(
                "fiat sent needs WaitingFiat, got {:?}",
                order.state
            )));
        }
        order.state = OrderState::FiatSent;
        order.push_log("buyer marked fiat sent");
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!("state=FiatSent");
        Ok(order.view())
    }

    /// Stage 3 Path A / Stage 4 Path B: pay buyer from Twine, then settle_invoice(H, S).
    /// Accepts FiatSent or Releasing so a stage-2 order can continue.
    /// On buyer payment failure → Leg2Failed (no settle, no auto-retry).
    pub async fn release(
        &self,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<OrderView, OrderError> {
        let state = self.guard().state;
        if state != OrderState::FiatSent && state != OrderState::Releasing {
            return Err(OrderError::BadState(format!(
                "release needs FiatSent or Releasing, got {state:?}"
            )));
        }

        {
            let mut order = self.guard();
            if order.state == OrderState::FiatSent {
                order.state = OrderState::Releasing;
                order.push_log("seller released; path A: pay buyer then settle hold");
                save(&self.path, &order).map_err(OrderError::Save)?;
                eprintln!("state=Releasing (path A)");
            } else {
                order.push_log("path A continue from Releasing: pay buyer then settle hold");
                save(&self.path, &order).map_err(OrderError::Save)?;
            }
        }

        self.pay_buyer_then_settle(rpc, config, PayFailureMode::Leg2Failed)
            .await
    }

    /// Stage 4 Path B retry: from Leg2Failed, create a new buyer invoice and run path A.
    pub async fn retry(
        &self,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<OrderView, OrderError> {
        let state = self.guard().state;
        if state != OrderState::Leg2Failed {
            return Err(OrderError::BadState(format!(
                "retry needs Leg2Failed, got {state:?}"
            )));
        }

        {
            let mut order = self.guard();
            order.state = OrderState::Releasing;
            order.push_log(
                "retry: buyer submitted a new invoice path; path A: pay buyer then settle hold",
            );
            save(&self.path, &order).map_err(OrderError::Save)?;
            eprintln!("state=Releasing (path B retry → path A)");
        }

        self.pay_buyer_then_settle(rpc, config, PayFailureMode::Leg2Failed)
            .await
    }

    /// Stage 5 Path C: open a dispute from WaitingFiat / FiatSent / Leg2Failed while hold is Received.
    pub async fn open_dispute(
        &self,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<OrderView, OrderError> {
        let (state, payment_hash) = {
            let order = self.guard();
            (order.state, order.payment_hash.clone())
        };
        match state {
            OrderState::WaitingFiat | OrderState::FiatSent | OrderState::Leg2Failed => {}
            other => {
                return Err(OrderError::BadState(format!(
                    "open dispute needs WaitingFiat, FiatSent, or Leg2Failed, got {other:?}"
                )));
            }
        }
        let payment_hash =
            payment_hash.ok_or_else(|| OrderError::BadState("missing hold payment hash".into()))?;

        let status = fetch_invoice_status(rpc, &config.twine_rpc, &payment_hash).await?;
        require_received_for_dispute(&status)?;

        let mut order = self.guard();
        match order.state {
            OrderState::WaitingFiat | OrderState::FiatSent | OrderState::Leg2Failed => {}
            other => {
                return Err(OrderError::BadState(format!(
                    "order changed while opening dispute, got {other:?}"
                )));
            }
        }
        order.state = OrderState::Disputed;
        order.invoice_status = Some(status.clone());
        order.push_log(format!(
            "dispute opened (invoice still {status}); chat is plain text on the daemon"
        ));
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!("state=Disputed H={payment_hash}");
        Ok(order.view())
    }

    /// Stage 5 Path C: post a plain-text chat line while Disputed.
    pub fn post_chat(&self, body: &PostChatBody) -> Result<OrderView, OrderError> {
        let from = body.from.trim().to_ascii_lowercase();
        if from != "buyer" && from != "seller" {
            return Err(OrderError::BadState(
                "chat from must be buyer or seller".into(),
            ));
        }
        let text = body.text.trim();
        if text.is_empty() {
            return Err(OrderError::BadState("chat text is required".into()));
        }
        if text.len() > 500 {
            return Err(OrderError::BadState(
                "chat text must be at most 500 characters".into(),
            ));
        }

        let mut order = self.guard();
        if order.state != OrderState::Disputed {
            return Err(OrderError::BadState(format!(
                "post chat needs Disputed, got {:?}",
                order.state
            )));
        }
        order.chat.push(ChatLine {
            at: timestamp(),
            from: from.clone(),
            text: text.to_string(),
        });
        order.push_log(format!("chat ({from}): {text}"));
        save(&self.path, &order).map_err(OrderError::Save)?;
        Ok(order.view())
    }

    /// Stage 5 Path C buyer wins: same pay+settle as Path A. Route fail → stay Disputed.
    pub async fn award_buyer(
        &self,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<OrderView, OrderError> {
        let (state, payment_hash) = {
            let order = self.guard();
            (order.state, order.payment_hash.clone())
        };
        if state != OrderState::Disputed {
            return Err(OrderError::BadState(format!(
                "award buyer needs Disputed, got {state:?}"
            )));
        }
        let payment_hash =
            payment_hash.ok_or_else(|| OrderError::BadState("missing hold payment hash".into()))?;

        let status = fetch_invoice_status(rpc, &config.twine_rpc, &payment_hash).await?;
        require_received_for_award(&status)?;

        {
            let mut order = self.guard();
            if order.state != OrderState::Disputed {
                return Err(OrderError::BadState(
                    "order changed while awarding buyer".into(),
                ));
            }
            order.invoice_status = Some(status);
            order.push_log(
                "solver awarded buyer; path C → path A: pay buyer then settle hold",
            );
            save(&self.path, &order).map_err(OrderError::Save)?;
            eprintln!("path C award buyer H={payment_hash}");
        }

        self.pay_buyer_then_settle(rpc, config, PayFailureMode::StayDisputed)
            .await
    }

    /// Stage 5 Path C seller wins: do not settle or cancel. Hold stays Received until TLC expiry.
    pub async fn award_seller(
        &self,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<OrderView, OrderError> {
        let (state, payment_hash) = {
            let order = self.guard();
            (order.state, order.payment_hash.clone())
        };
        if state != OrderState::Disputed {
            return Err(OrderError::BadState(format!(
                "award seller needs Disputed, got {state:?}"
            )));
        }
        let payment_hash =
            payment_hash.ok_or_else(|| OrderError::BadState("missing hold payment hash".into()))?;

        let status = fetch_invoice_status(rpc, &config.twine_rpc, &payment_hash).await?;
        require_received_for_award(&status)?;

        let mut order = self.guard();
        if order.state != OrderState::Disputed {
            return Err(OrderError::BadState(
                "order changed while awarding seller".into(),
            ));
        }
        order.invoice_status = Some(status.clone());
        order.push_log(format!(
            "solver awarded seller: hold stays {status}; settle_invoice not called; cancel_invoice not called; seller refund at TLC expiry"
        ));
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!(
            "path C award seller H={payment_hash} invoice={status} (no settle, no cancel)"
        );
        Ok(order.view())
    }

    /// Shared Path A payment + settle. Payment failure disposition depends on mode.
    async fn pay_buyer_then_settle(
        &self,
        rpc: &FiberRpc,
        config: &Config,
        on_failure: PayFailureMode,
    ) -> Result<OrderView, OrderError> {
        let (amount, hold_hash, hold_preimage) = {
            let order = self.guard();
            (
                order.amount.clone(),
                order.payment_hash.clone(),
                order.payment_preimage.clone(),
            )
        };
        let amount = amount.ok_or_else(|| OrderError::BadState("order has no amount".into()))?;
        let hold_hash =
            hold_hash.ok_or_else(|| OrderError::BadState("missing hold payment hash".into()))?;
        let hold_preimage = hold_preimage
            .ok_or_else(|| OrderError::BadState("missing hold payment preimage".into()))?;
        let shannon = amount_to_shannon(&amount).map_err(OrderError::BadAmount)?;

        let buyer_invoice = rpc
            .call(
                &config.buyer_rpc,
                "new_invoice",
                json!([{
                    "amount": hex_u128(shannon),
                    "currency": "Fibt",
                    "description": format!("twine path A buyer {amount} CKB"),
                    "hash_algorithm": "sha256",
                }]),
            )
            .await
            .map_err(OrderError::Fiber)?;
        let buyer_address = invoice_address(&buyer_invoice).ok_or_else(|| {
            OrderError::Fiber("buyer new_invoice missing invoice_address".into())
        })?;
        let buyer_payment_hash = invoice_payment_hash(&buyer_invoice).ok_or_else(|| {
            OrderError::Fiber("buyer new_invoice missing payment_hash".into())
        })?;

        {
            let mut order = self.guard();
            order.push_log(format!(
                "pay: buyer invoice created H={buyer_payment_hash} (normal invoice, preimage on buyer node)"
            ));
            save(&self.path, &order).map_err(OrderError::Save)?;
        }

        // Optional pause so a Path B demo can stop the buyer node after new_invoice
        // and before send_payment (TWINE_RELEASE_PAUSE_MS, e.g. 3000).
        if let Ok(raw) = std::env::var("TWINE_RELEASE_PAUSE_MS") {
            if let Ok(ms) = raw.parse::<u64>() {
                if ms > 0 {
                    eprintln!("TWINE_RELEASE_PAUSE_MS={ms}: pausing before send_payment");
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                }
            }
        }

        let sent = match rpc
            .call(
                &config.twine_rpc,
                "send_payment",
                json!([{
                    "invoice": buyer_address,
                    "max_fee_amount": "0x5f5e100",
                }]),
            )
            .await
        {
            Ok(sent) => sent,
            Err(err) => {
                return self.record_pay_failure(
                    format!("send_payment failed: {err}"),
                    on_failure,
                );
            }
        };
        let pay_hash = payment_hash_of(&sent).unwrap_or(buyer_payment_hash.clone());

        {
            let mut order = self.guard();
            order.push_log(format!(
                "pay: twine send_payment submitted payment_hash={pay_hash}"
            ));
            save(&self.path, &order).map_err(OrderError::Save)?;
        }

        match poll_payment_done(rpc, &config.twine_rpc, &pay_hash).await {
            Ok(status) => {
                let mut order = self.guard();
                order.push_log(format!("pay: get_payment={status}"));
                save(&self.path, &order).map_err(OrderError::Save)?;
            }
            Err(err) => {
                let detail = match &err {
                    OrderError::Fiber(message) => message.clone(),
                    other => format!("{other:?}"),
                };
                return self.record_pay_failure(detail, on_failure);
            }
        }

        rpc.call(
            &config.twine_rpc,
            "settle_invoice",
            json!([{
                "payment_hash": hold_hash,
                "payment_preimage": hold_preimage,
            }]),
        )
        .await
        .map_err(OrderError::Fiber)?;

        {
            let mut order = self.guard();
            order.push_log("settle: settle_invoice(H, S) submitted on twine");
            save(&self.path, &order).map_err(OrderError::Save)?;
        }

        let status = poll_invoice_paid(rpc, &config.twine_rpc, &hold_hash).await?;

        let mut order = self.guard();
        order.state = OrderState::Settled;
        order.invoice_status = Some(status.clone());
        order.push_log(format!("settle: hold invoice={status}"));
        order.push_log(match on_failure {
            PayFailureMode::Leg2Failed => "path A complete: state Settled".to_string(),
            PayFailureMode::StayDisputed => {
                "path C buyer wins complete: state Settled".to_string()
            }
        });
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!("settled H={hold_hash} invoice={status} state=Settled");
        Ok(order.view())
    }

    /// Path B / Path C payment failure. Hold untouched; do not cancel_invoice.
    fn record_pay_failure(
        &self,
        detail: String,
        mode: PayFailureMode,
    ) -> Result<OrderView, OrderError> {
        match mode {
            PayFailureMode::Leg2Failed => self.record_leg2_failed(detail),
            PayFailureMode::StayDisputed => self.record_award_buyer_failed(detail),
        }
    }

    /// Path B: payment to buyer failed. Hold untouched; do not cancel_invoice.
    /// Returns Ok so the client observes Leg2Failed before any separate retry.
    fn record_leg2_failed(&self, detail: String) -> Result<OrderView, OrderError> {
        let mut order = self.guard();
        order.state = OrderState::Leg2Failed;
        order.push_log(format!(
            "pay failed: {detail}; settle_invoice not called (hold stays Received)"
        ));
        order.push_log(
            "path B: buyer should submit a new invoice to retry (POST /order/retry)",
        );
        order.push_log(
            "if the buyer never returns, the seller is refunded when the TLC expires (do not cancel_invoice on Received)",
        );
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!("path B Leg2Failed: {detail}");
        Ok(order.view())
    }

    /// Path C buyer-wins payment failed: stay Disputed, do not settle.
    fn record_award_buyer_failed(&self, detail: String) -> Result<OrderView, OrderError> {
        let mut order = self.guard();
        order.state = OrderState::Disputed;
        order.push_log(format!(
            "path C buyer award pay failed: {detail}; settle_invoice not called; stay Disputed (hold stays Received)"
        ));
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!("path C award buyer failed, stay Disputed: {detail}");
        Ok(order.view())
    }

    /// Stage 6 Path D: one poll of Twine `get_invoice`. When Fiber marks the hold
    /// `Expired`, move to `Expired`, log the seller refund, attempt one failing
    /// `settle_invoice`, and record seller channel balances. Never calls `cancel_invoice`.
    pub async fn poll_hold_expiry(
        &self,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<Option<OrderView>, OrderError> {
        let (state, payment_hash, already_expired) = {
            let order = self.guard();
            (
                order.state,
                order.payment_hash.clone(),
                order.state == OrderState::Expired,
            )
        };
        if already_expired || !Order::watches_hold_invoice(state) {
            return Ok(None);
        }
        let payment_hash = match payment_hash {
            Some(hash) => hash,
            None => return Ok(None),
        };

        let current = rpc
            .call(
                &config.twine_rpc,
                "get_invoice",
                json!([{ "payment_hash": payment_hash }]),
            )
            .await
            .map_err(OrderError::Fiber)?;
        let status = invoice_status(&current).unwrap_or_else(|| "unknown".into());

        if status == "Received" {
            let seller_local = seller_local_balance(rpc, config).await.ok();
            let mut order = self.guard();
            if !Order::watches_hold_invoice(order.state) {
                return Ok(None);
            }
            order.invoice_status = Some(status);
            if let Some(local) = seller_local {
                order.seller_local_while_held = Some(local);
            }
            save(&self.path, &order).map_err(OrderError::Save)?;
            return Ok(None);
        }

        if status != "Expired" {
            let mut order = self.guard();
            if Order::watches_hold_invoice(order.state) {
                order.invoice_status = Some(status);
                save(&self.path, &order).map_err(OrderError::Save)?;
            }
            return Ok(None);
        }

        self.apply_path_d_expiry(rpc, config, &payment_hash)
            .await
            .map(Some)
    }

    async fn apply_path_d_expiry(
        &self,
        rpc: &FiberRpc,
        config: &Config,
        payment_hash: &str,
    ) -> Result<OrderView, OrderError> {
        let (preimage, in_flight) = {
            let order = self.guard();
            (
                order.payment_preimage.clone(),
                order.seller_local_while_held.clone(),
            )
        };

        // Attempt settle once so acceptance can show it fails after expiry.
        // A failed settle is not a successful settle; do not call cancel_invoice.
        let settle_err = match preimage {
            Some(preimage) => match rpc
                .call(
                    &config.twine_rpc,
                    "settle_invoice",
                    json!([{
                        "payment_hash": payment_hash,
                        "payment_preimage": preimage,
                    }]),
                )
                .await
            {
                Ok(_) => Some(
                    "settle_invoice returned ok after Expired (unexpected; not treated as Path A settle)"
                        .to_string(),
                ),
                Err(err) => Some(err),
            },
            None => Some("missing sealed preimage S; settle_invoice not attempted".into()),
        };

        // Give the channel a moment to restore local balance after TLC expiry.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let after = seller_local_balance(rpc, config).await.ok();

        let mut order = self.guard();
        if order.state == OrderState::Expired {
            return Ok(order.view());
        }
        if !Order::watches_hold_invoice(order.state) {
            return Err(OrderError::BadState(format!(
                "path D expiry interrupted; order is {:?}",
                order.state
            )));
        }
        order.state = OrderState::Expired;
        order.invoice_status = Some("Expired".into());
        order.push_log(format!(
            "path D: hold invoice Expired H={payment_hash}; seller payment failed back; seller refunded because the TLC expired"
        ));
        order.push_log(
            "path D: cancel_invoice not called (not legal / not needed after Received→Expired)",
        );
        if let Some(err) = settle_err {
            order.push_log(format!(
                "path D: settle_invoice(H, S) after expiry failed as expected: {err} (not a successful settle)"
            ));
        }
        match (&in_flight, &after) {
            (Some(before), Some(after_bal)) => {
                order.push_log(format!(
                    "path D: seller→twine local while payment in flight={before}; after expiry={after_bal}"
                ));
            }
            (Some(before), None) => {
                order.push_log(format!(
                    "path D: seller→twine local while payment in flight={before}; after expiry unavailable"
                ));
            }
            (None, Some(after_bal)) => {
                order.push_log(format!(
                    "path D: seller→twine local while in flight unavailable; after expiry={after_bal}"
                ));
            }
            (None, None) => {
                order.push_log(
                    "path D: seller→twine local balances unavailable around expiry",
                );
            }
        }
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!(
            "path D Expired H={payment_hash} (seller refunded at TLC expiry; settle failed; no cancel)"
        );
        Ok(order.view())
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, Order> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

async fn fetch_invoice_status(
    rpc: &FiberRpc,
    twine_rpc: &str,
    payment_hash: &str,
) -> Result<String, OrderError> {
    let result = rpc
        .call(
            twine_rpc,
            "get_invoice",
            json!([{ "payment_hash": payment_hash }]),
        )
        .await
        .map_err(OrderError::Fiber)?;
    Ok(invoice_status(&result).unwrap_or_else(|| "unknown".into()))
}

fn require_received_for_dispute(status: &str) -> Result<(), OrderError> {
    if status == "Received" {
        return Ok(());
    }
    Err(OrderError::BadState(format!(
        "open dispute needs hold invoice Received, got {status}"
    )))
}

fn require_received_for_award(status: &str) -> Result<(), OrderError> {
    if status == "Expired" {
        return Err(OrderError::BadState(
            "award refused: hold invoice is Expired; neither buyer nor seller award can run"
                .into(),
        ));
    }
    if status == "Received" {
        return Ok(());
    }
    Err(OrderError::BadState(format!(
        "award needs hold invoice Received, got {status}"
    )))
}

async fn poll_invoice_received(
    rpc: &FiberRpc,
    twine_rpc: &str,
    payment_hash: &str,
) -> Result<String, OrderError> {
    let deadline = tokio::time::Instant::now() + LOCK_TIMEOUT;
    loop {
        let result = rpc
            .call(
                twine_rpc,
                "get_invoice",
                json!([{ "payment_hash": payment_hash }]),
            )
            .await
            .map_err(OrderError::Fiber)?;
        let status = invoice_status(&result).unwrap_or_else(|| "unknown".into());
        if status == "Received" {
            return Ok(status);
        }
        if status == "Paid" || status == "Cancelled" || status == "Expired" {
            return Err(OrderError::Fiber(format!(
                "invoice reached {status} while waiting for Received"
            )));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(OrderError::Fiber(format!(
                "timed out waiting for Received (last status {status})"
            )));
        }
        tokio::time::sleep(LOCK_POLL).await;
    }
}

async fn poll_payment_done(
    rpc: &FiberRpc,
    node_rpc: &str,
    payment_hash: &str,
) -> Result<String, OrderError> {
    let deadline = tokio::time::Instant::now() + PAY_TIMEOUT;
    loop {
        let result = rpc
            .call(
                node_rpc,
                "get_payment",
                json!([{ "payment_hash": payment_hash }]),
            )
            .await
            .map_err(OrderError::Fiber)?;
        let status = payment_status(&result).unwrap_or_else(|| "unknown".into());
        if status == "Success" {
            return Ok(status);
        }
        if status == "Failed" {
            let detail = result
                .get("failed_error")
                .and_then(Value::as_str)
                .unwrap_or("payment Failed");
            return Err(OrderError::Fiber(format!(
                "get_payment Failed for {payment_hash}: {detail}"
            )));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(OrderError::Fiber(format!(
                "timed out waiting for payment Success (last status {status})"
            )));
        }
        tokio::time::sleep(PAY_POLL).await;
    }
}

async fn poll_invoice_paid(
    rpc: &FiberRpc,
    twine_rpc: &str,
    payment_hash: &str,
) -> Result<String, OrderError> {
    let deadline = tokio::time::Instant::now() + PAY_TIMEOUT;
    loop {
        let result = rpc
            .call(
                twine_rpc,
                "get_invoice",
                json!([{ "payment_hash": payment_hash }]),
            )
            .await
            .map_err(OrderError::Fiber)?;
        let status = invoice_status(&result).unwrap_or_else(|| "unknown".into());
        if status == "Paid" {
            return Ok(status);
        }
        if status == "Cancelled" || status == "Expired" {
            return Err(OrderError::Fiber(format!(
                "invoice reached {status} while waiting for Paid after settle"
            )));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(OrderError::Fiber(format!(
                "timed out waiting for Paid after settle (last status {status})"
            )));
        }
        tokio::time::sleep(PAY_POLL).await;
    }
}

fn generate_preimage() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let preimage = format!("0x{}", hex::encode(bytes));
    let hash = Sha256::digest(bytes);
    let payment_hash = format!("0x{}", hex::encode(hash));
    (preimage, payment_hash)
}

fn invoice_address(value: &Value) -> Option<String> {
    value
        .get("invoice_address")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn invoice_status(value: &Value) -> Option<String> {
    value
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn invoice_payment_hash(value: &Value) -> Option<String> {
    value
        .pointer("/invoice/data/payment_hash")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            value
                .get("payment_hash")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn payment_hash_of(value: &Value) -> Option<String> {
    value
        .get("payment_hash")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn payment_status(value: &Value) -> Option<String> {
    value
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn hex_u128(value: u128) -> String {
    format!("0x{value:x}")
}

fn hex_u64(value: u64) -> String {
    format!("0x{value:x}")
}

fn normalize_hex(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X")
        .trim_start_matches('0')
        .to_ascii_lowercase()
}

fn invoice_final_expiry_delta(value: &Value) -> Option<String> {
    let attrs = value
        .pointer("/invoice/data/attrs")
        .and_then(Value::as_array)?;
    for attr in attrs {
        if let Some(delta) = attr
            .get("final_htlc_minimum_expiry_delta")
            .and_then(Value::as_str)
        {
            return Some(delta.to_string());
        }
    }
    None
}

async fn seller_local_balance(rpc: &FiberRpc, config: &Config) -> Result<String, OrderError> {
    let twine_info = rpc
        .call(&config.twine_rpc, "node_info", Value::Array(vec![]))
        .await
        .map_err(OrderError::Fiber)?;
    let twine_pubkey = twine_info
        .get("pubkey")
        .and_then(Value::as_str)
        .ok_or_else(|| OrderError::Fiber("twine node_info missing pubkey".into()))?;
    let listed = rpc
        .call(&config.seller_rpc, "list_channels", json!([{}]))
        .await
        .map_err(OrderError::Fiber)?;
    let channel = crate::health::channel_for_peer(&listed, twine_pubkey)
        .map_err(OrderError::Fiber)?
        .ok_or_else(|| OrderError::Fiber("seller→twine channel not found".into()))?;
    channel
        .get("local_balance")
        .and_then(|value| match value {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .ok_or_else(|| OrderError::Fiber("seller→twine channel missing local_balance".into()))
}

pub fn amount_to_shannon(amount: &str) -> Result<u128, String> {
    let normalized = normalize_amount(amount)?;
    let (whole, frac) = match normalized.split_once('.') {
        Some((whole, frac)) => (whole, frac),
        None => (normalized.as_str(), ""),
    };
    let whole: u128 = whole
        .parse()
        .map_err(|_| "amount is too large".to_string())?;
    let mut frac_digits = frac.to_string();
    while frac_digits.len() < 8 {
        frac_digits.push('0');
    }
    let frac: u128 = frac_digits
        .parse()
        .map_err(|_| "amount is too large".to_string())?;
    whole
        .checked_mul(SHANNONS_PER_CKB)
        .and_then(|v| v.checked_add(frac))
        .ok_or_else(|| "amount is too large".to_string())
}

fn save(path: &Path, order: &Order) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    let text = serde_json::to_string_pretty(order).map_err(|err| err.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|err| err.to_string())?;
    fs::rename(&tmp, path).map_err(|err| err.to_string())
}

pub fn normalize_amount(raw: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err("amount is required".to_string());
    }
    let (whole, frac) = match text.split_once('.') {
        Some((whole, frac)) => (whole, frac),
        None => (text, ""),
    };
    if whole.is_empty()
        || !whole.chars().all(|ch| ch.is_ascii_digit())
        || !frac.chars().all(|ch| ch.is_ascii_digit())
    {
        return Err("amount must be a positive number of CKB".to_string());
    }
    if frac.len() > 8 {
        return Err("amount supports at most 8 decimal places".to_string());
    }
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let frac = frac.trim_end_matches('0');
    if whole == "0" && frac.is_empty() {
        return Err("amount must be greater than zero".to_string());
    }
    if frac.is_empty() {
        Ok(whole.to_string())
    } else {
        Ok(format!("{whole}.{frac}"))
    }
}

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "twine-order-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn create_moves_idle_to_pending_and_reloads() {
        let path = temp_path();
        let store = OrderStore::open(path.clone()).unwrap();
        assert_eq!(store.snapshot().state, OrderState::Idle);

        let created = store.create("1.50").unwrap();
        assert_eq!(created.state, OrderState::Pending);
        assert_eq!(created.amount.as_deref(), Some("1.5"));
        assert_eq!(created.log.len(), 1);
        assert_eq!(created.log[0].text, "created order for 1.5 CKB");
        assert!(created.payment_hash.is_none());

        let reloaded = OrderStore::open(path.clone()).unwrap();
        assert_eq!(reloaded.snapshot(), created);
        assert!(reloaded.guard().payment_preimage.is_none());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn second_create_is_refused() {
        let path = temp_path();
        let store = OrderStore::open(path.clone()).unwrap();
        store.create("1").unwrap();
        assert_eq!(store.create("2").unwrap_err(), OrderError::AlreadyOpen);
        assert_eq!(store.snapshot().amount.as_deref(), Some("1"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn view_omits_preimage() {
        let path = temp_path();
        let store = OrderStore::open(path.clone()).unwrap();
        store.create("1").unwrap();
        {
            let mut order = store.guard();
            order.payment_preimage = Some("0xdead".into());
            order.payment_hash = Some("0xbeef".into());
        }
        let view = store.snapshot();
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("preimage"));
        assert!(!json.contains("0xdead"));
        assert!(json.contains("0xbeef"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn amount_one_ckb_is_hex_shannon() {
        assert_eq!(amount_to_shannon("1").unwrap(), 0x5f5e100);
        assert_eq!(hex_u128(amount_to_shannon("1").unwrap()), "0x5f5e100");
    }

    #[test]
    fn rejects_empty_zero_and_too_precise_amounts() {
        assert!(normalize_amount("").is_err());
        assert!(normalize_amount("0").is_err());
        assert!(normalize_amount("0.0").is_err());
        assert!(normalize_amount("1.123456789").is_err());
        assert_eq!(normalize_amount("0.00000001").as_deref(), Ok("0.00000001"));
    }

    #[test]
    fn leg2_failed_is_open() {
        let path = temp_path();
        let store = OrderStore::open(path.clone()).unwrap();
        store.create("1").unwrap();
        {
            let mut order = store.guard();
            order.state = OrderState::Leg2Failed;
        }
        assert!(store.guard().is_open());
        assert_eq!(store.create("2").unwrap_err(), OrderError::AlreadyOpen);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn disputed_is_open() {
        let path = temp_path();
        let store = OrderStore::open(path.clone()).unwrap();
        store.create("1").unwrap();
        {
            let mut order = store.guard();
            order.state = OrderState::Disputed;
        }
        assert!(store.guard().is_open());
        assert_eq!(store.create("2").unwrap_err(), OrderError::AlreadyOpen);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn award_buyer_pay_failure_stays_disputed() {
        let path = temp_path();
        let store = OrderStore::open(path.clone()).unwrap();
        store.create("1").unwrap();
        {
            let mut order = store.guard();
            order.state = OrderState::Disputed;
            order.payment_hash = Some("0xhold".into());
            order.invoice_status = Some("Received".into());
        }
        let view = store
            .record_award_buyer_failed("route failed in test".into())
            .unwrap();
        assert_eq!(view.state, OrderState::Disputed);
        assert_eq!(view.invoice_status.as_deref(), Some("Received"));
        assert!(view
            .log
            .iter()
            .any(|line| line.text.contains("stay Disputed")));
        assert!(view
            .log
            .iter()
            .any(|line| line.text.contains("settle_invoice not called")));
        assert!(!view
            .log
            .iter()
            .any(|line| line.text.contains("settle_invoice(H")));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn expired_invoice_refuses_both_awards() {
        let err = require_received_for_award("Expired").unwrap_err();
        assert_eq!(
            err,
            OrderError::BadState(
                "award refused: hold invoice is Expired; neither buyer nor seller award can run"
                    .into()
            )
        );
        assert!(require_received_for_award("Received").is_ok());
        assert!(require_received_for_dispute("Received").is_ok());
        assert!(require_received_for_dispute("Expired").is_err());
    }

    #[test]
    fn expired_is_closed_for_create() {
        let path = temp_path();
        let store = OrderStore::open(path.clone()).unwrap();
        store.create("1").unwrap();
        {
            let mut order = store.guard();
            order.state = OrderState::Expired;
        }
        assert!(!store.guard().is_open());
        let created = store.create("2").unwrap();
        assert_eq!(created.state, OrderState::Pending);
        assert_eq!(created.amount.as_deref(), Some("2"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn watches_hold_invoice_covers_path_d_states() {
        assert!(Order::watches_hold_invoice(OrderState::Held));
        assert!(Order::watches_hold_invoice(OrderState::WaitingFiat));
        assert!(Order::watches_hold_invoice(OrderState::FiatSent));
        assert!(Order::watches_hold_invoice(OrderState::Leg2Failed));
        assert!(Order::watches_hold_invoice(OrderState::Disputed));
        assert!(Order::watches_hold_invoice(OrderState::Releasing));
        assert!(!Order::watches_hold_invoice(OrderState::Expired));
        assert!(!Order::watches_hold_invoice(OrderState::Settled));
        assert!(!Order::watches_hold_invoice(OrderState::Pending));
    }

    #[test]
    fn final_expiry_delta_matches_probed_minimum() {
        assert_eq!(FINAL_EXPIRY_DELTA_MS, 57_600_000);
        assert_eq!(hex_u64(FINAL_EXPIRY_DELTA_MS), "0x36ee800");
        assert_eq!(normalize_hex("0x36ee800"), normalize_hex("0x036ee800"));
    }

    #[test]
    fn post_chat_persists_and_view_omits_preimage() {
        let path = temp_path();
        let store = OrderStore::open(path.clone()).unwrap();
        store.create("1").unwrap();
        {
            let mut order = store.guard();
            order.state = OrderState::Disputed;
            order.payment_preimage = Some("0xsecret".into());
        }
        let view = store
            .post_chat(&PostChatBody {
                from: "buyer".into(),
                text: "where is my fiat?".into(),
            })
            .unwrap();
        assert_eq!(view.chat.len(), 1);
        assert_eq!(view.chat[0].from, "buyer");
        assert_eq!(view.chat[0].text, "where is my fiat?");
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("0xsecret"));
        assert!(json.contains("where is my fiat?"));

        let seller = store
            .post_chat(&PostChatBody {
                from: "seller".into(),
                text: "sent already".into(),
            })
            .unwrap();
        assert_eq!(seller.chat.len(), 2);

        let reloaded = OrderStore::open(path.clone()).unwrap();
        assert_eq!(reloaded.snapshot().chat.len(), 2);
        assert_eq!(
            reloaded.guard().payment_preimage.as_deref(),
            Some("0xsecret")
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn generate_preimage_hashes_with_sha256() {
        let (preimage, payment_hash) = generate_preimage();
        let raw = hex::decode(preimage.trim_start_matches("0x")).unwrap();
        let expected = format!("0x{}", hex::encode(Sha256::digest(raw)));
        assert_eq!(payment_hash, expected);
    }
}
