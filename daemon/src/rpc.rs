use serde_json::{json, Value};

#[derive(Clone)]
pub struct FiberRpc {
    http: reqwest::Client,
}

impl FiberRpc {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("reqwest client");
        Self { http }
    }

    pub async fn call(&self, url: &str, method: &str, params: Value) -> Result<Value, String> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });
        let response = self
            .http
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|err| err.to_string())?;
        let status = response.status();
        let value: Value = response.json().await.map_err(|err| err.to_string())?;
        if let Some(err) = value.get("error") {
            let message = err
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| err.to_string());
            return Err(message);
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| format!("fnn {method} returned no result (http {status})"))
    }
}
