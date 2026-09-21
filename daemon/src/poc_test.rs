//! One walk of the proof of concept against a fake Fiber node.
//!
//! Create, cancel-while-open, hold, lock, refuse cancel after Received,
//! path B then a path A retry, path C both ways, then path D expiry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::{Path as UrlPath, State};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{OrderError, OrderState, OrderStore, OrderView, PostChatBody};
use crate::health::Config;
use crate::rpc::FiberRpc;

const ONE_CKB: &str = "0x5f5e100";
const EXPIRY_DELTA: &str = "0x36ee800";
const PEER: &str = "02cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const LOCAL_IN_FLIGHT: &str = "0x1dcd6500";
const LOCAL_RESTORED: &str = "0x2faf0800";

struct Hold {
    address: String,
    status: String,
}

struct FiberNode {
    holds: HashMap<String, Hold>,
    demos: HashMap<String, String>,
    payments: HashMap<String, String>,
    last_buyer_invoice: Option<String>,
    active_hold: Option<String>,
    fail_twine_payments: u32,
    seller_local: String,
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
    let store = OrderStore::open(path.clone()).unwrap();

    assert!(store
        .poll_hold_expiry(&rpc, &config)
        .await
        .unwrap()
        .is_none());

    // Unpaid cancel leaves the live order Pending. A second create is refused.
    let created = store.create("1").unwrap();
    assert_eq!(created.state, OrderState::Pending);
    assert_eq!(store.create("9").unwrap_err(), OrderError::AlreadyOpen);
    let after_demo = store.demo_cancel(&rpc, &config).await.unwrap();
    assert_eq!(after_demo.state, OrderState::Pending);
    assert!(after_demo.payment_hash.is_none());
    assert!(log_contains(&after_demo, "demo cancel"));
    assert!(log_contains(&after_demo, "Cancelled"));

    let waiting = store.create_hold(&rpc, &config).await.unwrap();
    assert_eq!(waiting.state, OrderState::WaitingHold);
    assert_eq!(waiting.invoice_status.as_deref(), Some("Open"));
    assert!(log_contains(&waiting, "S sealed"));
    assert_hides_preimage(&waiting, &path);
    let hold_hash = waiting.payment_hash.clone().unwrap();
    let reloaded = OrderStore::open(path.clone()).unwrap();
    assert_eq!(reloaded.snapshot().payment_hash.as_deref(), Some(hold_hash.as_str()));
    assert!(reloaded.guard().payment_preimage.is_some());

    let held = store.lock_payment(&rpc, &config).await.unwrap();
    assert_eq!(held.state, OrderState::Held);
    assert_eq!(held.invoice_status.as_deref(), Some("Received"));
    assert_eq!(fiber.lock().unwrap().status_of(&hold_hash), "Received");
    let cancels_before = fiber.lock().unwrap().cancels.len();
    let skipped = store.try_cancel(&rpc, &config).await.unwrap();
    assert_eq!(skipped.state, OrderState::Held);
    assert!(log_contains(&skipped, "cancel_invoice not applied"));
    assert_eq!(fiber.lock().unwrap().cancels.len(), cancels_before);

    store.accept().unwrap();
    let fiat_sent = store.fiat_sent().unwrap();
    assert_eq!(fiat_sent.state, OrderState::FiatSent);

    // Path B: paying the buyer fails. The hold stays Received and is not settled.
    fiber.lock().unwrap().fail_twine_payments = 1;
    let marked = call_len(&fiber);
    let failed = store.release(&rpc, &config).await.unwrap();
    assert_eq!(failed.state, OrderState::Leg2Failed);
    assert_eq!(failed.invoice_status.as_deref(), Some("Received"));
    assert!(log_contains(&failed, "settle_invoice not called"));
    assert!(log_contains(&failed, "TLC expires"));
    assert!(!failed.log.iter().any(|line| line.text.contains("settle:")));
    assert_eq!(store.create("1").unwrap_err(), OrderError::AlreadyOpen);
    let failed_calls = calls_since(&fiber, marked);
    assert!(failed_calls.iter().any(|call| call == "buyer new_invoice"));
    assert!(failed_calls.iter().any(|call| call == "twine send_payment"));
    assert!(!failed_calls.iter().any(|call| call.contains("settle_invoice")));
    assert!(!failed_calls.iter().any(|call| call.contains("cancel_invoice")));
    assert_eq!(fiber.lock().unwrap().status_of(&hold_hash), "Received");

    // Retry is path A: pay the buyer, then settle. Pay is logged before settle.
    let marked = call_len(&fiber);
    let settled = store.retry(&rpc, &config).await.unwrap();
    assert_eq!(settled.state, OrderState::Settled);
    assert_eq!(settled.invoice_status.as_deref(), Some("Paid"));
    assert!(log_contains(&settled, "path A complete"));
    assert_pay_then_settle(&settled);
    assert_hides_preimage(&settled, &path);
    assert_pay_then_settle_calls(&calls_since(&fiber, marked));
    assert_eq!(fiber.lock().unwrap().status_of(&hold_hash), "Paid");
    assert!(store.poll_hold_expiry(&rpc, &config).await.unwrap().is_none());

    // Path C, buyer wins, but the first route fails and must not settle.
    let disputed = open_dispute(&store, &rpc, &config, &path).await;
    let disputed_hash = disputed.payment_hash.clone().unwrap();
    fiber.lock().unwrap().fail_twine_payments = 1;
    let marked = call_len(&fiber);
    let stayed = store.award_buyer(&rpc, &config).await.unwrap();
    assert_eq!(stayed.state, OrderState::Disputed);
    assert_eq!(stayed.invoice_status.as_deref(), Some("Received"));
    assert!(log_contains(&stayed, "stay Disputed"));
    assert!(log_contains(&stayed, "settle_invoice not called"));
    assert!(!calls_since(&fiber, marked).iter().any(|call| call.contains("settle_invoice")));
    assert_eq!(fiber.lock().unwrap().status_of(&disputed_hash), "Received");

    let marked = call_len(&fiber);
    let buyer_wins = store.award_buyer(&rpc, &config).await.unwrap();
    assert_eq!(buyer_wins.state, OrderState::Settled);
    assert_eq!(buyer_wins.invoice_status.as_deref(), Some("Paid"));
    assert!(log_contains(&buyer_wins, "path C buyer wins complete"));
    assert_pay_then_settle(&buyer_wins);
    assert_pay_then_settle_calls(&calls_since(&fiber, marked));
    assert_hides_preimage(&buyer_wins, &path);

    // Path C, seller wins: chat is stored, nothing is settled or cancelled.
    let seller_case = open_dispute(&store, &rpc, &config, &path).await;
    let seller_hash = seller_case.payment_hash.clone().unwrap();
    store
        .post_chat(&PostChatBody {
            from: "buyer".into(),
            text: "I sent the fiat".into(),
        })
        .unwrap();
    let chat = store
        .post_chat(&PostChatBody {
            from: "seller".into(),
            text: "I never got it".into(),
        })
        .unwrap();
    assert_eq!(chat.chat.len(), 2);
    assert_eq!(chat.chat[0].text, "I sent the fiat");
    assert_eq!(chat.chat[1].from, "seller");
    assert_hides_preimage(&chat, &path);
    let marked = call_len(&fiber);
    let seller_wins = store.award_seller(&rpc, &config).await.unwrap();
    assert_eq!(seller_wins.state, OrderState::Disputed);
    assert_eq!(seller_wins.invoice_status.as_deref(), Some("Received"));
    assert!(log_contains(
        &seller_wins,
        "settle_invoice not called; cancel_invoice not called"
    ));
    assert!(log_contains(&seller_wins, "TLC expiry"));
    let award_calls = calls_since(&fiber, marked);
    assert!(award_calls.iter().any(|call| call == "twine get_invoice"));
    assert!(!award_calls.iter().any(|call| call.contains("settle_invoice")));
    assert!(!award_calls.iter().any(|call| call.contains("cancel_invoice")));
    assert_eq!(store.create("1").unwrap_err(), OrderError::AlreadyOpen);
    assert_eq!(fiber.lock().unwrap().status_of(&seller_hash), "Received");

    // Once Fiber expires the hold, both awards fail and the seller is refunded.
    {
        let mut node = fiber.lock().unwrap();
        node.holds.get_mut(&seller_hash).unwrap().status = "Expired".into();
        node.seller_local = LOCAL_RESTORED.into();
    }
    let marked = call_len(&fiber);
    assert!(matches!(
        store.award_buyer(&rpc, &config).await.unwrap_err(),
        OrderError::BadState(message) if message.contains("Expired")
    ));
    assert!(matches!(
        store.award_seller(&rpc, &config).await.unwrap_err(),
        OrderError::BadState(message) if message.contains("Expired")
    ));
    assert!(!calls_since(&fiber, marked).iter().any(|call| call.contains("settle_invoice")));
    assert_eq!(store.snapshot().state, OrderState::Disputed);

    let expired = store.poll_hold_expiry(&rpc, &config).await.unwrap().unwrap();
    assert_eq!(expired.state, OrderState::Expired);
    assert_eq!(expired.invoice_status.as_deref(), Some("Expired"));
    assert!(log_contains(&expired, "path D: hold invoice Expired"));
    assert!(log_contains(&expired, "seller refunded because the TLC expired"));
    assert!(log_contains(&expired, "cancel_invoice not called"));
    assert!(log_contains(
        &expired,
        "settle_invoice(H, S) after expiry failed as expected"
    ));
    assert!(log_contains(&expired, LOCAL_IN_FLIGHT));
    assert!(log_contains(&expired, LOCAL_RESTORED));
    assert!(!log_contains(&expired, "path A complete"));
    assert!(!log_contains(&expired, "path C buyer wins"));
    assert_hides_preimage(&expired, &path);
    assert_eq!(fiber.lock().unwrap().status_of(&seller_hash), "Expired");
    assert!(store.poll_hold_expiry(&rpc, &config).await.unwrap().is_none());

    let next = store.create("1").unwrap();
    assert_eq!(next.state, OrderState::Pending);

    let node = fiber.lock().unwrap();
    assert!(node.faults.is_empty(), "{:?}", node.faults);
    assert!(!node.cancels.is_empty());
    for hash in node.holds.keys() {
        assert!(!node.cancels.iter().any(|cancelled| cancelled == hash), "{hash}");
    }
}

async fn open_dispute(
    store: &OrderStore,
    rpc: &FiberRpc,
    config: &Config,
    path: &Path,
) -> OrderView {
    store.create("1").unwrap();
    let waiting = store.create_hold(rpc, config).await.unwrap();
    assert_eq!(waiting.state, OrderState::WaitingHold);
    assert!(log_contains(&waiting, EXPIRY_DELTA));
    let held = store.lock_payment(rpc, config).await.unwrap();
    assert_eq!(held.invoice_status.as_deref(), Some("Received"));
    store.accept().unwrap();
    let disputed = store.open_dispute(rpc, config).await.unwrap();
    assert_eq!(disputed.state, OrderState::Disputed);
    assert_eq!(disputed.invoice_status.as_deref(), Some("Received"));
    assert_hides_preimage(&disputed, path);
    disputed
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
        seller_rpc: format!("http://{addr}/seller"),
        twine_rpc: format!("http://{addr}/twine"),
        buyer_rpc: format!("http://{addr}/buyer"),
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
            last_buyer_invoice: None,
            active_hold: None,
            fail_twine_payments: 0,
            seller_local: LOCAL_IN_FLIGHT.into(),
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
                    "local_balance": self.seller_local,
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
        let description = field(arg, "description");
        if node == "buyer" {
            if arg.get("payment_hash").is_some() {
                return self.fault("buyer invoice is a normal invoice".into());
            }
            self.pay_seq += 1;
            let hash = format!("0xbuyer{:x}", self.pay_seq);
            let address = format!("fibb1buyer{:x}", self.pay_seq);
            self.last_buyer_invoice = Some(address.clone());
            return ok(invoice_json(&hash, &address, "Open"));
        }
        if node != "twine" {
            return self.fault(format!("new_invoice on {node}"));
        }
        let hash = field(arg, "payment_hash");
        if hash.is_empty() {
            return self.fault("twine new_invoice missing payment_hash".into());
        }
        let address = format!("fibb1{hash}");
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
        self.active_hold = Some(hash.into());
        ok(invoice_json(hash, &address, "Open"))
    }

    fn cancel_invoice(&mut self, arg: &Value) -> Value {
        let hash = field(arg, "payment_hash");
        self.cancels.push(hash.into());
        match self.demos.get_mut(hash) {
            Some(status) if status == "Open" => {
                *status = "Cancelled".into();
                ok(json!({"status": "Cancelled", "payment_hash": hash}))
            }
            _ => self.fault(format!("cancel_invoice refused for {hash}")),
        }
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
        if node == "seller" {
            let Some(hash) = self.active_hold.clone() else {
                return self.fault("seller send_payment with no hold".into());
            };
            let expected = self.holds.get(&hash).map(|hold| hold.address.clone());
            let Some(expected) = expected else {
                return self.fault("seller send_payment missing hold".into());
            };
            if expected != invoice {
                return self.fault(format!("seller paid {invoice}, hold is {expected}"));
            }
            self.holds.get_mut(&hash).unwrap().status = "Received".into();
            return ok(json!({"payment_hash": hash, "status": "Inflight"}));
        }
        if node != "twine" {
            return self.fault(format!("send_payment on {node}"));
        }
        if self.fail_twine_payments > 0 {
            self.fail_twine_payments -= 1;
            return fail("no route to buyer");
        }
        if self.last_buyer_invoice.as_deref() != Some(invoice) {
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

fn log_contains(view: &OrderView, needle: &str) -> bool {
    view.log.iter().any(|line| line.text.contains(needle))
}

fn assert_pay_then_settle(view: &OrderView) {
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
    assert!(calls.iter().any(|call| call == "buyer new_invoice"));
}

fn assert_hides_preimage(view: &OrderView, path: &Path) {
    let file: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let preimage = file["payment_preimage"]
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
