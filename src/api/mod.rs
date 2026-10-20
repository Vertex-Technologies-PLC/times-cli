pub mod envelope;
pub mod types;

use std::time::Duration;

use reqwest::{Method, RequestBuilder};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::error::CliError;

pub const DEFAULT_API_URL: &str = "https://api.times.et";
const USER_AGENT: &str = concat!("times-cli/", env!("CARGO_PKG_VERSION"));

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
}

impl Client {
    pub fn new(base: &str) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .build()?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
        })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    pub async fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> anyhow::Result<T> {
        let mut req: RequestBuilder = self
            .http
            .request(method, self.url(path))
            .timeout(Duration::from_secs(90))
            .header("Accept", "application/json");
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        if !query.is_empty() {
            req = req.query(query);
        }
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(network_error)?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(network_error)?;
        let data = envelope::parse(status, &text)?;
        serde_json::from_value(data).map_err(|e| {
            CliError::Api {
                status,
                code: "UNEXPECTED_RESPONSE".into(),
                message: format!("The API returned a response the CLI does not understand: {e}"),
                details: None,
            }
            .into()
        })
    }

    pub async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        token: Option<&str>,
        query: &[(&str, String)],
    ) -> anyhow::Result<T> {
        self.send(Method::GET, path, token, query, None).await
    }

    pub async fn post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        token: Option<&str>,
        body: &B,
    ) -> anyhow::Result<T> {
        let body = serde_json::to_value(body)?;
        self.send(Method::POST, path, token, &[], Some(&body)).await
    }

    pub async fn patch<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        token: Option<&str>,
        body: &B,
    ) -> anyhow::Result<T> {
        let body = serde_json::to_value(body)?;
        self.send(Method::PATCH, path, token, &[], Some(&body))
            .await
    }

    pub async fn put<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        token: Option<&str>,
        body: &B,
    ) -> anyhow::Result<T> {
        let body = serde_json::to_value(body)?;
        self.send(Method::PUT, path, token, &[], Some(&body)).await
    }

    pub async fn delete<T: DeserializeOwned>(
        &self,
        path: &str,
        token: Option<&str>,
    ) -> anyhow::Result<T> {
        self.send(Method::DELETE, path, token, &[], None).await
    }
}

pub fn network_error(e: reqwest::Error) -> anyhow::Error {
    let msg = if e.is_timeout() {
        "The request timed out.".to_string()
    } else if e.is_connect() {
        "Could not connect to the TIMES API.".to_string()
    } else {
        format!("Network error: {e}")
    };
    CliError::Network(msg).into()
}

pub fn seg(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
