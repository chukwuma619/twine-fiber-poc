use serde::Serialize;
use serde_json::{json, Value};

use crate::rpc::FiberRpc;

const SHANNONS_PER_CKB: u128 = 100_000_000;
const DEFAULT_FUNDING_SHANNONS: u128 = 50_000_000_000;

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: String,
    pub twine_rpc: String,
    pub twine_p2p: String,
    pub funding_shannons: u128,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            listen: env_or("LISTEN", "127.0.0.1:8080"),
            twine_rpc: env_or("TWINE_RPC", "http://127.0.0.1:8237"),
            twine_p2p: env_or("TWINE_P2P", "/ip4/127.0.0.1/tcp/8238"),
            funding_shannons: parse_u128(&env_or(
                "FUNDING_SHANNONS",
                &DEFAULT_FUNDING_SHANNONS.to_string(),
            ))
            .unwrap_or(DEFAULT_FUNDING_SHANNONS),
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
    pub twine: NodeHealth,
}

#[derive(Serialize)]
pub struct NodeHealth {
    pub rpc: String,
    pub pubkey: Option<String>,
    pub node_name: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct TwineInfo {
    pub rpc: String,
    pub pubkey: Option<String>,
    pub p2p_address: String,
    pub node_name: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct ConnectResult {
    pub connected: bool,
    pub channel_open: bool,
    pub channel_id: Option<String>,
    pub message: String,
}

pub async fn health_report(rpc: &FiberRpc, config: &Config) -> Health {
    let twine_info = rpc
        .call(&config.twine_rpc, "node_info", Value::Array(vec![]))
        .await;
    let twine = node_health(&config.twine_rpc, twine_info);
    Health {
        ready: twine.pubkey.is_some(),
        twine,
    }
}

pub async fn twine_info(rpc: &FiberRpc, config: &Config) -> TwineInfo {
    let info = rpc
        .call(&config.twine_rpc, "node_info", Value::Array(vec![]))
        .await;
    match info {
        Ok(value) => TwineInfo {
            rpc: config.twine_rpc.clone(),
            pubkey: value
                .get("pubkey")
                .and_then(Value::as_str)
                .map(str::to_string),
            p2p_address: config.twine_p2p.clone(),
            node_name: value
                .get("node_name")
                .and_then(Value::as_str)
                .map(str::to_string),
            error: None,
        },
        Err(err) => TwineInfo {
            rpc: config.twine_rpc.clone(),
            pubkey: None,
            p2p_address: config.twine_p2p.clone(),
            node_name: None,
            error: Some(err),
        },
    }
}

pub async fn connect_user(
    rpc: &FiberRpc,
    config: &Config,
    pubkey: &str,
    address: &str,
) -> Result<ConnectResult, String> {
    let pubkey = pubkey.trim();
    let address = address.trim();
    if pubkey.is_empty() {
        return Err("pubkey is required".into());
    }
    if address.is_empty() {
        return Err("address is required".into());
    }

    match rpc
        .call(
            &config.twine_rpc,
            "connect_peer",
            json!([{
                "pubkey": pubkey,
                "address": address,
                "save": true,
            }]),
        )
        .await
    {
        Ok(_) => {}
        Err(err) if already_connected(&err) => {}
        Err(err) => return Err(err),
    }

    let listed = rpc
        .call(&config.twine_rpc, "list_channels", json!([{}]))
        .await?;
    if let Some(channel) = channel_for_peer(&listed, pubkey)? {
        let channel_id = json_string(channel.get("channel_id"));
        let ready = channel
            .get("state")
            .and_then(|state| state.get("state_name"))
            .and_then(Value::as_str)
            .is_some_and(is_ready_state);
        return Ok(ConnectResult {
            connected: true,
            channel_open: ready,
            channel_id,
            message: if ready {
                "Twine already has a ready channel to this node".into()
            } else {
                "Twine is already opening a channel to this node".into()
            },
        });
    }

    rpc.call(
        &config.twine_rpc,
        "open_channel",
        json!([{
            "pubkey": pubkey,
            "funding_amount": format!("0x{:x}", config.funding_shannons),
            "public": true,
        }]),
    )
    .await?;

    let listed = rpc
        .call(&config.twine_rpc, "list_channels", json!([{}]))
        .await
        .ok();
    let channel_id = listed
        .as_ref()
        .and_then(|value| channel_for_peer(value, pubkey).ok().flatten())
        .and_then(|channel| json_string(channel.get("channel_id")));

    Ok(ConnectResult {
        connected: true,
        channel_open: true,
        channel_id,
        message: "Twine connected and opened a channel toward this node".into(),
    })
}

fn already_connected(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("already") || lower.contains("connected")
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
    }
}
