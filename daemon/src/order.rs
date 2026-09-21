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
    pub log: Vec<LogLine>,
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
            log: Vec::new(),
        }
    }

    pub fn view(&self) -> OrderView {
        OrderView {
            state: self.state,
            amount: self.amount.clone(),
            payment_hash: self.payment_hash.clone(),
            invoice_address: self.invoice_address.clone(),
            invoice_status: self.invoice_status.clone(),
            log: self.log.clone(),
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
            log: vec![LogLine {
                at: timestamp(),
                text: format!("created order for {amount} CKB"),
            }],
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
                }]),
            )
            .await
            .map_err(OrderError::Fiber)?;
        let address = invoice_address(&created)
            .ok_or_else(|| OrderError::Fiber("new_invoice missing invoice_address".into()))?;
        let status = invoice_status(&created).unwrap_or_else(|| "Open".into());

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
            "hold invoice created H={payment_hash} S sealed in daemon (never sent to app)"
        ));
        order.push_log(format!("invoice address {address}"));
        order.push_log(format!("invoice status {status}"));
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!("hold created H={payment_hash} state=WaitingHold");
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

        let mut order = self.guard();
        if order.state != OrderState::WaitingHold {
            return Err(OrderError::BadState(
                "order changed while locking".into(),
            ));
        }
        order.state = OrderState::Held;
        order.invoice_status = Some(status.clone());
        order.push_log(format!(
            "seller locked: send_payment submitted, get_invoice={status}"
        ));
        order.push_log(format!(
            "H={payment_hash} S still sealed in daemon; Twine spendable balance unchanged until settle"
        ));
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

    /// Stage 3 Path A: pay buyer from Twine, then settle_invoice(H, S).
    /// Accepts FiatSent or Releasing so a stage-2 order can continue.
    pub async fn release(
        &self,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<OrderView, OrderError> {
        let (state, amount, hold_hash, hold_preimage) = {
            let order = self.guard();
            (
                order.state,
                order.amount.clone(),
                order.payment_hash.clone(),
                order.payment_preimage.clone(),
            )
        };
        if state != OrderState::FiatSent && state != OrderState::Releasing {
            return Err(OrderError::BadState(format!(
                "release needs FiatSent or Releasing, got {state:?}"
            )));
        }
        let amount = amount.ok_or_else(|| OrderError::BadState("order has no amount".into()))?;
        let hold_hash =
            hold_hash.ok_or_else(|| OrderError::BadState("missing hold payment hash".into()))?;
        let hold_preimage = hold_preimage
            .ok_or_else(|| OrderError::BadState("missing hold payment preimage".into()))?;
        let shannon = amount_to_shannon(&amount).map_err(OrderError::BadAmount)?;

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

        let sent = rpc
            .call(
                &config.twine_rpc,
                "send_payment",
                json!([{
                    "invoice": buyer_address,
                    "max_fee_amount": "0x5f5e100",
                }]),
            )
            .await
            .map_err(OrderError::Fiber)?;
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
                let mut order = self.guard();
                order.push_log(format!(
                    "pay failed: {err}; settle_invoice not called (hold stays Received)"
                ));
                save(&self.path, &order).map_err(OrderError::Save)?;
                return Err(err);
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

        let settled = rpc
            .call(
                &config.twine_rpc,
                "get_invoice",
                json!([{ "payment_hash": hold_hash }]),
            )
            .await
            .map_err(OrderError::Fiber)?;
        let status = invoice_status(&settled).unwrap_or_else(|| "Paid".into());

        let mut order = self.guard();
        order.state = OrderState::Settled;
        order.invoice_status = Some(status.clone());
        order.push_log(format!(
            "settle: settle_invoice(H, S) on twine; hold invoice={status}"
        ));
        order.push_log("path A complete: state Settled");
        save(&self.path, &order).map_err(OrderError::Save)?;
        eprintln!("path A settled H={hold_hash} invoice={status} state=Settled");
        Ok(order.view())
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, Order> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
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
            "twine-order-{}-{}",
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
    fn generate_preimage_hashes_with_sha256() {
        let (preimage, payment_hash) = generate_preimage();
        let raw = hex::decode(preimage.trim_start_matches("0x")).unwrap();
        let expected = format!("0x{}", hex::encode(Sha256::digest(raw)));
        assert_eq!(payment_hash, expected);
    }
}
