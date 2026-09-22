use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::health::Config;
use crate::order::{
    add_ckb, allows_chat, attempt_settle_after_expiry, cancel_hold_invoice, ckb_from_fiat,
    cmp_ckb, create_hold_invoice, decode_payment_proof, demo_cancel_invoice,
    fetch_invoice_status, fiat_from_ckb, new_id, normalize_amount, normalize_pubkey,
    party_from, payment_hash_of, poll_invoice_paid, poll_invoice_received, poll_payment_done,
    require_received_for_award, require_received_for_dispute, send_payment_to_invoice,
    settle_hold, timestamp, BuyerInvoiceBody, CreateAdBody, CreateTradeBody, FiatSentBody,
    OpenDisputeBody, OrderError, OrderState, PayFailureMode, PostChatBody, ProofMeta, Trade,
    TradeView, FINAL_EXPIRY_DELTA_MS,
};
use crate::rpc::FiberRpc;

#[derive(Clone, Debug)]
pub struct MarketStore {
    path: PathBuf,
    inner: Arc<Mutex<Market>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Market {
    #[serde(default)]
    pub ads: HashMap<String, Ad>,
    #[serde(default)]
    pub trades: HashMap<String, Trade>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ad {
    pub id: String,
    #[serde(alias = "seller_pubkey")]
    pub pubkey: String,
    #[serde(alias = "available_ckb")]
    pub available: String,
    #[serde(alias = "fiat")]
    pub currency: String,
    #[serde(alias = "rate")]
    pub price: String,
    #[serde(default, alias = "min_fiat")]
    pub min: String,
    #[serde(default, alias = "max_fiat")]
    pub max: String,
    pub payment_method: String,
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default)]
    pub open_trade_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AdView {
    pub id: String,
    pub pubkey: String,
    pub available: String,
    pub currency: String,
    pub price: String,
    pub min: String,
    pub max: String,
    pub payment_method: String,
    pub open_trade_id: Option<String>,
}

impl Ad {
    pub fn view(&self) -> AdView {
        AdView {
            id: self.id.clone(),
            pubkey: self.pubkey.clone(),
            available: self.available.clone(),
            currency: self.currency.clone(),
            price: self.price.clone(),
            min: self.min.clone(),
            max: self.max.clone(),
            payment_method: self.payment_method.clone(),
            open_trade_id: self.open_trade_id.clone(),
        }
    }
}

impl MarketStore {
    pub fn from_env() -> Result<Self, String> {
        let path = match std::env::var("MARKET_FILE") {
            Ok(value) if !value.is_empty() => PathBuf::from(value),
            _ => match std::env::var("ORDER_FILE") {
                Ok(value) if !value.is_empty() => {
                    let order = PathBuf::from(value);
                    match order.parent() {
                        Some(parent) if !parent.as_os_str().is_empty() => parent.join("market.json"),
                        _ => PathBuf::from("market.json"),
                    }
                }
                _ => PathBuf::from("market.json"),
            },
        };
        Self::open(path)
    }

    pub fn open(path: PathBuf) -> Result<Self, String> {
        let market = if path.exists() {
            let text = fs::read_to_string(&path).map_err(|err| err.to_string())?;
            serde_json::from_str(&text)
                .map_err(|err| format!("market file {}: {err}", path.display()))?
        } else {
            Market::default()
        };
        Ok(Self {
            path,
            inner: Arc::new(Mutex::new(market)),
        })
    }

    pub fn list_ads(&self) -> Vec<AdView> {
        let market = self.guard();
        let mut ads: Vec<AdView> = market
            .ads
            .values()
            .filter(|ad| ad_is_shoppable(&market, ad))
            .map(Ad::view)
            .collect();
        ads.sort_by(|left, right| left.id.cmp(&right.id));
        ads
    }

    pub fn create_ad(&self, body: &CreateAdBody) -> Result<AdView, OrderError> {
        let pubkey = require_text(&body.pubkey, "pubkey")?;
        let available = normalize_amount(&body.available).map_err(OrderError::BadAmount)?;
        let price = normalize_amount(&body.price).map_err(OrderError::BadAmount)?;
        let min = normalize_amount(&body.min).map_err(OrderError::BadAmount)?;
        let max = normalize_amount(&body.max).map_err(OrderError::BadAmount)?;
        let payment_method = require_text(&body.payment_method, "payment_method")?;
        let currency = body
            .currency
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("NGN")
            .to_string();
        if cmp_ckb(&min, &max).map_err(OrderError::BadAmount)? == std::cmp::Ordering::Greater {
            return Err(OrderError::BadAmount("min cannot exceed max".into()));
        }
        let min_ckb = ckb_from_fiat(&min, &price).map_err(OrderError::BadAmount)?;
        let max_ckb = ckb_from_fiat(&max, &price).map_err(OrderError::BadAmount)?;
        if cmp_ckb(&min_ckb, &available).map_err(OrderError::BadAmount)?
            == std::cmp::Ordering::Greater
        {
            return Err(OrderError::BadAmount(
                "available is too small for the minimum take".into(),
            ));
        }
        if cmp_ckb(&max_ckb, &available).map_err(OrderError::BadAmount)?
            == std::cmp::Ordering::Greater
        {
            return Err(OrderError::BadAmount(
                "max take is larger than available".into(),
            ));
        }

        let id = new_id();
        let ad = Ad {
            id: id.clone(),
            pubkey,
            available,
            currency,
            price,
            min,
            max,
            payment_method,
            cancelled: false,
            open_trade_id: None,
        };
        let mut market = self.guard();
        market.ads.insert(id, ad.clone());
        save(&self.path, &market).map_err(OrderError::Save)?;
        Ok(ad.view())
    }

    pub fn cancel_ad(&self, id: &str) -> Result<AdView, OrderError> {
        let mut market = self.guard();
        if !market.ads.contains_key(id) {
            return Err(OrderError::NotFound(format!("ad {id} not found")));
        }
        if ad_has_open_trade(
            &market,
            market.ads.get(id).expect("ad exists"),
        ) {
            return Err(OrderError::AlreadyOpen);
        }
        let ad = market.ads.get_mut(id).expect("ad exists");
        ad.cancelled = true;
        ad.open_trade_id = None;
        let view = ad.view();
        save(&self.path, &market).map_err(OrderError::Save)?;
        Ok(view)
    }

    pub fn list_trades(&self, pubkey: Option<&str>) -> Vec<TradeView> {
        let market = self.guard();
        let want = pubkey.map(normalize_pubkey).filter(|value| !value.is_empty());
        let mut trades: Vec<TradeView> = market
            .trades
            .values()
            .filter(|trade| match &want {
                Some(pubkey) => {
                    normalize_pubkey(&trade.pubkey) == *pubkey
                        || normalize_pubkey(&trade.taker) == *pubkey
                }
                None => true,
            })
            .map(Trade::view)
            .collect();
        trades.sort_by(|left, right| left.id.cmp(&right.id));
        trades
    }

    pub fn get_trade(&self, id: &str) -> Result<TradeView, OrderError> {
        self.guard()
            .trades
            .get(id)
            .map(Trade::view)
            .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))
    }

    pub fn persisted_preimage(&self, trade_id: &str) -> Option<String> {
        self.guard()
            .trades
            .get(trade_id)
            .and_then(|trade| trade.payment_preimage.clone())
    }

    fn proofs_dir(&self) -> PathBuf {
        match self.path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.join("proofs"),
            _ => PathBuf::from("proofs"),
        }
    }

    fn proof_path(&self, trade_id: &str) -> PathBuf {
        self.proofs_dir().join(trade_id)
    }

    fn write_proof(&self, trade_id: &str, bytes: &[u8]) -> Result<(), OrderError> {
        fs::create_dir_all(self.proofs_dir()).map_err(|err| OrderError::Save(err.to_string()))?;
        fs::write(self.proof_path(trade_id), bytes).map_err(|err| OrderError::Save(err.to_string()))
    }

    pub async fn create_trade(
        &self,
        rpc: &FiberRpc,
        config: &Config,
        body: &CreateTradeBody,
    ) -> Result<TradeView, OrderError> {
        let taker = require_text(&body.taker, "taker")?;
        let pay_amount = normalize_amount(&body.pay_amount).map_err(OrderError::BadAmount)?;
        let trade_id = new_id();

        let (ad_id, amount, listed_by, currency, price, payment_method) = {
            let mut market = self.guard();
            let ad = market
                .ads
                .get(&body.ad_id)
                .cloned()
                .ok_or_else(|| OrderError::NotFound(format!("ad {} not found", body.ad_id)))?;
            if ad.cancelled {
                return Err(OrderError::BadState("ad is cancelled".into()));
            }
            if ad_has_open_trade(&market, &ad) {
                return Err(OrderError::AlreadyOpen);
            }
            if normalize_pubkey(&ad.pubkey) == normalize_pubkey(&taker) {
                return Err(OrderError::BadState(
                    "taker must be a different user".into(),
                ));
            }
            if !ad.min.is_empty()
                && cmp_ckb(&pay_amount, &ad.min).map_err(OrderError::BadAmount)?
                    == std::cmp::Ordering::Less
            {
                return Err(OrderError::BadAmount(format!(
                    "take must be at least {} {}",
                    ad.min, ad.currency
                )));
            }
            if !ad.max.is_empty()
                && cmp_ckb(&pay_amount, &ad.max).map_err(OrderError::BadAmount)?
                    == std::cmp::Ordering::Greater
            {
                return Err(OrderError::BadAmount(format!(
                    "take must be at most {} {}",
                    ad.max, ad.currency
                )));
            }
            let available_pay =
                fiat_from_ckb(&ad.available, &ad.price).map_err(OrderError::BadAmount)?;
            if cmp_ckb(&pay_amount, &available_pay).map_err(OrderError::BadAmount)?
                == std::cmp::Ordering::Greater
            {
                return Err(OrderError::BadAmount(
                    "not enough CKB available on this ad".into(),
                ));
            }
            let amount = ckb_from_fiat(&pay_amount, &ad.price).map_err(OrderError::BadAmount)?;
            if cmp_ckb(&amount, &ad.available).map_err(OrderError::BadAmount)?
                == std::cmp::Ordering::Greater
            {
                return Err(OrderError::BadAmount(
                    "not enough CKB available on this ad".into(),
                ));
            }
            let remaining =
                subtract_ckb(&ad.available, &amount).map_err(OrderError::BadAmount)?;
            let trade = Trade {
                id: trade_id.clone(),
                ad_id: ad.id.clone(),
                pubkey: ad.pubkey.clone(),
                taker: taker.clone(),
                currency: ad.currency.clone(),
                price: ad.price.clone(),
                pay_amount: pay_amount.clone(),
                amount: amount.clone(),
                payment_method: ad.payment_method.clone(),
                state: OrderState::Pending,
                payment_hash: None,
                payment_preimage: None,
                invoice_address: None,
                invoice_status: None,
                buyer_invoice: None,
                proof: None,
                dispute_from: None,
                dispute_reason: None,
                log: vec![],
                chat: vec![],
            };
            {
                let listed = market.ads.get_mut(&body.ad_id).expect("ad exists");
                listed.available = remaining;
                listed.open_trade_id = Some(trade_id.clone());
            }
            market.trades.insert(trade_id.clone(), trade);
            save(&self.path, &market).map_err(OrderError::Save)?;
            (
                body.ad_id.clone(),
                amount,
                ad.pubkey,
                ad.currency,
                ad.price,
                ad.payment_method,
            )
        };

        let hold = create_hold_invoice(rpc, &config.twine_rpc, &amount).await;
        let (preimage, payment_hash, address, status) = match hold {
            Ok(created) => created,
            Err(err) => {
                self.rollback_trade(&trade_id, &ad_id, &amount)?;
                return Err(err);
            }
        };

        let mut market = self.guard();
        let Some(trade) = market.trades.get_mut(&trade_id) else {
            return Err(OrderError::NotFound(format!("trade {trade_id} not found")));
        };
        trade.state = OrderState::WaitingHold;
        trade.payment_hash = Some(payment_hash.clone());
        trade.payment_preimage = Some(preimage);
        trade.invoice_address = Some(address.clone());
        trade.invoice_status = Some(status.clone());
        trade.push_log(format!(
            "created trade for {amount} CKB ({pay_amount} {currency} at {price} {currency}/CKB); listed by {listed_by}"
        ));
        trade.push_log(format!(
            "hold invoice created H={payment_hash} final_expiry_delta={FINAL_EXPIRY_DELTA_MS}ms ({}) S sealed in daemon (never sent to app)",
            format!("0x{FINAL_EXPIRY_DELTA_MS:x}")
        ));
        trade.push_log(format!("invoice address {address}"));
        trade.push_log(format!("invoice status {status}"));
        let _ = (listed_by, payment_method);
        let view = trade.view();
        save(&self.path, &market).map_err(OrderError::Save)?;
        eprintln!(
            "trade {trade_id} hold created H={payment_hash} state=WaitingHold S sealed"
        );
        Ok(view)
    }

    pub async fn demo_cancel(
        &self,
        id: &str,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<TradeView, OrderError> {
        let amount = self
            .guard()
            .trades
            .get(id)
            .map(|trade| trade.amount.clone())
            .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
        let (payment_hash, address, status) =
            demo_cancel_invoice(rpc, &config.twine_rpc, &amount).await?;
        let mut market = self.guard();
        let trade = market
            .trades
            .get_mut(id)
            .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
        let live_state = trade.state;
        trade.push_log(format!(
            "demo cancel: unpaid invoice {status} H={payment_hash} (live order unchanged, still {live_state:?})"
        ));
        if !address.is_empty() {
            trade.push_log(format!("demo cancel invoice was {address}"));
        }
        let view = trade.view();
        save(&self.path, &market).map_err(OrderError::Save)?;
        eprintln!("demo cancel H={payment_hash} status={status}");
        Ok(view)
    }

    /// Seller already paid the hold on their own node. Poll Twine until Received,
    /// then start the fiat window.
    pub async fn mark_locked(
        &self,
        id: &str,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<TradeView, OrderError> {
        let (state, payment_hash) = {
            let market = self.guard();
            let trade = market
                .trades
                .get(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            (trade.state, trade.payment_hash.clone())
        };
        if state != OrderState::WaitingHold {
            return Err(OrderError::BadState(format!(
                "lock needs WaitingHold, got {state:?}"
            )));
        }
        let payment_hash =
            payment_hash.ok_or_else(|| OrderError::BadState("missing payment hash".into()))?;
        let status = poll_invoice_received(rpc, &config.twine_rpc, &payment_hash).await?;

        let mut market = self.guard();
        let trade = market
            .trades
            .get_mut(id)
            .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
        if trade.state != OrderState::WaitingHold {
            return Err(OrderError::BadState("trade changed while locking".into()));
        }
        trade.state = OrderState::WaitingFiat;
        trade.invoice_status = Some(status.clone());
        trade.push_log(format!(
            "seller locked: send_payment submitted, get_invoice={status}"
        ));
        trade.push_log(format!(
            "H={payment_hash} S still sealed in daemon; Twine spendable balance unchanged until settle"
        ));
        trade.push_log("buyer accepted; start fiat timer in the app");
        let view = trade.view();
        save(&self.path, &market).map_err(OrderError::Save)?;
        eprintln!("locked H={payment_hash} invoice={status} state=WaitingFiat");
        Ok(view)
    }

    pub async fn try_cancel(
        &self,
        id: &str,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<TradeView, OrderError> {
        let (state, payment_hash, amount, ad_id) = {
            let market = self.guard();
            let trade = market
                .trades
                .get(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            (
                trade.state,
                trade.payment_hash.clone(),
                trade.amount.clone(),
                trade.ad_id.clone(),
            )
        };
        let payment_hash =
            payment_hash.ok_or_else(|| OrderError::BadState("no hold invoice yet".into()))?;
        let status = fetch_invoice_status(rpc, &config.twine_rpc, &payment_hash).await?;

        if status != "Open" {
            let mut market = self.guard();
            let trade = market
                .trades
                .get_mut(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            trade.invoice_status = Some(status.clone());
            trade.push_log(format!(
                "cancel_invoice not applied (state={state:?}, invoice={status}): only legal while Open; after Received the seller refund is TLC expiry, not cancel"
            ));
            let view = trade.view();
            save(&self.path, &market).map_err(OrderError::Save)?;
            return Ok(view);
        }

        match cancel_hold_invoice(rpc, &config.twine_rpc, &payment_hash).await {
            Ok(result) => {
                let status = result
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Cancelled")
                    .to_string();
                let mut market = self.guard();
                let view = {
                    let trade = market
                        .trades
                        .get_mut(id)
                        .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
                    trade.state = OrderState::Cancelled;
                    trade.invoice_status = Some(status.clone());
                    trade.push_log(format!(
                        "cancel_invoice succeeded: invoice {status} H={payment_hash}"
                    ));
                    trade.view()
                };
                return_ckb(&mut market, &ad_id, id, &amount)?;
                save(&self.path, &market).map_err(OrderError::Save)?;
                Ok(view)
            }
            Err(err) => {
                let mut market = self.guard();
                let trade = market
                    .trades
                    .get_mut(id)
                    .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
                trade.push_log(format!(
                    "cancel_invoice refused (state={state:?}, invoice={status}): {err}"
                ));
                let view = trade.view();
                save(&self.path, &market).map_err(OrderError::Save)?;
                Ok(view)
            }
        }
    }

    pub fn fiat_sent(&self, id: &str, body: &FiatSentBody) -> Result<TradeView, OrderError> {
        let invoice = require_text(&body.invoice, "invoice")?;
        let (content_type, data) = decode_payment_proof(&body.proof_b64, &body.content_type)?;
        {
            let market = self.guard();
            let trade = market
                .trades
                .get(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            if trade.state != OrderState::WaitingFiat {
                return Err(OrderError::BadState(format!(
                    "fiat sent needs WaitingFiat, got {:?}",
                    trade.state
                )));
            }
        }
        self.write_proof(id, &data)?;
        let mut market = self.guard();
        let trade = market
            .trades
            .get_mut(id)
            .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
        if trade.state != OrderState::WaitingFiat {
            return Err(OrderError::BadState(format!(
                "trade changed while storing proof, got {:?}",
                trade.state
            )));
        }
        trade.buyer_invoice = Some(invoice);
        trade.proof = Some(ProofMeta {
            content_type,
            bytes: data.len(),
        });
        trade.state = OrderState::FiatSent;
        trade.push_log("buyer marked fiat sent with payment proof");
        let view = trade.view();
        save(&self.path, &market).map_err(OrderError::Save)?;
        eprintln!("trade {id} state=FiatSent");
        Ok(view)
    }

    pub fn get_proof(&self, id: &str) -> Result<(String, Vec<u8>), OrderError> {
        let content_type = {
            let market = self.guard();
            let trade = market
                .trades
                .get(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            trade
                .proof
                .as_ref()
                .map(|proof| proof.content_type.clone())
                .ok_or_else(|| OrderError::NotFound(format!("proof for trade {id} not found")))?
        };
        let bytes = fs::read(self.proof_path(id)).map_err(|_| {
            OrderError::NotFound(format!("proof for trade {id} not found"))
        })?;
        Ok((content_type, bytes))
    }

    pub async fn release(
        &self,
        id: &str,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<TradeView, OrderError> {
        let state = self.trade_state(id)?;
        if state != OrderState::FiatSent && state != OrderState::Releasing {
            return Err(OrderError::BadState(format!(
                "release needs FiatSent or Releasing, got {state:?}"
            )));
        }
        {
            let mut market = self.guard();
            let trade = market
                .trades
                .get_mut(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            if trade.state == OrderState::FiatSent {
                trade.state = OrderState::Releasing;
                trade.push_log("seller released; path A: pay buyer then settle hold");
            } else {
                trade.push_log("path A continue from Releasing: pay buyer then settle hold");
            }
            save(&self.path, &market).map_err(OrderError::Save)?;
        }
        self.pay_buyer_then_settle(id, rpc, config, PayFailureMode::Leg2Failed)
            .await
    }

    pub async fn retry(
        &self,
        id: &str,
        rpc: &FiberRpc,
        config: &Config,
        body: &BuyerInvoiceBody,
    ) -> Result<TradeView, OrderError> {
        let invoice = require_text(&body.invoice, "invoice")?;
        let state = self.trade_state(id)?;
        if state != OrderState::Leg2Failed {
            return Err(OrderError::BadState(format!(
                "retry needs Leg2Failed, got {state:?}"
            )));
        }
        {
            let mut market = self.guard();
            let trade = market
                .trades
                .get_mut(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            trade.buyer_invoice = Some(invoice);
            trade.state = OrderState::Releasing;
            trade.push_log(
                "retry: buyer submitted a new invoice path; path A: pay buyer then settle hold",
            );
            save(&self.path, &market).map_err(OrderError::Save)?;
        }
        self.pay_buyer_then_settle(id, rpc, config, PayFailureMode::Leg2Failed)
            .await
    }

    pub async fn open_dispute(
        &self,
        id: &str,
        rpc: &FiberRpc,
        config: &Config,
        body: &OpenDisputeBody,
    ) -> Result<TradeView, OrderError> {
        let from = party_from(&body.from)?;
        let reason = require_text(&body.reason, "reason")?;
        if reason.len() > 500 {
            return Err(OrderError::BadState(
                "reason must be at most 500 characters".into(),
            ));
        }
        let (state, payment_hash) = {
            let market = self.guard();
            let trade = market
                .trades
                .get(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            (trade.state, trade.payment_hash.clone())
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

        let mut market = self.guard();
        let trade = market
            .trades
            .get_mut(id)
            .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
        match trade.state {
            OrderState::WaitingFiat | OrderState::FiatSent | OrderState::Leg2Failed => {}
            other => {
                return Err(OrderError::BadState(format!(
                    "trade changed while opening dispute, got {other:?}"
                )));
            }
        }
        trade.state = OrderState::Disputed;
        trade.dispute_from = Some(from.clone());
        trade.dispute_reason = Some(reason.clone());
        trade.invoice_status = Some(status.clone());
        trade.push_log(format!(
            "dispute opened by {from}: {reason} (invoice still {status}); Twine may award only after this appeal"
        ));
        let view = trade.view();
        save(&self.path, &market).map_err(OrderError::Save)?;
        eprintln!("trade {id} state=Disputed H={payment_hash}");
        Ok(view)
    }

    pub fn post_chat(&self, id: &str, body: &PostChatBody) -> Result<TradeView, OrderError> {
        let from = party_from(&body.from)?;
        let text = body.text.trim();
        if text.is_empty() {
            return Err(OrderError::BadState("chat text is required".into()));
        }
        if text.len() > 500 {
            return Err(OrderError::BadState(
                "chat text must be at most 500 characters".into(),
            ));
        }
        let mut market = self.guard();
        let trade = market
            .trades
            .get_mut(id)
            .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
        if !allows_chat(trade.state) {
            return Err(OrderError::BadState(format!(
                "post chat needs an open trade, got {:?}",
                trade.state
            )));
        }
        trade.chat.push(crate::order::ChatLine {
            at: timestamp(),
            from: from.clone(),
            text: text.to_string(),
        });
        trade.push_log(format!("chat ({from}): {text}"));
        let view = trade.view();
        save(&self.path, &market).map_err(OrderError::Save)?;
        Ok(view)
    }

    pub async fn award_buyer(
        &self,
        id: &str,
        rpc: &FiberRpc,
        config: &Config,
        body: &BuyerInvoiceBody,
    ) -> Result<TradeView, OrderError> {
        let invoice = require_text(&body.invoice, "invoice")?;
        let (state, payment_hash) = {
            let market = self.guard();
            let trade = market
                .trades
                .get(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            (trade.state, trade.payment_hash.clone())
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
            let mut market = self.guard();
            let trade = market
                .trades
                .get_mut(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            if trade.state != OrderState::Disputed {
                return Err(OrderError::BadState(
                    "trade changed while awarding buyer".into(),
                ));
            }
            trade.buyer_invoice = Some(invoice);
            trade.invoice_status = Some(status);
            trade.push_log("solver awarded buyer; path C → path A: pay buyer then settle hold");
            save(&self.path, &market).map_err(OrderError::Save)?;
        }
        self.pay_buyer_then_settle(id, rpc, config, PayFailureMode::StayDisputed)
            .await
    }

    pub async fn award_seller(
        &self,
        id: &str,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<TradeView, OrderError> {
        let (state, payment_hash) = {
            let market = self.guard();
            let trade = market
                .trades
                .get(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            (trade.state, trade.payment_hash.clone())
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

        let mut market = self.guard();
        let trade = market
            .trades
            .get_mut(id)
            .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
        if trade.state != OrderState::Disputed {
            return Err(OrderError::BadState(
                "trade changed while awarding seller".into(),
            ));
        }
        trade.invoice_status = Some(status.clone());
        trade.push_log(format!(
            "solver awarded seller: hold stays {status}; settle_invoice not called; cancel_invoice not called; seller refund at TLC expiry"
        ));
        let view = trade.view();
        save(&self.path, &market).map_err(OrderError::Save)?;
        eprintln!(
            "path C award seller H={payment_hash} invoice={status} (no settle, no cancel)"
        );
        Ok(view)
    }

    pub async fn poll_all_hold_expiry(
        &self,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<Vec<TradeView>, OrderError> {
        let ids: Vec<String> = {
            let market = self.guard();
            market
                .trades
                .values()
                .filter(|trade| Trade::watches_hold_invoice(trade.state))
                .map(|trade| trade.id.clone())
                .collect()
        };
        let mut expired = Vec::new();
        for id in ids {
            if let Some(view) = self.poll_hold_expiry(&id, rpc, config).await? {
                expired.push(view);
            }
        }
        Ok(expired)
    }

    pub async fn poll_hold_expiry(
        &self,
        id: &str,
        rpc: &FiberRpc,
        config: &Config,
    ) -> Result<Option<TradeView>, OrderError> {
        let (state, payment_hash, already_expired) = {
            let market = self.guard();
            let Some(trade) = market.trades.get(id) else {
                return Ok(None);
            };
            (
                trade.state,
                trade.payment_hash.clone(),
                trade.state == OrderState::Expired,
            )
        };
        if already_expired || !Trade::watches_hold_invoice(state) {
            return Ok(None);
        }
        let payment_hash = match payment_hash {
            Some(hash) => hash,
            None => return Ok(None),
        };
        let status = fetch_invoice_status(rpc, &config.twine_rpc, &payment_hash).await?;
        if status != "Expired" {
            let mut market = self.guard();
            if let Some(trade) = market.trades.get_mut(id) {
                if Trade::watches_hold_invoice(trade.state) {
                    trade.invoice_status = Some(status);
                    save(&self.path, &market).map_err(OrderError::Save)?;
                }
            }
            return Ok(None);
        }
        self.apply_path_d_expiry(id, rpc, config, &payment_hash)
            .await
            .map(Some)
    }

    async fn apply_path_d_expiry(
        &self,
        id: &str,
        rpc: &FiberRpc,
        config: &Config,
        payment_hash: &str,
    ) -> Result<TradeView, OrderError> {
        let (preimage, amount, ad_id) = {
            let market = self.guard();
            let trade = market
                .trades
                .get(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            (
                trade.payment_preimage.clone(),
                trade.amount.clone(),
                trade.ad_id.clone(),
            )
        };
        let settle_err = attempt_settle_after_expiry(
            rpc,
            &config.twine_rpc,
            payment_hash,
            preimage.as_deref(),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        let mut market = self.guard();
        let view = {
            let trade = market
                .trades
                .get_mut(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            if trade.state == OrderState::Expired {
                return Ok(trade.view());
            }
            if !Trade::watches_hold_invoice(trade.state) {
                return Err(OrderError::BadState(format!(
                    "path D expiry interrupted; trade is {:?}",
                    trade.state
                )));
            }
            trade.state = OrderState::Expired;
            trade.invoice_status = Some("Expired".into());
            trade.push_log(format!(
                "path D: hold invoice Expired H={payment_hash}; seller payment failed back; seller refunded because the TLC expired"
            ));
            trade.push_log(
                "path D: cancel_invoice not called (not legal / not needed after Received→Expired)",
            );
            if let Some(err) = settle_err {
                trade.push_log(format!(
                    "path D: settle_invoice(H, S) after expiry failed as expected: {err} (not a successful settle)"
                ));
            }
            trade.view()
        };
        return_ckb(&mut market, &ad_id, id, &amount)?;
        save(&self.path, &market).map_err(OrderError::Save)?;
        eprintln!(
            "path D Expired H={payment_hash} (seller refunded at TLC expiry; settle failed; no cancel)"
        );
        Ok(view)
    }

    async fn pay_buyer_then_settle(
        &self,
        id: &str,
        rpc: &FiberRpc,
        config: &Config,
        on_failure: PayFailureMode,
    ) -> Result<TradeView, OrderError> {
        let (amount, hold_hash, hold_preimage, buyer_invoice) = {
            let market = self.guard();
            let trade = market
                .trades
                .get(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            (
                trade.amount.clone(),
                trade.payment_hash.clone(),
                trade.payment_preimage.clone(),
                trade.buyer_invoice.clone(),
            )
        };
        let _amount = amount;
        let hold_hash =
            hold_hash.ok_or_else(|| OrderError::BadState("missing hold payment hash".into()))?;
        let hold_preimage = hold_preimage
            .ok_or_else(|| OrderError::BadState("missing hold payment preimage".into()))?;
        let buyer_address = buyer_invoice
            .ok_or_else(|| OrderError::BadState("missing buyer invoice".into()))?;

        {
            let mut market = self.guard();
            let trade = market
                .trades
                .get_mut(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            trade.push_log(format!(
                "pay: using buyer invoice {buyer_address} (normal invoice, preimage on buyer node)"
            ));
            save(&self.path, &market).map_err(OrderError::Save)?;
        }

        if let Ok(raw) = std::env::var("TWINE_RELEASE_PAUSE_MS") {
            if let Ok(ms) = raw.parse::<u64>() {
                if ms > 0 {
                    eprintln!("TWINE_RELEASE_PAUSE_MS={ms}: pausing before send_payment");
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                }
            }
        }

        let sent = match send_payment_to_invoice(rpc, &config.twine_rpc, &buyer_address).await {
            Ok(sent) => sent,
            Err(err) => {
                return self.record_pay_failure(id, format!("send_payment failed: {err}"), on_failure);
            }
        };
        let pay_hash = payment_hash_of(&sent).unwrap_or_else(|| buyer_address.clone());

        {
            let mut market = self.guard();
            let trade = market
                .trades
                .get_mut(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            trade.push_log(format!(
                "pay: twine send_payment submitted payment_hash={pay_hash}"
            ));
            save(&self.path, &market).map_err(OrderError::Save)?;
        }

        match poll_payment_done(rpc, &config.twine_rpc, &pay_hash).await {
            Ok(status) => {
                let mut market = self.guard();
                let trade = market
                    .trades
                    .get_mut(id)
                    .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
                trade.push_log(format!("pay: get_payment={status}"));
                save(&self.path, &market).map_err(OrderError::Save)?;
            }
            Err(err) => {
                let detail = match &err {
                    OrderError::Fiber(message) => message.clone(),
                    other => format!("{other:?}"),
                };
                return self.record_pay_failure(id, detail, on_failure);
            }
        }

        settle_hold(rpc, &config.twine_rpc, &hold_hash, &hold_preimage)
            .await
            .map_err(OrderError::Fiber)?;

        {
            let mut market = self.guard();
            let trade = market
                .trades
                .get_mut(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            trade.push_log("settle: settle_invoice(H, S) submitted on twine");
            save(&self.path, &market).map_err(OrderError::Save)?;
        }

        let status = poll_invoice_paid(rpc, &config.twine_rpc, &hold_hash).await?;
        let mut market = self.guard();
        let (view, ad_id) = {
            let trade = market
                .trades
                .get_mut(id)
                .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
            trade.state = OrderState::Settled;
            trade.invoice_status = Some(status.clone());
            trade.push_log(format!("settle: hold invoice={status}"));
            trade.push_log(match on_failure {
                PayFailureMode::Leg2Failed => "path A complete: state Settled".to_string(),
                PayFailureMode::StayDisputed => {
                    "path C buyer wins complete: state Settled".to_string()
                }
            });
            (trade.view(), trade.ad_id.clone())
        };
        if let Some(ad) = market.ads.get_mut(&ad_id) {
            if ad.open_trade_id.as_deref() == Some(id) {
                ad.open_trade_id = None;
            }
        }
        save(&self.path, &market).map_err(OrderError::Save)?;
        eprintln!("settled H={hold_hash} invoice={status} state=Settled");
        Ok(view)
    }

    fn record_pay_failure(
        &self,
        id: &str,
        detail: String,
        mode: PayFailureMode,
    ) -> Result<TradeView, OrderError> {
        match mode {
            PayFailureMode::Leg2Failed => self.record_leg2_failed(id, detail),
            PayFailureMode::StayDisputed => self.record_award_buyer_failed(id, detail),
        }
    }

    fn record_leg2_failed(&self, id: &str, detail: String) -> Result<TradeView, OrderError> {
        let mut market = self.guard();
        let trade = market
            .trades
            .get_mut(id)
            .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
        trade.state = OrderState::Leg2Failed;
        trade.push_log(format!(
            "pay failed: {detail}; settle_invoice not called (hold stays Received)"
        ));
        trade.push_log(
            "path B: buyer should submit a new invoice to retry (POST /trades/:id/retry)",
        );
        trade.push_log(
            "if the buyer never returns, the seller is refunded when the TLC expires (do not cancel_invoice on Received)",
        );
        let view = trade.view();
        save(&self.path, &market).map_err(OrderError::Save)?;
        eprintln!("path B Leg2Failed: {detail}");
        Ok(view)
    }

    fn record_award_buyer_failed(&self, id: &str, detail: String) -> Result<TradeView, OrderError> {
        let mut market = self.guard();
        let trade = market
            .trades
            .get_mut(id)
            .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))?;
        trade.state = OrderState::Disputed;
        trade.push_log(format!(
            "path C buyer award pay failed: {detail}; settle_invoice not called; stay Disputed (hold stays Received)"
        ));
        let view = trade.view();
        save(&self.path, &market).map_err(OrderError::Save)?;
        eprintln!("path C award buyer failed, stay Disputed: {detail}");
        Ok(view)
    }

    fn rollback_trade(&self, trade_id: &str, ad_id: &str, amount: &str) -> Result<(), OrderError> {
        let mut market = self.guard();
        market.trades.remove(trade_id);
        return_ckb(&mut market, ad_id, trade_id, amount)?;
        save(&self.path, &market).map_err(OrderError::Save)
    }

    fn trade_state(&self, id: &str) -> Result<OrderState, OrderError> {
        self.guard()
            .trades
            .get(id)
            .map(|trade| trade.state)
            .ok_or_else(|| OrderError::NotFound(format!("trade {id} not found")))
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, Market> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

fn ad_has_open_trade(market: &Market, ad: &Ad) -> bool {
    let Some(id) = &ad.open_trade_id else {
        return false;
    };
    market
        .trades
        .get(id)
        .is_some_and(Trade::is_open)
}

fn ad_is_shoppable(market: &Market, ad: &Ad) -> bool {
    !ad.cancelled && !ad_has_open_trade(market, ad) && can_fill_min(ad)
}

fn can_fill_min(ad: &Ad) -> bool {
    if parse_available(&ad.available).is_ok_and(|value| value == 0) {
        return false;
    }
    if ad.min.trim().is_empty() {
        return parse_available(&ad.available).is_ok_and(|value| value > 0);
    }
    match ckb_from_fiat(&ad.min, &ad.price) {
        Ok(min_ckb) => {
            cmp_ckb(&ad.available, &min_ckb).is_ok_and(|order| order != std::cmp::Ordering::Less)
        }
        Err(_) => false,
    }
}

fn return_ckb(market: &mut Market, ad_id: &str, trade_id: &str, amount: &str) -> Result<(), OrderError> {
    let Some(ad) = market.ads.get_mut(ad_id) else {
        return Ok(());
    };
    ad.available = add_ckb(&ad.available, amount).map_err(OrderError::BadAmount)?;
    if ad.open_trade_id.as_deref() == Some(trade_id) {
        ad.open_trade_id = None;
    }
    Ok(())
}

fn subtract_ckb(left: &str, right: &str) -> Result<String, String> {
    let left = crate::order::parse_decimal_8(left)?;
    let right = crate::order::parse_decimal_8(right)?;
    if right > left {
        return Err("not enough CKB available".into());
    }
    Ok(crate::order::format_decimal_8(left - right))
}

fn parse_available(raw: &str) -> Result<u128, String> {
    crate::order::parse_decimal_8(raw)
}

fn require_text(raw: &str, field: &str) -> Result<String, OrderError> {
    let text = raw.trim();
    if text.is_empty() {
        return Err(OrderError::BadState(format!("{field} is required")));
    }
    Ok(text.to_string())
}

fn save(path: &Path, market: &Market) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    let text = serde_json::to_string_pretty(market).map_err(|err| err.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|err| err.to_string())?;
    fs::rename(&tmp, path).map_err(|err| err.to_string())
}

#[cfg(test)]
mod range_tests {
    use super::*;

    fn ad(available: &str, min: &str) -> Ad {
        Ad {
            id: "ad1".into(),
            pubkey: "aa".into(),
            available: available.into(),
            currency: "NGN".into(),
            price: "2000".into(),
            min: min.into(),
            max: "4000".into(),
            payment_method: "Opay".into(),
            cancelled: false,
            open_trade_id: None,
        }
    }

    #[test]
    fn leftover_below_min_is_not_shoppable() {
        let market = Market::default();
        assert!(ad_is_shoppable(&market, &ad("1", "2000")));
        assert!(!ad_is_shoppable(&market, &ad("0.5", "2000")));
        assert!(!ad_is_shoppable(&market, &ad("0", "2000")));
    }
}

