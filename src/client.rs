//! Blocking HTTP client for the Gmail REST API, retry with backoff, and status translation.

use std::time::Duration;

use serde_json::Value;

use crate::account::Resolved;
use crate::auth::TokenManager;
use crate::error::{Error, ErrorCode, Result};
use crate::output;
use crate::secrets::Store;
use crate::util;

pub const DEFAULT_BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me/";

#[derive(Clone)]
pub struct GmailClient {
    pub account_name: String,
    pub account_email: String,
    #[allow(dead_code)]
    pub scope_profile: String,
    pub client_ref: String,
    pub store: Store,
    pub verbose: bool,
    pub timeout_seconds: u64,
    pub base_url: String,
}

#[derive(Clone, Copy, Debug)]
enum Method {
    Get,
    Post,
    Put,
    Delete,
}

impl GmailClient {
    pub fn new(account: &Resolved, store: Store, verbose: bool, timeout_seconds: u64) -> Self {
        let mut base_url = std::env::var("GMAIL_API_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE.to_string());
        if !base_url.ends_with('/') {
            base_url.push('/');
        }

        GmailClient {
            account_name: account.name.clone(),
            account_email: account.email().to_string(),
            scope_profile: account.scope_profile().to_string(),
            client_ref: account.client_ref().to_string(),
            store,
            verbose,
            timeout_seconds,
            base_url,
        }
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        self.send(Method::Get, path, None)
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.send(Method::Post, path, Some(body))
    }

    pub fn put(&self, path: &str, body: &Value) -> Result<Value> {
        self.send(Method::Put, path, Some(body))
    }

    pub fn delete(&self, path: &str) -> Result<()> {
        let _ = self.send(Method::Delete, path, None)?;
        Ok(())
    }

    pub fn get_attachment(&self, message_id: &str, attachment_id: &str) -> Result<Vec<u8>> {
        let doc = self.get(&format!("messages/{message_id}/attachments/{attachment_id}"))?;
        let data = doc.get("data").and_then(Value::as_str).unwrap_or("");
        util::base64_url_decode(data)
    }

    fn send(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value> {
        let max_attempts = 5;
        let mut delay = Duration::from_secs(1);
        let url = format!("{}{}", self.base_url, path);

        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(self.timeout_seconds)))
            .timeout_connect(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .user_agent(concat!("gmail-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();

        for attempt in 1..=max_attempts {
            let token = TokenManager::get_access_token(&self.account_name, &self.client_ref, self.store)?;

            let method_str = match method {
                Method::Get => "GET",
                Method::Post => "POST",
                Method::Put => "PUT",
                Method::Delete => "DELETE",
            };

            if self.verbose {
                output::status(format!("{method_str} {url}"));
            }

            let auth_header = format!("Bearer {token}");
            let mut resp = match (method, body) {
                (Method::Get, _) => {
                    agent.get(&url).header("Authorization", &auth_header).header("Accept", "application/json").call()
                }
                (Method::Delete, _) => {
                    agent.delete(&url).header("Authorization", &auth_header).header("Accept", "application/json").call()
                }
                (Method::Post, Some(b)) => {
                    let json = serde_json::to_vec(b).map_err(|e| Error::other(e.to_string()))?;
                    agent
                        .post(&url)
                        .header("Authorization", &auth_header)
                        .header("Accept", "application/json")
                        .header("Content-Type", "application/json")
                        .send(&json[..])
                }
                (Method::Post, None) => agent
                    .post(&url)
                    .header("Authorization", &auth_header)
                    .header("Accept", "application/json")
                    .send_empty(),
                (Method::Put, Some(b)) => {
                    let json = serde_json::to_vec(b).map_err(|e| Error::other(e.to_string()))?;
                    agent
                        .put(&url)
                        .header("Authorization", &auth_header)
                        .header("Accept", "application/json")
                        .header("Content-Type", "application/json")
                        .send(&json[..])
                }
                (Method::Put, None) => agent
                    .put(&url)
                    .header("Authorization", &auth_header)
                    .header("Accept", "application/json")
                    .send_empty(),
            }
            .map_err(|e| Error::network("Could not reach Google.").detail(e.to_string()))?;

            let status = resp.status().as_u16();
            let bytes = resp
                .body_mut()
                .with_config()
                .limit(64 * 1024 * 1024)
                .read_to_vec()
                .map_err(|e| Error::network(format!("Failed reading response: {e}")))?;
            let resp_text = String::from_utf8_lossy(&bytes).to_string();

            if self.verbose {
                output::status(format!("  -> {status} ({} bytes)", bytes.len()));
            }

            if (200..300).contains(&status) {
                if bytes.iter().all(u8::is_ascii_whitespace) {
                    return Ok(Value::Object(Default::default()));
                }
                return serde_json::from_slice(&bytes)
                    .map_err(|e| Error::other("Failed parsing JSON response.").detail(e.to_string()));
            }

            let is_rate_limit = is_rate_limit_error(status, &resp_text);
            if (status == 429 || status >= 500 || is_rate_limit) && attempt < max_attempts {
                let wait = if let Some(retry_after) = resp.headers().get("Retry-After").and_then(|v| v.to_str().ok()) {
                    retry_after.parse::<u64>().map(Duration::from_secs).unwrap_or(delay)
                } else {
                    let mut jitter_buf = [0u8; 4];
                    util::random_bytes(&mut jitter_buf);
                    let jitter_ms = u32::from_ne_bytes(jitter_buf) as u64 % delay.as_millis().max(1) as u64;
                    Duration::from_millis(jitter_ms)
                };

                if self.verbose {
                    output::status(format!(
                        "  retrying in {:.1}s (attempt {attempt}/{max_attempts})",
                        wait.as_secs_f64()
                    ));
                }

                std::thread::sleep(wait);
                delay = delay.saturating_mul(2);
                continue;
            }

            return Err(self.translate_error(status, &resp_text, attempt));
        }

        Err(Error::new(ErrorCode::RateLimited, format!("Rate limit exceeded after {max_attempts} attempts.")))
    }

    fn translate_error(&self, status: u16, body: &str, attempts: usize) -> Error {
        let detail = extract_api_message(body);

        match status {
            401 => {
                Error::new(ErrorCode::AuthRequired, format!("Account '{}' was rejected by Google.", self.account_name))
                    .detail(detail.unwrap_or_default())
                    .fix(format!("gmail account reauth {}", self.account_name))
            }

            403 if is_rate_limit_body(body) => Error::new(
                ErrorCode::RateLimited,
                format!("Gmail rate limit still exceeded after {attempts} attempts."),
            )
            .detail(detail.unwrap_or_default())
            .fix("Wait a few seconds and retry, or lower --concurrency."),

            403 => Error::new(ErrorCode::AuthRequired, "Gmail refused the request.")
                .detail(detail.unwrap_or_default())
                .fix(format!(
                    "The account may lack the required scope. Try: gmail account reauth {} --scope-profile draft",
                    self.account_name
                )),

            429 => Error::new(
                ErrorCode::RateLimited,
                format!("Gmail rate limit still exceeded after {attempts} attempts."),
            )
            .detail(detail.unwrap_or_default())
            .fix("Wait a few seconds and retry."),

            404 => Error::new(ErrorCode::NotFound, "Not found.").detail(detail.unwrap_or_default()),

            400 => Error::new(ErrorCode::InvalidInput, "Gmail rejected the request as malformed.")
                .detail(detail.unwrap_or_default()),

            _ => Error::new(ErrorCode::Error, format!("Gmail returned {status}.")).detail(detail.unwrap_or_default()),
        }
    }
}

fn is_rate_limit_error(status: u16, body: &str) -> bool {
    status == 429 || (status == 403 && is_rate_limit_body(body))
}

fn is_rate_limit_body(body: &str) -> bool {
    body.contains("rateLimitExceeded") || body.contains("userRateLimitExceeded")
}

fn extract_api_message(body: &str) -> Option<String> {
    if body.trim().is_empty() {
        return None;
    }
    if let Ok(doc) = serde_json::from_str::<Value>(body)
        && let Some(err) = doc.get("error")
        && let Some(msg) = err.get("message").and_then(Value::as_str)
    {
        return Some(msg.to_string());
    }
    if body.len() > 400 { Some(body[..400].to_string()) } else { Some(body.to_string()) }
}
