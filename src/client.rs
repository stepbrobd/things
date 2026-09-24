use std::{collections::BTreeMap, fmt, time::Instant};

use anyhow::{Context, Result, anyhow};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use tracing::debug;
use urlencoding::encode;

use crate::{common::one_line, wire::wire_object::WireObject};

const BASE_URL: &str = "https://cloud.culturedcode.com/version/1";
const USER_AGENT: &str = "ThingsMac/32209501";
const CLIENT_INFO: &str = "eyJkbSI6Ik1hYzE0LDIiLCJsciI6IlVTIiwibmYiOnRydWUsIm5rIjp0cnVlLCJubiI6IlRoaW5nc01hYyIsIm52IjoiMzIyMDk1MDEiLCJvbiI6Im1hY09TIiwib3YiOiIyNi4zLjAiLCJwbCI6ImVuLVVTIiwidWwiOiJlbi1MYXRuLVVTIn0=";
const APP_ID: &str = "com.culturedcode.ThingsMac";
const SCHEMA: &str = "301";
const WRITE_PUSH_PRIORITY: &str = "10";
const APP_INSTANCE_ID: &str = "things";

/// an answer with an error status
///
/// a caller tells it apart from a request that never reached the server
#[derive(Debug)]
pub struct HttpStatus {
    pub status: u16,
    label: String,
    body: String,
}

impl fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HTTP {} for {}: {}", self.status, self.label, self.body)
    }
}

impl std::error::Error for HttpStatus {}

impl HttpStatus {
    /// the answer as a message shows it, capped and on one line
    fn new(status: u16, label: &str, text: &str) -> Self {
        Self {
            status,
            label: label.to_string(),
            body: one_line(&text.chars().take(300).collect::<String>()),
        }
    }
}

pub struct ThingsCloudClient {
    pub email: String,
    pub password: String,
    pub history_key: Option<String>,
    pub head_index: i64,
    http: Client,
}

impl ThingsCloudClient {
    pub fn new(email: String, password: String) -> Result<Self> {
        let http = Client::builder().build()?;
        Ok(Self {
            email,
            password,
            history_key: None,
            head_index: 0,
            http,
        })
    }

    /// `label` names the request in messages
    ///
    /// the url stays out of them
    /// the history key in it alone reads and writes the account
    fn request(
        &self,
        method: reqwest::Method,
        url: &str,
        label: &str,
        body: Option<Value>,
        extra_headers: &[(&str, String)],
    ) -> Result<Value> {
        let mut req = self
            .http
            .request(method, url)
            .header("Accept", "application/json")
            .header("Accept-Charset", "UTF-8")
            .header("User-Agent", USER_AGENT)
            .header("things-client-info", CLIENT_INFO)
            .header("App-Id", APP_ID)
            .header("Schema", SCHEMA)
            .header("App-Instance-Id", APP_INSTANCE_ID);

        for (k, v) in extra_headers {
            req = req.header(*k, v);
        }

        if let Some(payload) = body {
            req = req
                .header("Content-Type", "application/json; charset=UTF-8")
                .header("Content-Encoding", "UTF-8")
                .json(&payload);
        }

        let started = Instant::now();
        let resp = req
            .send()
            .map_err(reqwest::Error::without_url)
            .with_context(|| format!("request failed: {label}"))?;
        let status = resp.status();
        let text = resp
            .text()
            .map_err(reqwest::Error::without_url)
            .with_context(|| {
                format!(
                    "failed reading body from {label} (HTTP {})",
                    status.as_u16()
                )
            })?;
        debug!(target: "things::cloud", request = label, status = status.as_u16(), elapsed = ?started.elapsed(), "answered");
        if !status.is_success() {
            return Err(HttpStatus::new(status.as_u16(), label, &text).into());
        }
        if text.trim().is_empty() {
            return Ok(json!({}));
        }
        serde_json::from_str(&text).with_context(|| format!("invalid json from {label}"))
    }

    pub fn authenticate(&mut self) -> Result<String> {
        let url = format!("{BASE_URL}/account/{}", encode(&self.email));
        let result = self.request(
            reqwest::Method::GET,
            &url,
            "the account",
            None,
            &[(
                "Authorization",
                format!("Password {}", encode(&self.password)),
            )],
        )?;
        let key = result
            .get("history-key")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing history-key in auth response"))?
            .to_string();
        self.history_key = Some(key.clone());
        Ok(key)
    }

    pub fn get_items_page(&self, start_index: i64) -> Result<Value> {
        let history_key = self
            .history_key
            .as_ref()
            .ok_or_else(|| anyhow!("Must authenticate first"))?;
        let url = format!("{BASE_URL}/history/{history_key}/items?start-index={start_index}");
        self.request(
            reqwest::Method::GET,
            &url,
            &format!("the history items from {start_index}"),
            None,
            &[],
        )
    }

    pub fn commit(&mut self, changes: BTreeMap<String, WireObject>) -> Result<i64> {
        let history_key = self
            .history_key
            .as_ref()
            .ok_or_else(|| anyhow!("Must authenticate first"))?;
        let idx = self.head_index;
        let url = format!("{BASE_URL}/history/{history_key}/commit?ancestor-index={idx}&_cnt=1");

        let result = self.request(
            reqwest::Method::POST,
            &url,
            &format!("the commit on {idx}"),
            Some(serde_json::to_value(changes)?),
            &[("Push-Priority", WRITE_PUSH_PRIORITY.to_string())],
        )?;

        let new_index = result
            .get("server-head-index")
            .and_then(Value::as_i64)
            .unwrap_or(idx);
        self.head_index = new_index;
        Ok(new_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_body_stays_on_one_line() {
        let shown = HttpStatus::new(
            401,
            "the account",
            "denied\n2026-09-24T08:00:00Z  WARN things::replay: forged\r\u{1b}[8m",
        )
        .to_string();
        assert!(!shown.contains(['\n', '\r', '\u{1b}']), "{shown:?}");
    }
}
