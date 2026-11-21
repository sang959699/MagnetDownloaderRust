use crate::models::AriaAddUri;
use serde_json::json;
use std::collections::HashMap;

/// Minimal async client for aria2's JSON-RPC API, enough for `aria2.addUri`.
pub struct Aria2Client {
    rpc_url: String,
    rpc_token: String,
    http: reqwest::Client,
}

impl Aria2Client {
    pub fn new(rpc_url: String, rpc_token: String) -> Self {
        Self {
            rpc_url,
            rpc_token,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
        }
    }

    /// Add a magnet URI to aria2 for download into `dir`. Returns true on a
    /// successful JSON-RPC response (no error field in the reply).
    pub async fn add_uri(&self, uri: &str, dir: Option<&str>) -> Result<bool, String> {
        let mut params: Vec<serde_json::Value> = Vec::new();
        params.push(json!({ "token": self.rpc_token }));
        params.push(json!([uri]));

        let mut opts: HashMap<String, String> = HashMap::new();
        if let Some(d) = dir {
            opts.insert("dir".to_string(), d.to_string());
        }
        params.push(serde_json::to_value(opts).map_err(|e| e.to_string())?);

        let body = AriaAddUri {
            jsonrpc: "2.0",
            method: "aria2.addUri",
            id: "",
            params,
        };

        let resp = self
            .http
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("HTTP {}: {}", status, text));
        }

        // aria2 returns { "result": "gid" } on success and { "error": {...} }
        // on failure. We check for the presence of an error object.
        let v: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("bad response JSON: {}", e))?;
        if v.get("error").is_some() {
            return Err(format!("aria2 error: {}", v["error"]));
        }
        if v.get("result").is_none() {
            return Err(format!("unexpected response: {}", text));
        }
        Ok(true)
    }
}
