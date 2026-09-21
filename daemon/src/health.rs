use serde::Serialize;
use serde_json::Value;

use crate::rpc::FiberRpc;

const SHANNONS_PER_CKB: u128 = 100_000_000;

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: String,
    pub seller_rpc: String,
    pub twine_rpc: String,
    pub buyer_rpc: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            listen: env_or("LISTEN", "127.0.0.1:8080"),
            seller_rpc: env_or("SELLER_RPC", "http://127.0.0.1:8227"),
            twine_rpc: env_or("TWINE_RPC", "http://127.0.0.1:8237"),
            buyer_rpc: env_or("BUYER_RPC", "http://127.0.0.1:8247"),
        }
    }
}

fn env_or(name: &str, default: &str) -> String {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => value,
        _ => default.to_string(),
    }
}

#[derive(Serialize)]
pub struct Health {
    pub ready: bool,
    pub nodes: Nodes,
    pub channels: Channels,
}

#[derive(Serialize)]
pub struct Nodes {
    pub seller: NodeHealth,
    pub twine: NodeHealth,
    pub buyer: NodeHealth,
}

#[derive(Serialize)]
pub struct NodeHealth {
    pub rpc: String,
    pub pubkey: Option<String>,
    pub node_name: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct Channels {
    pub seller_to_twine: ChannelHealth,
    pub twine_to_buyer: ChannelHealth,
}

#[derive(Serialize)]
pub struct ChannelHealth {
    pub funder: &'static str,
    pub counterparty: &'static str,
    pub open: bool,
    pub channel_id: Option<String>,
    pub state: Option<String>,
    pub local_balance: Option<String>,
    pub remote_balance: Option<String>,
    pub local_balance_ckb: Option<String>,
    pub remote_balance_ckb: Option<String>,
    pub error: Option<String>,
}

pub async fn health_report(rpc: &FiberRpc, config: &Config) -> Health {
    let (seller_info, twine_info, buyer_info) = tokio::join!(
        rpc.call(&config.seller_rpc, "node_info", Value::Array(vec![])),
        rpc.call(&config.twine_rpc, "node_info", Value::Array(vec![])),
        rpc.call(&config.buyer_rpc, "node_info", Value::Array(vec![])),
    );

    let seller = node_health(&config.seller_rpc, seller_info);
    let twine = node_health(&config.twine_rpc, twine_info);
    let buyer = node_health(&config.buyer_rpc, buyer_info);

    let seller_to_twine = channel_between(
        rpc,
        &config.seller_rpc,
        "seller",
        "twine",
        seller.pubkey.as_deref(),
        twine.pubkey.as_deref(),
    )
    .await;
    let twine_to_buyer = channel_between(
        rpc,
        &config.twine_rpc,
        "twine",
        "buyer",
        twine.pubkey.as_deref(),
        buyer.pubkey.as_deref(),
    )
    .await;

    let ready = seller.pubkey.is_some()
        && twine.pubkey.is_some()
        && buyer.pubkey.is_some()
        && seller_to_twine.open
        && twine_to_buyer.open;

    Health {
        ready,
        nodes: Nodes {
            seller,
            twine,
            buyer,
        },
        channels: Channels {
            seller_to_twine,
            twine_to_buyer,
        },
    }
}

fn node_health(rpc_url: &str, info: Result<Value, String>) -> NodeHealth {
    match info {
        Ok(value) => NodeHealth {
            rpc: rpc_url.to_string(),
            pubkey: value
                .get("pubkey")
                .and_then(Value::as_str)
                .map(str::to_string),
            node_name: value
                .get("node_name")
                .and_then(Value::as_str)
                .map(str::to_string),
            error: None,
        },
        Err(err) => NodeHealth {
            rpc: rpc_url.to_string(),
            pubkey: None,
            node_name: None,
            error: Some(err),
        },
    }
}

async fn channel_between(
    rpc: &FiberRpc,
    funder_rpc: &str,
    funder: &'static str,
    counterparty: &'static str,
    funder_pubkey: Option<&str>,
    counterparty_pubkey: Option<&str>,
) -> ChannelHealth {
    let Some(peer) = counterparty_pubkey else {
        return empty_channel(
            funder,
            counterparty,
            Some(format!("{counterparty} pubkey unavailable")),
        );
    };
    if funder_pubkey.is_none() {
        return empty_channel(
            funder,
            counterparty,
            Some(format!("{funder} pubkey unavailable")),
        );
    }

    let listed = rpc
        .call(funder_rpc, "list_channels", serde_json::json!([{}]))
        .await;
    match listed {
        Ok(value) => match channel_for_peer(&value, peer) {
            Ok(Some(channel)) => channel_health(funder, counterparty, &channel),
            Ok(None) => empty_channel(funder, counterparty, None),
            Err(err) => empty_channel(funder, counterparty, Some(err)),
        },
        Err(err) => empty_channel(funder, counterparty, Some(err)),
    }
}

fn empty_channel(
    funder: &'static str,
    counterparty: &'static str,
    error: Option<String>,
) -> ChannelHealth {
    ChannelHealth {
        funder,
        counterparty,
        open: false,
        channel_id: None,
        state: None,
        local_balance: None,
        remote_balance: None,
        local_balance_ckb: None,
        remote_balance_ckb: None,
        error,
    }
}

fn channel_health(
    funder: &'static str,
    counterparty: &'static str,
    channel: &Value,
) -> ChannelHealth {
    let state = channel
        .get("state")
        .and_then(|state| state.get("state_name"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let local_balance = json_string(channel.get("local_balance"));
    let remote_balance = json_string(channel.get("remote_balance"));
    ChannelHealth {
        funder,
        counterparty,
        open: state.as_deref().is_some_and(is_ready_state),
        channel_id: json_string(channel.get("channel_id")),
        state,
        local_balance_ckb: local_balance.as_deref().and_then(format_ckb),
        remote_balance_ckb: remote_balance.as_deref().and_then(format_ckb),
        local_balance,
        remote_balance,
        error: None,
    }
}

pub fn channel_for_peer(list: &Value, peer_pubkey: &str) -> Result<Option<Value>, String> {
    let channels = list
        .get("channels")
        .and_then(Value::as_array)
        .ok_or_else(|| "list_channels response has no channels array".to_string())?;
    let want = normalize_pubkey(peer_pubkey);
    let matches: Vec<&Value> = channels
        .iter()
        .filter(|channel| is_ckb_channel(channel))
        .filter(|channel| {
            channel
                .get("pubkey")
                .and_then(Value::as_str)
                .is_some_and(|pubkey| normalize_pubkey(pubkey) == want)
        })
        .collect();
    let ready = matches.iter().find(|channel| {
        channel
            .get("state")
            .and_then(|state| state.get("state_name"))
            .and_then(Value::as_str)
            .is_some_and(is_ready_state)
    });
    if let Some(channel) = ready {
        return Ok(Some((*channel).clone()));
    }
    Ok(matches.first().cloned().cloned())
}

fn is_ckb_channel(channel: &Value) -> bool {
    match channel.get("funding_udt_type_script") {
        None | Some(Value::Null) => true,
        _ => false,
    }
}

fn is_ready_state(name: &str) -> bool {
    name == "ChannelReady" || name == "CHANNEL_READY"
}

fn json_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    }
}

pub fn format_ckb(raw: &str) -> Option<String> {
    let shannon = parse_u128(raw)?;
    let whole = shannon / SHANNONS_PER_CKB;
    let frac = shannon % SHANNONS_PER_CKB;
    Some(format!("{whole}.{frac:08}"))
}

fn parse_u128(raw: &str) -> Option<u128> {
    let text = raw.trim();
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        let digits = if hex.is_empty() { "0" } else { hex };
        return u128::from_str_radix(digits, 16).ok();
    }
    text.parse().ok()
}

fn normalize_pubkey(pubkey: &str) -> String {
    pubkey.trim().trim_start_matches("0x").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_one_ckb() {
        assert_eq!(format_ckb("0x5f5e100").as_deref(), Some("1.00000000"));
    }

    #[test]
    fn prefers_ready_ckb_channel_for_the_peer() {
        let list = json!({
            "channels": [
                {
                    "channel_id": "0xpending",
                    "pubkey": "02AA",
                    "funding_udt_type_script": null,
                    "state": {"state_name": "AwaitingChannelReady"},
                    "local_balance": "0x1",
                    "remote_balance": "0x0"
                },
                {
                    "channel_id": "0xudt",
                    "pubkey": "02aa",
                    "funding_udt_type_script": {"code_hash": "0x1"},
                    "state": {"state_name": "ChannelReady"},
                    "local_balance": "0x2",
                    "remote_balance": "0x0"
                },
                {
                    "channel_id": "0xready",
                    "pubkey": "0x02aa",
                    "state": {"state_name": "ChannelReady"},
                    "local_balance": "0x5f5e100",
                    "remote_balance": "0x0"
                }
            ]
        });
        let channel = channel_for_peer(&list, "02aa").unwrap().unwrap();
        assert_eq!(channel["channel_id"], "0xready");
        let health = channel_health("seller", "twine", &channel);
        assert!(health.open);
        assert_eq!(health.local_balance_ckb.as_deref(), Some("1.00000000"));
    }
}
