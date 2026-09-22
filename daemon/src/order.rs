use std::time::Duration;

use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofMeta {
    pub content_type: String,
    pub bytes: usize,
}

/// Persisted trade. `payment_preimage` stays on disk / in the daemon only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trade {
    pub id: String,
    pub ad_id: String,
    #[serde(alias = "seller_pubkey")]
    pub pubkey: String,
    #[serde(alias = "buyer_pubkey")]
    pub taker: String,
    #[serde(alias = "fiat")]
    pub currency: String,
    #[serde(alias = "rate")]
    pub price: String,
    #[serde(alias = "fiat_amount")]
    pub pay_amount: String,
    pub amount: String,
    pub payment_method: String,
    pub state: OrderState,
    #[serde(default)]
    pub payment_hash: Option<String>,
    #[serde(default)]
    pub payment_preimage: Option<String>,
    #[serde(default)]
    pub invoice_address: Option<String>,
    #[serde(default)]
    pub invoice_status: Option<String>,
    #[serde(default)]
    pub buyer_invoice: Option<String>,
    #[serde(default)]
    pub proof: Option<ProofMeta>,
    #[serde(default)]
    pub dispute_from: Option<String>,
    #[serde(default)]
    pub dispute_reason: Option<String>,
    pub log: Vec<LogLine>,
    #[serde(default)]
    pub chat: Vec<ChatLine>,
}

/// Public trade view. Never includes `payment_preimage`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TradeView {
    pub id: String,
    pub ad_id: String,
    pub pubkey: String,
    pub taker: String,
    pub currency: String,
    pub price: String,
    pub pay_amount: String,
    pub amount: String,
    pub payment_method: String,
    pub state: OrderState,
    pub payment_hash: Option<String>,
    pub invoice_address: Option<String>,
    pub invoice_status: Option<String>,
    pub buyer_invoice: Option<String>,
    pub proof: Option<ProofMeta>,
    pub dispute_from: Option<String>,
    pub dispute_reason: Option<String>,
    pub log: Vec<LogLine>,
    pub chat: Vec<ChatLine>,
}

impl Trade {
    /// States where a Received hold may still expire on Fiber (Path D).
    pub fn watches_hold_invoice(state: OrderState) -> bool {
        matches!(
            state,
            OrderState::Held
                | OrderState::WaitingHold
                | OrderState::WaitingFiat
                | OrderState::FiatSent
                | OrderState::Leg2Failed
                | OrderState::Disputed
                | OrderState::Releasing
        )
    }

    pub fn view(&self) -> TradeView {
        TradeView {
            id: self.id.clone(),
            ad_id: self.ad_id.clone(),
            pubkey: self.pubkey.clone(),
            taker: self.taker.clone(),
            currency: self.currency.clone(),
            price: self.price.clone(),
            pay_amount: self.pay_amount.clone(),
            amount: self.amount.clone(),
            payment_method: self.payment_method.clone(),
            state: self.state,
            payment_hash: self.payment_hash.clone(),
            invoice_address: self.invoice_address.clone(),
            invoice_status: self.invoice_status.clone(),
            buyer_invoice: self.buyer_invoice.clone(),
            proof: self.proof.clone(),
            dispute_from: self.dispute_from.clone(),
            dispute_reason: self.dispute_reason.clone(),
            log: self.log.clone(),
            chat: self.chat.clone(),
        }
    }

    pub fn is_open(&self) -> bool {
        !matches!(
            self.state,
            OrderState::Idle
                | OrderState::Cancelled
                | OrderState::Paid
                | OrderState::Settled
                | OrderState::Expired
        )
    }

    pub fn push_log(&mut self, text: impl Into<String>) {
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

#[derive(Debug, Deserialize)]
pub struct FiatSentBody {
    pub invoice: String,
    pub proof_b64: String,
    pub content_type: String,
}

#[derive(Debug, Deserialize)]
pub struct OpenDisputeBody {
    pub from: String,
    pub reason: String,
}

/// JPEG or PNG, 1.5 MB max.
pub const MAX_PROOF_BYTES: usize = 1_572_864;

pub fn decode_payment_proof(
    proof_b64: &str,
    content_type: &str,
) -> Result<(String, Vec<u8>), OrderError> {
    let raw = proof_b64.trim();
    if raw.is_empty() {
        return Err(OrderError::BadState("proof_b64 is required".into()));
    }
    let compact: String = raw.chars().filter(|ch| !ch.is_whitespace()).collect();
    let data = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &compact)
        .map_err(|_| OrderError::BadState("proof_b64 is not valid base64".into()))?;
    if data.len() > MAX_PROOF_BYTES {
        return Err(OrderError::BadState("proof must be at most 1.5 MB".into()));
    }
    let detected = detect_image(&data)?;
    let requested = normalize_content_type(content_type)?;
    if detected != requested {
        return Err(OrderError::BadState(format!(
            "proof content_type {content_type} does not match file"
        )));
    }
    Ok((detected, data))
}

fn detect_image(data: &[u8]) -> Result<String, OrderError> {
    if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return Ok("image/jpeg".into());
    }
    if data.len() >= 8
        && data[..8] == [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
    {
        return Ok("image/png".into());
    }
    Err(OrderError::BadState(
        "proof must be a JPEG or PNG".into(),
    ))
}

fn normalize_content_type(raw: &str) -> Result<String, OrderError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => Ok("image/jpeg".into()),
        "image/png" => Ok("image/png".into()),
        "" => Err(OrderError::BadState("content_type is required".into())),
        _ => Err(OrderError::BadState("proof must be a JPEG or PNG".into())),
    }
}

pub fn party_from(raw: &str) -> Result<String, OrderError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "lister" | "seller" => Ok("lister".into()),
        "taker" | "buyer" => Ok("taker".into()),
        _ => Err(OrderError::BadState(
            "from must be lister or taker".into(),
        )),
    }
}

pub fn allows_chat(state: OrderState) -> bool {
    matches!(
        state,
        OrderState::WaitingFiat
            | OrderState::FiatSent
            | OrderState::Leg2Failed
            | OrderState::Disputed
    )
}

/// How Path A payment failure updates trade state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PayFailureMode {
    /// Stage 3/4: move to Leg2Failed.
    Leg2Failed,
    /// Stage 5 Path C buyer-wins: stay Disputed, do not settle.
    StayDisputed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OrderError {
    BadAmount(String),
    AlreadyOpen,
    NotFound(String),
    BadState(String),
    Fiber(String),
    Save(String),
}

#[derive(Debug, Deserialize)]
pub struct CreateAdBody {
    #[serde(alias = "seller_pubkey")]
    pub pubkey: String,
    #[serde(alias = "available_ckb")]
    pub available: String,
    #[serde(alias = "fiat")]
    pub currency: Option<String>,
    #[serde(alias = "rate")]
    pub price: String,
    #[serde(alias = "min_fiat")]
    pub min: String,
    #[serde(alias = "max_fiat")]
    pub max: String,
    pub payment_method: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTradeBody {
    pub ad_id: String,
    #[serde(alias = "buyer_pubkey")]
    pub taker: String,
    #[serde(alias = "fiat_amount")]
    pub pay_amount: String,
}

#[derive(Debug, Deserialize)]
pub struct ConnectBody {
    pub pubkey: String,
    pub address: String,
}

#[derive(Debug, Deserialize)]
pub struct BuyerInvoiceBody {
    pub invoice: String,
}

pub(crate) async fn create_hold_invoice(
    rpc: &FiberRpc,
    twine_rpc: &str,
    amount: &str,
) -> Result<(String, String, String, String), OrderError> {
    let shannon = amount_to_shannon(amount).map_err(OrderError::BadAmount)?;
    let (preimage, payment_hash) = generate_preimage();
    let created = rpc
        .call(
            twine_rpc,
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
    if let Some(attr) = invoice_final_expiry_delta(&created) {
        let expected = hex_u64(FINAL_EXPIRY_DELTA_MS);
        if normalize_hex(&attr) != normalize_hex(&expected) {
            return Err(OrderError::Fiber(format!(
                "new_invoice final_htlc_minimum_expiry_delta={attr} did not match requested {expected}"
            )));
        }
    }
    Ok((preimage, payment_hash, address, status))
}

pub(crate) async fn demo_cancel_invoice(
    rpc: &FiberRpc,
    twine_rpc: &str,
    amount: &str,
) -> Result<(String, String, String), OrderError> {
    let shannon = amount_to_shannon(amount).map_err(OrderError::BadAmount)?;
    let (_s, payment_hash) = generate_preimage();
    let created = rpc
        .call(
            twine_rpc,
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
            twine_rpc,
            "cancel_invoice",
            json!([{ "payment_hash": payment_hash }]),
        )
        .await
        .map_err(OrderError::Fiber)?;
    let status = invoice_status(&cancelled).unwrap_or_else(|| "Cancelled".into());
    Ok((payment_hash, address, status))
}

pub(crate) async fn fetch_invoice_status(
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

pub(crate) fn require_received_for_dispute(status: &str) -> Result<(), OrderError> {
    if status == "Received" {
        return Ok(());
    }
    Err(OrderError::BadState(format!(
        "open dispute needs hold invoice Received, got {status}"
    )))
}

pub(crate) fn require_received_for_award(status: &str) -> Result<(), OrderError> {
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

pub(crate) async fn poll_invoice_received(
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

pub(crate) async fn poll_payment_done(
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

pub(crate) async fn poll_invoice_paid(
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

pub(crate) async fn attempt_settle_after_expiry(
    rpc: &FiberRpc,
    twine_rpc: &str,
    payment_hash: &str,
    preimage: Option<&str>,
) -> Option<String> {
    match preimage {
        Some(preimage) => match rpc
            .call(
                twine_rpc,
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
    }
}

pub(crate) async fn send_payment_to_invoice(
    rpc: &FiberRpc,
    twine_rpc: &str,
    invoice: &str,
) -> Result<Value, String> {
    rpc.call(
        twine_rpc,
        "send_payment",
        json!([{
            "invoice": invoice,
            "max_fee_amount": "0x5f5e100",
        }]),
    )
    .await
}

pub(crate) async fn settle_hold(
    rpc: &FiberRpc,
    twine_rpc: &str,
    payment_hash: &str,
    preimage: &str,
) -> Result<Value, String> {
    rpc.call(
        twine_rpc,
        "settle_invoice",
        json!([{
            "payment_hash": payment_hash,
            "payment_preimage": preimage,
        }]),
    )
    .await
}

pub(crate) async fn cancel_hold_invoice(
    rpc: &FiberRpc,
    twine_rpc: &str,
    payment_hash: &str,
) -> Result<Value, String> {
    rpc.call(
        twine_rpc,
        "cancel_invoice",
        json!([{ "payment_hash": payment_hash }]),
    )
    .await
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

pub(crate) fn payment_hash_of(value: &Value) -> Option<String> {
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

pub fn amount_to_shannon(amount: &str) -> Result<u128, String> {
    parse_decimal_8(amount).and_then(|value| {
        if value == 0 {
            Err("amount must be greater than zero".to_string())
        } else {
            Ok(value)
        }
    })
}

pub fn parse_decimal_8(raw: &str) -> Result<u128, String> {
    let normalized = normalize_decimal(raw)?;
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

pub fn format_decimal_8(scaled: u128) -> String {
    let whole = scaled / SHANNONS_PER_CKB;
    let frac = scaled % SHANNONS_PER_CKB;
    if frac == 0 {
        return whole.to_string();
    }
    let frac = format!("{frac:08}");
    let frac = frac.trim_end_matches('0');
    format!("{whole}.{frac}")
}

pub fn ckb_from_fiat(fiat: &str, rate: &str) -> Result<String, String> {
    let fiat = parse_decimal_8(fiat)?;
    let rate = parse_decimal_8(rate)?;
    if rate == 0 {
        return Err("rate must be greater than zero".to_string());
    }
    let ckb = fiat
        .checked_mul(SHANNONS_PER_CKB)
        .and_then(|v| v.checked_div(rate))
        .ok_or_else(|| "amount is too large".to_string())?;
    if ckb == 0 {
        return Err("fiat amount is too small for this rate".to_string());
    }
    Ok(format_decimal_8(ckb))
}

pub fn fiat_from_ckb(ckb: &str, rate: &str) -> Result<String, String> {
    let ckb = parse_decimal_8(ckb)?;
    let rate = parse_decimal_8(rate)?;
    if rate == 0 {
        return Err("rate must be greater than zero".to_string());
    }
    let fiat = ckb
        .checked_mul(rate)
        .and_then(|v| v.checked_div(SHANNONS_PER_CKB))
        .ok_or_else(|| "amount is too large".to_string())?;
    Ok(format_decimal_8(fiat))
}

pub fn add_ckb(left: &str, right: &str) -> Result<String, String> {
    let sum = parse_decimal_8(left)?
        .checked_add(parse_decimal_8(right)?)
        .ok_or_else(|| "amount is too large".to_string())?;
    Ok(format_decimal_8(sum))
}

pub fn cmp_ckb(left: &str, right: &str) -> Result<std::cmp::Ordering, String> {
    Ok(parse_decimal_8(left)?.cmp(&parse_decimal_8(right)?))
}

pub fn normalize_amount(raw: &str) -> Result<String, String> {
    let text = normalize_decimal(raw)?;
    if parse_decimal_8(&text)? == 0 {
        return Err("amount must be greater than zero".to_string());
    }
    Ok(text)
}

pub fn normalize_decimal(raw: &str) -> Result<String, String> {
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
        return Err("amount must be a positive number".to_string());
    }
    if frac.len() > 8 {
        return Err("amount supports at most 8 decimal places".to_string());
    }
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        Ok(whole.to_string())
    } else {
        Ok(format!("{whole}.{frac}"))
    }
}

pub fn normalize_pubkey(pubkey: &str) -> String {
    pubkey.trim().trim_start_matches("0x").to_ascii_lowercase()
}

pub fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn new_id() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn fiat_over_rate_is_integer_shannon_ckb() {
        assert_eq!(ckb_from_fiat("2000", "2000").as_deref(), Ok("1"));
        assert_eq!(ckb_from_fiat("1000", "2000").as_deref(), Ok("0.5"));
        assert_eq!(fiat_from_ckb("1", "2000").as_deref(), Ok("2000"));
        assert_eq!(fiat_from_ckb("0.5", "2000").as_deref(), Ok("1000"));
        assert!(ckb_from_fiat("1", "200000000000").is_err());
        assert!(ckb_from_fiat("10", "0").is_err());
    }

    #[test]
    fn generate_preimage_hashes_with_sha256() {
        let (preimage, payment_hash) = generate_preimage();
        let raw = hex::decode(preimage.trim_start_matches("0x")).unwrap();
        let expected = format!("0x{}", hex::encode(Sha256::digest(raw)));
        assert_eq!(payment_hash, expected);
    }

    #[test]
    fn decode_payment_proof_accepts_jpeg_and_png() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xD9];
        let png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let jpeg_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, jpeg);
        let png_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png);
        let (kind, bytes) = decode_payment_proof(&jpeg_b64, "image/jpg").unwrap();
        assert_eq!(kind, "image/jpeg");
        assert_eq!(bytes, jpeg);
        let (kind, bytes) = decode_payment_proof(&png_b64, "image/png").unwrap();
        assert_eq!(kind, "image/png");
        assert_eq!(bytes, png);
        assert!(decode_payment_proof("", "image/jpeg").is_err());
        assert!(decode_payment_proof(&jpeg_b64, "image/png").is_err());
        assert!(decode_payment_proof("@@@", "image/jpeg").is_err());
    }
}

#[cfg(test)]
mod poc_test;
