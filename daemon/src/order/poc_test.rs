//! One walk of the proof of concept against a fake Fiber node.
//!
//! Post an ad, take it, cancel-while-open, lock, refuse cancel after Received,
//! path B then a path A retry, path C both ways, then path D expiry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::{Path as UrlPath, State};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::order::{
    BuyerInvoiceBody, CreateAdBody, CreateTradeBody, OrderError, OrderState, PostChatBody,
    TradeView, fiat_from_ckb,
};
use crate::health::Config;
use crate::market::MarketStore;
use crate::rpc::FiberRpc;

const ONE_CKB: &str = "0x5f5e100";
const EXPIRY_DELTA: &str = "0x36ee800";
const PEER: &str = "02cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const SELLER: &str = "02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BUYER: &str = "02bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct Hold {
    address: String,
    status: String,
}

struct FiberNode {
    holds: HashMap<String, Hold>,
    demos: HashMap<String, String>,
    payments: HashMap<String, String>,
    fail_twine_payments: u32,
    pay_seq: u32,
    calls: Vec<String>,
    cancels: Vec<String>,
    faults: Vec<String>,
}

struct RemoveFile(PathBuf);

impl Drop for RemoveFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn walks_hold_release_dispute_and_expiry() {
    std::env::remove_var("TWINE_RELEASE_PAUSE_MS");
    let path = temp_path();
    let _remove = RemoveFile(path.clone());
    let fiber = Arc::new(Mutex::new(FiberNode::new()));
    let config = serve(Arc::clone(&fiber)).await;
    let rpc = FiberRpc::new();
    let store = MarketStore::open(path.clone()).unwrap();

    assert!(store
        .poll_all_hold_expiry(&rpc, &config)
        .await
        .unwrap()
        .is_empty());

    let created = start_trade(&store, &rpc, &config).await;
    assert_eq!(created.state, OrderState::WaitingHold);
    assert_eq!(created.invoice_status.as_deref(), Some("Open"));
    assert!(log_contains(&created, "S sealed"));
    assert_hides_preimage(&created, &path);
    let hold_hash = created.payment_hash.clone().unwrap();
    let trade_id = created.id.clone();
    let reloaded = MarketStore::open(path.clone()).unwrap();
    assert_eq!(
        reloaded.get_trade(&trade_id).unwrap().payment_hash.as_deref(),
        Some(hold_hash.as_str())
    );
    assert!(reloaded.persisted_preimage(&trade_id).is_some());

    let after_demo = store.demo_cancel(&trade_id, &rpc, &config).await.unwrap();
    assert_eq!(after_demo.state, OrderState::WaitingHold);
    assert!(log_contains(&after_demo, "demo cancel"));
    assert!(log_contains(&after_demo, "Cancelled"));

    assert!(matches!(
        start_trade_on(&store, &rpc, &config, &created.ad_id).await,
        Err(OrderError::AlreadyOpen)
    ));

    mark_received(&fiber, &hold_hash);
    let locked = store.mark_locked(&trade_id, &rpc, &config).await.unwrap();
    assert_eq!(locked.state, OrderState::WaitingFiat);
    assert_eq!(locked.invoice_status.as_deref(), Some("Received"));
    assert_eq!(fiber.lock().unwrap().status_of(&hold_hash), "Received");
    let cancels_before = fiber.lock().unwrap().cancels.len();
    let skipped = store.try_cancel(&trade_id, &rpc, &config).await.unwrap();
    assert_eq!(skipped.state, OrderState::WaitingFiat);
    assert!(log_contains(&skipped, "cancel_invoice not applied"));
    assert_eq!(fiber.lock().unwrap().cancels.len(), cancels_before);

    let fiat_sent = store
        .fiat_sent(&trade_id, &buyer_invoice("1"))
        .unwrap();
    assert_eq!(fiat_sent.state, OrderState::FiatSent);

    fiber.lock().unwrap().fail_twine_payments = 1;
    let marked = call_len(&fiber);
    let failed = store.release(&trade_id, &rpc, &config).await.unwrap();
    assert_eq!(failed.state, OrderState::Leg2Failed);
    assert_eq!(failed.invoice_status.as_deref(), Some("Received"));
    assert!(log_contains(&failed, "settle_invoice not called"));
    assert!(log_contains(&failed, "TLC expires"));
    assert!(!failed.log.iter().any(|line| line.text.contains("settle:")));
    let failed_calls = calls_since(&fiber, marked);
    assert!(failed_calls.iter().any(|call| call == "twine send_payment"));
    assert!(!failed_calls
        .iter()
        .any(|call| call.contains("settle_invoice")));
    assert!(!failed_calls
        .iter()
        .any(|call| call.contains("cancel_invoice")));
    assert!(!failed_calls.iter().any(|call| call.contains("new_invoice")));
    assert_eq!(fiber.lock().unwrap().status_of(&hold_hash), "Received");

    let marked = call_len(&fiber);
    let settled = store
        .retry(&trade_id, &rpc, &config, &buyer_invoice("2"))
        .await
        .unwrap();
    assert_eq!(settled.state, OrderState::Settled);
    assert_eq!(settled.invoice_status.as_deref(), Some("Paid"));
    assert!(log_contains(&settled, "path A complete"));
    assert_pay_then_settle(&settled);
    assert_hides_preimage(&settled, &path);
    assert_pay_then_settle_calls(&calls_since(&fiber, marked));
    assert_eq!(fiber.lock().unwrap().status_of(&hold_hash), "Paid");
    assert!(store
        .poll_hold_expiry(&trade_id, &rpc, &config)
        .await
        .unwrap()
        .is_none());

    let disputed = open_dispute(&store, &fiber, &rpc, &config, &path).await;
    let disputed_id = disputed.id.clone();
    let disputed_hash = disputed.payment_hash.clone().unwrap();
    fiber.lock().unwrap().fail_twine_payments = 1;
    let marked = call_len(&fiber);
    let stayed = store
        .award_buyer(&disputed_id, &rpc, &config, &buyer_invoice("3"))
        .await
        .unwrap();
    assert_eq!(stayed.state, OrderState::Disputed);
    assert_eq!(stayed.invoice_status.as_deref(), Some("Received"));
    assert!(log_contains(&stayed, "stay Disputed"));
    assert!(log_contains(&stayed, "settle_invoice not called"));
    assert!(!calls_since(&fiber, marked)
        .iter()
        .any(|call| call.contains("settle_invoice")));
    assert_eq!(fiber.lock().unwrap().status_of(&disputed_hash), "Received");

    let marked = call_len(&fiber);
    let buyer_wins = store
        .award_buyer(&disputed_id, &rpc, &config, &buyer_invoice("4"))
        .await
        .unwrap();
    assert_eq!(buyer_wins.state, OrderState::Settled);
    assert_eq!(buyer_wins.invoice_status.as_deref(), Some("Paid"));
    assert!(log_contains(&buyer_wins, "path C buyer wins complete"));
    assert_pay_then_settle(&buyer_wins);
    assert_pay_then_settle_calls(&calls_since(&fiber, marked));
    assert_hides_preimage(&buyer_wins, &path);

    let seller_case = open_dispute(&store, &fiber, &rpc, &config, &path).await;
    let seller_id = seller_case.id.clone();
    let seller_hash = seller_case.payment_hash.clone().unwrap();
    store
        .post_chat(
            &seller_id,
            &PostChatBody {
                from: "buyer".into(),
                text: "I sent the fiat".into(),
            },
        )
        .unwrap();
    let chat = store
        .post_chat(
            &seller_id,
            &PostChatBody {
                from: "seller".into(),
                text: "I never got it".into(),
            },
        )
        .unwrap();
    assert_eq!(chat.chat.len(), 2);
    assert_eq!(chat.chat[0].text, "I sent the fiat");
    assert_eq!(chat.chat[1].from, "seller");
    assert_hides_preimage(&chat, &path);
    let marked = call_len(&fiber);
    let seller_wins = store.award_seller(&seller_id, &rpc, &config).await.unwrap();
    assert_eq!(seller_wins.state, OrderState::Disputed);
    assert_eq!(seller_wins.invoice_status.as_deref(), Some("Received"));
    assert!(log_contains(
        &seller_wins,
        "settle_invoice not called; cancel_invoice not called"
    ));
    assert!(log_contains(&seller_wins, "TLC expiry"));
    let award_calls = calls_since(&fiber, marked);
    assert!(award_calls.iter().any(|call| call == "twine get_invoice"));
    assert!(!award_calls
        .iter()
        .any(|call| call.contains("settle_invoice")));
    assert!(!award_calls
        .iter()
        .any(|call| call.contains("cancel_invoice")));
    assert!(matches!(
        start_trade_on(&store, &rpc, &config, &seller_case.ad_id).await,
        Err(OrderError::AlreadyOpen)
    ));
    assert_eq!(fiber.lock().unwrap().status_of(&seller_hash), "Received");

    {
        let mut node = fiber.lock().unwrap();
        node.holds.get_mut(&seller_hash).unwrap().status = "Expired".into();
    }
    let marked = call_len(&fiber);
    assert!(matches!(
        store
            .award_buyer(&seller_id, &rpc, &config, &buyer_invoice("5"))
            .await
            .unwrap_err(),
        OrderError::BadState(message) if message.contains("Expired")
    ));
    assert!(matches!(
        store.award_seller(&seller_id, &rpc, &config).await.unwrap_err(),
        OrderError::BadState(message) if message.contains("Expired")
    ));
    assert!(!calls_since(&fiber, marked)
        .iter()
        .any(|call| call.contains("settle_invoice")));
    assert_eq!(
        store.get_trade(&seller_id).unwrap().state,
        OrderState::Disputed
    );

    let expired = store
        .poll_hold_expiry(&seller_id, &rpc, &config)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(expired.state, OrderState::Expired);
    assert_eq!(expired.invoice_status.as_deref(), Some("Expired"));
    assert!(log_contains(&expired, "path D: hold invoice Expired"));
    assert!(log_contains(
        &expired,
        "seller refunded because the TLC expired"
    ));
    assert!(log_contains(&expired, "cancel_invoice not called"));
    assert!(log_contains(
        &expired,
        "settle_invoice(H, S) after expiry failed as expected"
    ));
    assert!(!log_contains(&expired, "path A complete"));
    assert!(!log_contains(&expired, "path C buyer wins"));
    assert_hides_preimage(&expired, &path);
    assert_eq!(fiber.lock().unwrap().status_of(&seller_hash), "Expired");
    assert!(store
        .poll_hold_expiry(&seller_id, &rpc, &config)
        .await
        .unwrap()
        .is_none());

    let ads = store.list_ads();
    let restored = ads
        .iter()
        .find(|ad| ad.id == seller_case.ad_id)
        .expect("ad remains after expiry");
    assert_eq!(restored.available, "1");
    let next = start_trade_on(&store, &rpc, &config, &seller_case.ad_id)
        .await
        .unwrap();
    assert_eq!(next.state, OrderState::WaitingHold);

    let node = fiber.lock().unwrap();
    assert!(node.faults.is_empty(), "{:?}", node.faults);
    assert!(!node.cancels.is_empty());
    for hash in node.holds.keys() {
        assert!(
            !node.cancels.iter().any(|cancelled| cancelled == hash),
            "{hash}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ad_reserve_blocks_second_take_and_returns_on_cancel() {
    let path = temp_path();
    let _remove = RemoveFile(path.clone());
    let fiber = Arc::new(Mutex::new(FiberNode::new()));
    let config = serve(Arc::clone(&fiber)).await;
    let rpc = FiberRpc::new();
    let store = MarketStore::open(path).unwrap();

    let ad = post_ad(&store, "1");
    let first = start_trade_on(&store, &rpc, &config, &ad.id)
        .await
        .unwrap();
    assert!(!store.list_ads().iter().any(|listed| listed.id == ad.id));
    assert!(matches!(
        start_trade_on(&store, &rpc, &config, &ad.id).await,
        Err(OrderError::AlreadyOpen)
    ));

    let cancelled = store.try_cancel(&first.id, &rpc, &config).await.unwrap();
    assert_eq!(cancelled.state, OrderState::Cancelled);
    let restored = store
        .list_ads()
        .into_iter()
        .find(|listed| listed.id == ad.id)
        .expect("cancelled trade returns CKB to the ad");
    assert_eq!(restored.available, "1");

    let second = start_trade_on(&store, &rpc, &config, &ad.id)
        .await
        .unwrap();
    assert_eq!(second.state, OrderState::WaitingHold);
    assert!(!store.list_ads().iter().any(|listed| listed.id == ad.id));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn range_take_locks_the_slice_and_hides_dust() {
    let path = temp_path();
    let _remove = RemoveFile(path.clone());
    let fiber = Arc::new(Mutex::new(FiberNode::new()));
    let config = serve(Arc::clone(&fiber)).await;
    let rpc = FiberRpc::new();
    let store = MarketStore::open(path).unwrap();

    assert!(matches!(
        store.create_ad(&CreateAdBody {
            pubkey: SELLER.into(),
            available: "1".into(),
            currency: Some("NGN".into()),
            price: "2000".into(),
            min: "3000".into(),
            max: "4000".into(),
            payment_method: "Opay".into(),
        }),
        Err(OrderError::BadAmount(_))
    ));

    let ad = store
        .create_ad(&CreateAdBody {
            pubkey: SELLER.into(),
            available: "2".into(),
            currency: Some("NGN".into()),
            price: "2000".into(),
            min: "2000".into(),
            max: "3000".into(),
            payment_method: "Opay".into(),
        })
        .unwrap();
    assert_eq!(ad.min, "2000");
    assert_eq!(ad.max, "3000");

    let too_small = store
        .create_trade(
            &rpc,
            &config,
            &CreateTradeBody {
                ad_id: ad.id.clone(),
                taker: BUYER.into(),
                pay_amount: "1000".into(),
            },
        )
        .await;
    assert!(matches!(too_small, Err(OrderError::BadAmount(_))));

    let too_big = store
        .create_trade(
            &rpc,
            &config,
            &CreateTradeBody {
                ad_id: ad.id.clone(),
                taker: BUYER.into(),
                pay_amount: "4000".into(),
            },
        )
        .await;
    assert!(matches!(too_big, Err(OrderError::BadAmount(_))));

    let first = store
        .create_trade(
            &rpc,
            &config,
            &CreateTradeBody {
                ad_id: ad.id.clone(),
                taker: BUYER.into(),
                pay_amount: "2000".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(first.amount, "1");
    assert!(!store.list_ads().iter().any(|listed| listed.id == ad.id));

    let cancelled = store.try_cancel(&first.id, &rpc, &config).await.unwrap();
    assert_eq!(cancelled.state, OrderState::Cancelled);
    let restored = store
        .list_ads()
        .into_iter()
        .find(|listed| listed.id == ad.id)
        .expect("cancelled slice returns CKB to the ad");
    assert_eq!(restored.available, "2");
}

async fn open_dispute(
    store: &MarketStore,
    fiber: &Mutex<FiberNode>,
    rpc: &FiberRpc,
    config: &Config,
    path: &Path,
) -> TradeView {
    let waiting = start_trade(store, rpc, config).await;
    assert_eq!(waiting.state, OrderState::WaitingHold);
    assert!(log_contains(&waiting, EXPIRY_DELTA));
    let hash = waiting.payment_hash.clone().unwrap();
    mark_received(fiber, &hash);
    let locked = store.mark_locked(&waiting.id, rpc, config).await.unwrap();
    assert_eq!(locked.invoice_status.as_deref(), Some("Received"));
    let disputed = store.open_dispute(&waiting.id, rpc, config).await.unwrap();
    assert_eq!(disputed.state, OrderState::Disputed);
    assert_eq!(disputed.invoice_status.as_deref(), Some("Received"));
    assert_hides_preimage(&disputed, path);
    disputed
}

async fn start_trade(
    store: &MarketStore,
    rpc: &FiberRpc,
    config: &Config,
) -> TradeView {
    let ad = post_ad(store, "1");
    start_trade_on(store, rpc, config, &ad.id)
        .await
        .expect("create trade")
}

async fn start_trade_on(
    store: &MarketStore,
    rpc: &FiberRpc,
    config: &Config,
    ad_id: &str,
) -> Result<TradeView, OrderError> {
    store
        .create_trade(
            rpc,
            config,
            &CreateTradeBody {
                ad_id: ad_id.into(),
                taker: BUYER.into(),
                pay_amount: "2000".into(),
            },
        )
        .await
}

fn post_ad(store: &MarketStore, available: &str) -> crate::market::AdView {
    store
        .create_ad(&CreateAdBody {
            pubkey: SELLER.into(),
            available: available.into(),
            currency: Some("NGN".into()),
            price: "2000".into(),
            min: "2000".into(),
            max: fiat_from_ckb(available, "2000").expect("listing pay cap"),
            payment_method: "Opay".into(),
        })
        .unwrap()
}

fn buyer_invoice(seq: &str) -> BuyerInvoiceBody {
    BuyerInvoiceBody {
        invoice: format!("fibb1buyer{seq}"),
    }
}

fn mark_received(fiber: &Mutex<FiberNode>, hash: &str) {
    fiber
        .lock()
        .unwrap()
        .holds
        .get_mut(hash)
        .expect("hold")
        .status = "Received".into();
}

async fn serve(fiber: Arc<Mutex<FiberNode>>) -> Config {
    let app = Router::new()
        .route("/{node}", post(rpc))
        .with_state(fiber);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let config = Config {
        listen: "127.0.0.1:9".into(),
        twine_rpc: format!("http://{addr}/twine"),
        twine_p2p: "/ip4/127.0.0.1/tcp/8238".into(),
        funding_shannons: 50_000_000_000,
    };
    let rpc = FiberRpc::new();
    for _ in 0..50 {
        if rpc
            .call(&config.twine_rpc, "node_info", json!([]))
            .await
            .is_ok()
        {
            return config;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("fake Fiber node did not answer");
}

async fn rpc(
    UrlPath(node): UrlPath<String>,
    State(fiber): State<Arc<Mutex<FiberNode>>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let method = body
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let arg = body
        .get("params")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or(Value::Null);
    let mut fiber = fiber.lock().unwrap_or_else(|err| err.into_inner());
    fiber.calls.push(format!("{node} {method}"));
    Json(fiber.handle(&node, &method, &arg))
}

impl FiberNode {
    fn new() -> Self {
        Self {
            holds: HashMap::new(),
            demos: HashMap::new(),
            payments: HashMap::new(),
            fail_twine_payments: 0,
            pay_seq: 0,
            calls: Vec::new(),
            cancels: Vec::new(),
            faults: Vec::new(),
        }
    }

    fn status_of(&self, hash: &str) -> String {
        self.holds
            .get(hash)
            .map(|hold| hold.status.clone())
            .unwrap_or_else(|| "missing".into())
    }

    fn handle(&mut self, node: &str, method: &str, arg: &Value) -> Value {
        match method {
            "node_info" => ok(json!({
                "pubkey": PEER,
                "node_name": format!("twine-poc-{node}"),
            })),
            "list_channels" => ok(json!({
                "channels": [{
                    "channel_id": "0xchannel",
                    "pubkey": PEER,
                    "funding_udt_type_script": null,
                    "state": {"state_name": "ChannelReady"},
                    "local_balance": "0x1dcd6500",
                    "remote_balance": "0x0"
                }]
            })),
            "new_invoice" => self.new_invoice(node, arg),
            "cancel_invoice" => self.cancel_invoice(arg),
            "get_invoice" => self.get_invoice(arg),
            "send_payment" => self.send_payment(node, arg),
            "get_payment" => self.get_payment(arg),
            "settle_invoice" => self.settle_invoice(arg),
            other => self.fault(format!("unexpected {node} {other}")),
        }
    }

    fn new_invoice(&mut self, node: &str, arg: &Value) -> Value {
        if field(arg, "currency") != "Fibt" || field(arg, "amount") != ONE_CKB {
            return self.fault(format!("new_invoice amount/currency {arg}"));
        }
        if field(arg, "hash_algorithm") != "sha256" {
            return self.fault(format!("new_invoice hash_algorithm {arg}"));
        }
        if arg.get("payment_preimage").is_some() {
            return self.fault("new_invoice must not carry payment_preimage".into());
        }
        if node != "twine" {
            return self.fault(format!("new_invoice on {node}"));
        }
        let hash = field(arg, "payment_hash");
        if hash.is_empty() {
            return self.fault("twine new_invoice missing payment_hash".into());
        }
        let address = format!("fibb1{hash}");
        let description = field(arg, "description");
        if description.contains("demo cancel") {
            self.demos.insert(hash.into(), "Open".into());
            return ok(invoice_json(hash, &address, "Open"));
        }
        if field(arg, "final_expiry_delta") != EXPIRY_DELTA {
            return self.fault(format!(
                "hold final_expiry_delta want {EXPIRY_DELTA} got {}",
                field(arg, "final_expiry_delta")
            ));
        }
        self.holds.insert(
            hash.into(),
            Hold {
                address: address.clone(),
                status: "Open".into(),
            },
        );
        ok(invoice_json(hash, &address, "Open"))
    }

    fn cancel_invoice(&mut self, arg: &Value) -> Value {
        let hash = field(arg, "payment_hash");
        self.cancels.push(hash.into());
        if let Some(status) = self.demos.get_mut(hash) {
            if status == "Open" {
                *status = "Cancelled".into();
                return ok(json!({"status": "Cancelled", "payment_hash": hash}));
            }
        }
        if let Some(hold) = self.holds.get_mut(hash) {
            if hold.status == "Open" {
                hold.status = "Cancelled".into();
                return ok(json!({"status": "Cancelled", "payment_hash": hash}));
            }
        }
        self.fault(format!("cancel_invoice refused for {hash}"))
    }

    fn get_invoice(&mut self, arg: &Value) -> Value {
        let hash = field(arg, "payment_hash");
        if let Some(status) = self.demos.get(hash) {
            return ok(invoice_json(hash, &format!("fibb1{hash}"), status));
        }
        match self.holds.get(hash) {
            Some(hold) => ok(invoice_json(hash, &hold.address, &hold.status)),
            None => self.fault(format!("unknown invoice {hash}")),
        }
    }

    fn send_payment(&mut self, node: &str, arg: &Value) -> Value {
        let invoice = field(arg, "invoice");
        if node != "twine" {
            return self.fault(format!("send_payment on {node}"));
        }
        if self.fail_twine_payments > 0 {
            self.fail_twine_payments -= 1;
            return fail("no route to buyer");
        }
        if !invoice.starts_with("fibb1buyer") {
            return self.fault(format!("twine paid unexpected invoice {invoice}"));
        }
        self.pay_seq += 1;
        let hash = format!("0xpay{:x}", self.pay_seq);
        self.payments.insert(hash.clone(), "Success".into());
        ok(json!({"payment_hash": hash, "status": "Created"}))
    }

    fn get_payment(&mut self, arg: &Value) -> Value {
        let hash = field(arg, "payment_hash");
        match self.payments.get(hash) {
            Some(status) => ok(json!({"payment_hash": hash, "status": status})),
            None => ok(json!({
                "payment_hash": hash,
                "status": "Failed",
                "failed_error": "unknown payment"
            })),
        }
    }

    fn settle_invoice(&mut self, arg: &Value) -> Value {
        let hash = field(arg, "payment_hash").to_string();
        let preimage = field(arg, "payment_preimage");
        let hashed = match hex::decode(preimage.trim_start_matches("0x")) {
            Ok(raw) => format!("0x{}", hex::encode(Sha256::digest(raw))),
            Err(err) => return self.fault(format!("settle preimage: {err}")),
        };
        if hashed != hash {
            return self.fault(format!("settle preimage does not hash to {hash}"));
        }
        let status = self.holds.get(&hash).map(|hold| hold.status.clone());
        let Some(status) = status else {
            return self.fault(format!("settle unknown hold {hash}"));
        };
        if status == "Expired" {
            return fail("invoice Expired");
        }
        if status != "Received" {
            return self.fault(format!("settle while {status}"));
        }
        self.holds.get_mut(&hash).unwrap().status = "Paid".into();
        ok(json!({"status": "Paid", "payment_hash": hash}))
    }

    fn fault(&mut self, message: String) -> Value {
        self.faults.push(message.clone());
        fail(&message)
    }
}

fn invoice_json(hash: &str, address: &str, status: &str) -> Value {
    json!({
        "invoice_address": address,
        "status": status,
        "payment_hash": hash,
        "invoice": {
            "data": {
                "payment_hash": hash,
                "attrs": [{ "final_htlc_minimum_expiry_delta": EXPIRY_DELTA }]
            }
        }
    })
}

fn ok(result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "result": result})
}

fn fail(message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -1, "message": message}})
}

fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn log_contains(view: &TradeView, needle: &str) -> bool {
    view.log.iter().any(|line| line.text.contains(needle))
}

fn assert_pay_then_settle(view: &TradeView) {
    let pay = view
        .log
        .iter()
        .position(|line| line.text.starts_with("pay:"))
        .expect("pay log");
    let settle = view
        .log
        .iter()
        .position(|line| line.text.starts_with("settle:"))
        .expect("settle log");
    assert!(pay < settle, "{:?}", view.log);
}

fn assert_pay_then_settle_calls(calls: &[String]) {
    let pay = calls
        .iter()
        .position(|call| call == "twine send_payment")
        .expect("twine send_payment");
    let settle = calls
        .iter()
        .position(|call| call == "twine settle_invoice")
        .expect("twine settle_invoice");
    assert!(pay < settle, "{calls:?}");
    assert!(!calls.iter().any(|call| call.contains("new_invoice")));
}

fn assert_hides_preimage(view: &TradeView, path: &Path) {
    let file: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let preimage = file["trades"][&view.id]["payment_preimage"]
        .as_str()
        .expect("preimage stays on disk");
    let json = serde_json::to_string(view).unwrap();
    let bare = preimage.strip_prefix("0x").unwrap_or(preimage);
    assert!(!json.contains("payment_preimage"), "{json}");
    assert!(
        bare.len() == 64 && !json.contains(bare),
        "view leaked preimage"
    );
}

fn call_len(fiber: &Mutex<FiberNode>) -> usize {
    fiber.lock().unwrap().calls.len()
}

fn calls_since(fiber: &Mutex<FiberNode>, start: usize) -> Vec<String> {
    fiber.lock().unwrap().calls[start..].to_vec()
}

fn temp_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "twine-poc-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}
