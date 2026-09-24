//! Google OAuth2 authorization flow, token refresh, and profile fetching.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::config;
use crate::error::{Error, ErrorCode, Result};
use crate::output;
use crate::secrets::{ClientCredentials, Store, StoredTokens};
use crate::util;

pub const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
pub const REVOKE_ENDPOINT: &str = "https://oauth2.googleapis.com/revoke";
pub const PROFILE_ENDPOINT: &str = "https://gmail.googleapis.com/gmail/v1/users/me/profile";

pub fn token_endpoint() -> String {
    std::env::var("GMAIL_TOKEN_URL").unwrap_or_else(|_| TOKEN_ENDPOINT.to_string())
}

pub fn revoke_endpoint() -> String {
    std::env::var("GMAIL_REVOKE_URL").unwrap_or_else(|_| REVOKE_ENDPOINT.to_string())
}

pub fn profile_endpoint() -> String {
    std::env::var("GMAIL_PROFILE_URL").unwrap_or_else(|_| PROFILE_ENDPOINT.to_string())
}

pub struct ScopeProfiles;

impl ScopeProfiles {
    #[allow(dead_code)]
    pub const READ: &'static str = "read";
    pub const DRAFT: &'static str = "draft";
    #[allow(dead_code)]
    pub const NAMES: &'static [&'static str] = &[Self::READ, Self::DRAFT];

    pub fn scopes(profile: &str) -> Result<Vec<&'static str>> {
        match profile.to_lowercase().as_str() {
            "read" => Ok(vec!["https://www.googleapis.com/auth/gmail.readonly"]),
            "draft" => Ok(vec![
                "https://www.googleapis.com/auth/gmail.readonly",
                "https://www.googleapis.com/auth/gmail.compose",
            ]),
            _ => Err(Error::invalid(format!("Unknown scope profile '{profile}'."))
                .fix("Use --scope-profile read or --scope-profile draft.")),
        }
    }

    pub fn can_draft(profile: &str) -> bool {
        profile.eq_ignore_ascii_case(Self::DRAFT)
    }

    pub fn require_draft(account_name: &str, profile: &str) -> Result<()> {
        if Self::can_draft(profile) {
            return Ok(());
        }
        Err(Error::new(
            ErrorCode::AuthRequired,
            format!("Account '{account_name}' was authorized read-only and cannot create drafts."),
        )
        .detail(format!("Its scope profile is '{profile}'."))
        .fix(format!("gmail account reauth {account_name} --scope-profile draft")))
    }
}

pub struct TokenManager;

impl TokenManager {
    /// Returns a usable access token, refreshing under a cross-process lock when needed.
    pub fn get_access_token(account_name: &str, client_ref: &str, store: Store) -> Result<String> {
        let mut tokens = StoredTokens::load(store, account_name)?.ok_or_else(|| {
            Error::new(ErrorCode::AuthRequired, format!("Account '{account_name}' has no stored credentials."))
                .detail("The account is listed in config but its tokens are missing from the keystore.")
                .fix(format!("gmail account reauth {account_name}"))
        })?;

        if tokens.access_token_usable() {
            return Ok(tokens.access_token.unwrap());
        }

        let _lock = config::lock()?;

        // Another process may have refreshed while we waited for the lock
        if let Some(reloaded) = StoredTokens::load(store, account_name)? {
            tokens = reloaded;
            if tokens.access_token_usable() {
                return Ok(tokens.access_token.unwrap());
            }
        }

        let client = ClientCredentials::load(store, client_ref)?;
        let refreshed = Self::refresh(&client, &tokens, account_name)?;
        refreshed.save(store, account_name)?;
        Ok(refreshed.access_token.unwrap_or_default())
    }

    fn refresh(client: &ClientCredentials, tokens: &StoredTokens, account_name: &str) -> Result<StoredTokens> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();

        let form_body = format!(
            "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
            url_encode(&client.client_id),
            url_encode(&client.client_secret),
            url_encode(&tokens.refresh_token),
        );

        let mut resp = agent
            .post(&token_endpoint())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(form_body.as_bytes())
            .map_err(|e| Error::network("Could not reach Google token endpoint.").detail(e.to_string()))?;

        let status = resp.status().as_u16();
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(1024 * 1024)
            .read_to_vec()
            .map_err(|e| Error::network(format!("Failed reading token response: {e}")))?;
        let body = String::from_utf8_lossy(&bytes).to_string();

        if status != 200 {
            let summary = summarize_oauth_error(&body);
            if summary.to_lowercase().contains("invalid_grant") {
                return Err(Error::new(
                    ErrorCode::AuthRequired,
                    format!("Account '{account_name}' needs to be re-authorized."),
                )
                .detail(
                    "Google returned invalid_grant — the refresh token was revoked or has expired. \
                     If this account worked until about a week ago, the OAuth consent screen is probably \
                     still in Testing mode, which expires refresh tokens after 7 days.",
                )
                .fix(format!("gmail account reauth {account_name}")));
            }

            return Err(Error::new(
                ErrorCode::AuthRequired,
                format!("Could not refresh the access token for '{account_name}'."),
            )
            .detail(summary)
            .fix(format!("gmail account reauth {account_name}")));
        }

        let doc: Value = serde_json::from_str(&body)
            .map_err(|e| Error::other("Failed to parse token response JSON.").detail(e.to_string()))?;

        let expires_in = doc.get("expires_in").and_then(Value::as_i64).unwrap_or(3600);
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);

        let new_refresh = doc
            .get("refresh_token")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(&tokens.refresh_token)
            .to_string();

        let access_token = doc.get("access_token").and_then(Value::as_str).map(str::to_string);
        let scope = doc.get("scope").and_then(Value::as_str).map(str::to_string).or_else(|| tokens.scope.clone());

        Ok(StoredTokens {
            refresh_token: new_refresh,
            access_token,
            expires_at: Some((now + expires_in).to_string()),
            scope,
        })
    }

    /// Revokes the grant at Google so removing an account does not leave it live.
    pub fn revoke(tokens: &StoredTokens) -> bool {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .build()
            .into();

        let form = format!("token={}", url_encode(&tokens.refresh_token));
        match agent
            .post(&revoke_endpoint())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(form.as_bytes())
        {
            Ok(resp) => resp.status().as_u16() == 200,
            Err(_) => false,
        }
    }
}

#[allow(dead_code)]
pub struct Profile {
    pub email_address: String,
    pub messages_total: i64,
    pub threads_total: i64,
    pub history_id: i64,
}

pub struct GmailProfile;

impl GmailProfile {
    pub fn fetch(access_token: &str) -> Result<Profile> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .build()
            .into();

        let mut resp = agent
            .get(&profile_endpoint())
            .header("Authorization", &format!("Bearer {access_token}"))
            .call()
            .map_err(|e| Error::network("Could not read the mailbox profile.").detail(e.to_string()))?;

        let status = resp.status().as_u16();
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(1024 * 1024)
            .read_to_vec()
            .map_err(|e| Error::network(format!("Failed reading profile response: {e}")))?;
        let body = String::from_utf8_lossy(&bytes).to_string();

        if status != 200 {
            return Err(Error::auth("Could not read the mailbox profile.")
                .detail(summarize_oauth_error(&body))
                .fix("Check that the Gmail API is enabled in the Google Cloud project."));
        }

        let doc: Value = serde_json::from_str(&body)
            .map_err(|e| Error::other("Failed parsing profile response.").detail(e.to_string()))?;

        let email_address = doc.get("emailAddress").and_then(Value::as_str).unwrap_or("").to_string();
        let messages_total = parse_number(&doc, "messagesTotal");
        let threads_total = parse_number(&doc, "threadsTotal");
        let history_id = parse_number(&doc, "historyId");

        Ok(Profile { email_address, messages_total, threads_total, history_id })
    }
}

fn parse_number(doc: &Value, prop: &str) -> i64 {
    if let Some(val) = doc.get(prop) {
        if let Some(n) = val.as_i64() {
            return n;
        }
        if let Some(s) = val.as_str()
            && let Ok(n) = s.parse::<i64>()
        {
            return n;
        }
    }
    0
}

pub struct OAuthFlow;

impl OAuthFlow {
    pub fn authorize(client: &ClientCredentials, scopes: &[&str], requested_port: u16) -> Result<StoredTokens> {
        let listener = if requested_port > 0 {
            TcpListener::bind(format!("127.0.0.1:{requested_port}")).map_err(|e| {
                Error::other(format!("Could not listen on port {requested_port}."))
                    .detail(e.to_string())
                    .fix("Try a different port with --port, or check whether another process holds it.")
            })?
        } else {
            TcpListener::bind("127.0.0.1:0").map_err(|e| {
                Error::other("Could not bind an ephemeral port for OAuth listener.").detail(e.to_string())
            })?
        };

        let port = listener.local_addr().map_err(|e| Error::other(e.to_string()))?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}/");

        let mut verifier_bytes = [0u8; 32];
        util::random_bytes(&mut verifier_bytes);
        let verifier = util::base64_url_encode(&verifier_bytes);

        let challenge_hash = util::sha256(verifier.as_bytes());
        let challenge = util::base64_url_encode(&challenge_hash);

        let mut state_bytes = [0u8; 16];
        util::random_bytes(&mut state_bytes);
        let state = util::base64_url_encode(&state_bytes);

        let scopes_joined = scopes.join(" ");
        let auth_url = format!(
            "{AUTH_ENDPOINT}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&state={}&access_type=offline&prompt=consent",
            url_encode(&client.client_id),
            url_encode(&redirect_uri),
            url_encode(&scopes_joined),
            url_encode(&challenge),
            url_encode(&state),
        );

        output::status("Opening your browser to authorize this account.");
        output::status("If nothing opens, paste this into a browser on this machine:");
        output::status("");
        output::status(format!("  {auth_url}"));
        output::status("");
        output::status(format!("Listening on {redirect_uri} — waiting up to 3 minutes."));

        try_open_browser(&auth_url);

        let code = wait_for_code(listener, &state, Duration::from_secs(180))?;
        exchange_code(client, &code, &verifier, &redirect_uri)
    }
}

fn wait_for_code(listener: TcpListener, expected_state: &str, timeout: Duration) -> Result<String> {
    listener.set_nonblocking(true).map_err(|e| Error::other(e.to_string()))?;
    let start = Instant::now();

    loop {
        if start.elapsed() > timeout {
            return Err(Error::other("Timed out waiting for the Google consent redirect.")
                .detail("No response arrived on the loopback listener within 3 minutes.")
                .fix("Run the command again and complete the login in the browser."));
        }

        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                return handle_connection(&mut stream, expected_state);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(Error::other(format!("Listener error: {e}"))),
        }
    }
}

fn handle_connection(stream: &mut TcpStream, expected_state: &str) -> Result<String> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| Error::other(e.to_string()))?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).map_err(|e| Error::other(e.to_string()))?;

    // Consume remaining headers
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 || parts[0] != "GET" {
        respond(stream, "Invalid request. You can close this tab.")?;
        return Err(Error::invalid("Non-GET or malformed request to loopback listener."));
    }

    let path = parts[1];
    let query_string = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let query = parse_query(query_string);

    let error = query.get("error");
    let code = query.get("code");
    let state = query.get("state");

    let message = if let Some(err) = error {
        format!("Authorization failed: {err}. You can close this tab.")
    } else {
        "Authorized. You can close this tab and return to the terminal.".to_string()
    };

    respond(stream, &message)?;

    if let Some(err) = error {
        return Err(Error::new(
            ErrorCode::AuthRequired,
            format!("Google returned '{err}' instead of an authorization code."),
        ));
    }

    if state.map(|s| s.as_str()) != Some(expected_state) {
        return Err(Error::other("The OAuth state parameter did not match.").detail(
            "This can indicate a cross-site request forgery attempt, or a stale browser tab from an earlier run.",
        ));
    }

    let Some(code_val) = code else {
        return Err(Error::other("The redirect carried no authorization code."));
    };

    Ok(code_val.clone())
}

fn respond(stream: &mut TcpStream, message: &str) -> Result<()> {
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>gmail CLI</title>\
         <body style=\"font:16px system-ui;margin:80px auto;max-width:32em;color:#14171c\">\
         <h1 style=\"font-size:20px\">{}</h1>\
         </body>",
        html_escape(message)
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
    Ok(())
}

fn exchange_code(client: &ClientCredentials, code: &str, verifier: &str, redirect_uri: &str) -> Result<StoredTokens> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .into();

    let form_body = format!(
        "code={}&client_id={}&client_secret={}&code_verifier={}&redirect_uri={}&grant_type=authorization_code",
        url_encode(code),
        url_encode(&client.client_id),
        url_encode(&client.client_secret),
        url_encode(verifier),
        url_encode(redirect_uri),
    );

    let mut resp = agent
        .post(&token_endpoint())
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(form_body.as_bytes())
        .map_err(|e| Error::auth("Google rejected the token exchange.").detail(e.to_string()))?;

    let status = resp.status().as_u16();
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_vec()
        .map_err(|e| Error::network(format!("Failed reading token exchange response: {e}")))?;
    let body = String::from_utf8_lossy(&bytes).to_string();

    if status != 200 {
        return Err(Error::auth("Google rejected the token exchange.")
            .detail(summarize_oauth_error(&body))
            .fix("Check that the client id and secret from gmail setup belong to a Desktop app OAuth client."));
    }

    let doc: Value = serde_json::from_str(&body)
        .map_err(|e| Error::other("Failed parsing token response.").detail(e.to_string()))?;

    let refresh = doc.get("refresh_token").and_then(Value::as_str).filter(|s| !s.is_empty());
    let Some(refresh_token) = refresh else {
        return Err(Error::auth("Google did not return a refresh token.")
            .detail("This happens when the account has already granted access and consent was not re-requested.")
            .fix("Revoke the app at https://myaccount.google.com/permissions and run the command again."));
    };

    let expires_in = doc.get("expires_in").and_then(Value::as_i64).unwrap_or(3600);
    let now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);

    let access_token = doc.get("access_token").and_then(Value::as_str).map(str::to_string);
    let scope = doc.get("scope").and_then(Value::as_str).map(str::to_string);

    Ok(StoredTokens {
        refresh_token: refresh_token.to_string(),
        access_token,
        expires_at: Some((now + expires_in).to_string()),
        scope,
    })
}

fn try_open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(url).spawn();

    #[cfg(target_os = "linux")]
    let _ = Command::new("xdg-open").arg(url).spawn();

    #[cfg(windows)]
    let _ = Command::new("cmd").args(["/C", "start", "", url]).spawn();
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

fn url_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(qs: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in qs.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            map.insert(url_decode(k), url_decode(v));
        } else if !pair.is_empty() {
            map.insert(url_decode(pair), String::new());
        }
    }
    map
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

pub fn summarize_oauth_error(body: &str) -> String {
    if let Ok(doc) = serde_json::from_str::<Value>(body) {
        let err = doc.get("error").and_then(Value::as_str);
        let desc = doc.get("error_description").and_then(Value::as_str);
        match (err, desc) {
            (Some(e), Some(d)) => return format!("{e}: {d}"),
            (Some(e), None) => return e.to_string(),
            _ => {}
        }
    }
    if body.len() > 400 { body[..400].to_string() } else { body.to_string() }
}
