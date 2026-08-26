//! HTTP client for the local daemon. Sensitive values never hit stdout.

use anyhow::{anyhow, Context, Result};
use interaction_runtime::{ConfigService, Paths};
use serde_json::Value;
use std::path::Path;

pub struct Client {
    pub base: String,
    token: String,
    http: reqwest::Client,
}

/// Stable exit codes.
pub fn exit_code_for_status(status: u16) -> i32 {
    match status {
        200..=299 => 0,
        401 | 403 => 4,
        404 => 5,
        409 | 410 => 6,
        423 => 7,
        _ => 1,
    }
}

pub const EXIT_CONNECTION: i32 = 3;

impl Client {
    pub fn new(home: Option<&Path>, api: Option<String>, token: Option<String>) -> Result<Self> {
        let paths = Paths::resolve(home);
        let config_service = ConfigService::new(paths.clone());
        let base = match api {
            Some(explicit) => explicit,
            None => {
                let config = config_service.load_runtime_config().unwrap_or_default();
                format!("http://{}:{}", config.api_host, config.api_port)
            }
        };
        let token = match token {
            Some(t) => t,
            None => std::fs::read_to_string(paths.token_file())
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
        };
        Ok(Self {
            base,
            token,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .context("build http client")?,
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{}", self.base, path))
            .bearer_auth(&self.token)
    }

    /// Perform a request; returns (status, body). Connection failures map to
    /// a distinct error so callers can exit with EXIT_CONNECTION.
    pub async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(u16, Value)> {
        let mut req = self.request(method, path);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.map_err(|e| {
            if e.is_connect() {
                anyhow!(
                    "daemon offline: cannot reach {} ({e}); start it with `interact-ai serve`",
                    self.base
                )
            } else {
                anyhow!("request failed: {e}")
            }
        })?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        Ok((status, value))
    }

    pub async fn get(&self, path: &str) -> Result<(u16, Value)> {
        self.call(reqwest::Method::GET, path, None).await
    }

    pub async fn post(&self, path: &str, body: Option<Value>) -> Result<(u16, Value)> {
        self.call(reqwest::Method::POST, path, body).await
    }

    pub async fn patch(&self, path: &str, body: Value) -> Result<(u16, Value)> {
        self.call(reqwest::Method::PATCH, path, Some(body)).await
    }

    pub async fn delete(&self, path: &str) -> Result<(u16, Value)> {
        self.call(reqwest::Method::DELETE, path, None).await
    }

    /// Raw SSE tail: prints each event line-by-line for `seconds`.
    pub async fn tail_events(&self, seconds: u64, json_mode: bool) -> Result<()> {
        let resp = self
            .request(reqwest::Method::GET, "/v1/events")
            .send()
            .await
            .map_err(|e| anyhow!("daemon offline: {e}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("events stream refused: {}", resp.status()));
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(seconds);
        let mut stream = resp.bytes_stream();
        use futures::StreamExt;
        let mut buffer = String::new();
        loop {
            let chunk = tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = chunk else { break };
            let chunk = chunk.context("stream read")?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buffer.find("\n\n") {
                let frame: String = buffer.drain(..pos + 2).collect();
                let mut event_type = String::new();
                let mut data = String::new();
                for line in frame.lines() {
                    if let Some(rest) = line.strip_prefix("event: ") {
                        event_type = rest.to_string();
                    } else if let Some(rest) = line.strip_prefix("data: ") {
                        data = rest.to_string();
                    }
                }
                if data.is_empty() {
                    continue;
                }
                if json_mode {
                    println!("{data}");
                } else {
                    println!("[{event_type}] {data}");
                }
            }
        }
        Ok(())
    }
}
