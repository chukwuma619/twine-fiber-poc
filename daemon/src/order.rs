use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct OrderStore {
    path: PathBuf,
    inner: Arc<Mutex<Order>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    pub state: OrderState,
    pub amount: Option<String>,
    pub log: Vec<LogLine>,
}

impl Order {
    fn idle() -> Self {
        Self {
            state: OrderState::Idle,
            amount: None,
            log: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum OrderState {
    Idle,
    Pending,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub at: String,
    pub text: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CreateError {
    BadAmount(String),
    AlreadyOpen,
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

    pub fn snapshot(&self) -> Order {
        self.lock().clone()
    }

    pub fn create(&self, amount: &str) -> Result<Order, CreateError> {
        let amount = normalize_amount(amount).map_err(CreateError::BadAmount)?;
        let mut order = self.lock();
        match order.state {
            OrderState::Idle => {}
            OrderState::Pending => return Err(CreateError::AlreadyOpen),
        }
        order.state = OrderState::Pending;
        order.amount = Some(amount.clone());
        order.log = vec![LogLine {
            at: timestamp(),
            text: format!("created order for {amount} CKB"),
        }];
        save(&self.path, &order).map_err(CreateError::Save)?;
        eprintln!("order created amount={amount} CKB state=Pending");
        Ok(order.clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Order> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
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

        let reloaded = OrderStore::open(path.clone()).unwrap();
        assert_eq!(reloaded.snapshot(), created);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn second_create_is_refused() {
        let path = temp_path();
        let store = OrderStore::open(path.clone()).unwrap();
        store.create("1").unwrap();
        assert_eq!(store.create("2").unwrap_err(), CreateError::AlreadyOpen);
        assert_eq!(store.snapshot().amount.as_deref(), Some("1"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rejects_empty_zero_and_too_precise_amounts() {
        assert!(normalize_amount("").is_err());
        assert!(normalize_amount("0").is_err());
        assert!(normalize_amount("0.0").is_err());
        assert!(normalize_amount("1.123456789").is_err());
        assert_eq!(normalize_amount("0.00000001").as_deref(), Ok("0.00000001"));
    }
}
