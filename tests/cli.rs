//! In-process integration test suite for the `gmail` CLI binary.
//!
//! Uses an in-process mock HTTP server, isolated temp directories, and the plaintext store
//! so tests run completely offline and never touch real accounts, keystores, or networks.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct Recorded {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Option<Value>,
}

fn b64url(data: &[u8]) -> String {
    const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i];
        let b1 = if i + 1 < data.len() { data[i + 1] } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] } else { 0 };
        out.push(B64[(b0 >> 2) as usize] as char);
        out.push(B64[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < data.len() {
            out.push(B64[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize] as char);
        }
        if i + 2 < data.len() {
            out.push(B64[(b2 & 0x3F) as usize] as char);
        }
        i += 3;
    }
    out.replace('+', "-").replace('/', "_")
}

#[derive(Clone)]
struct Route {
    method: &'static str,
    path_prefix: &'static str,
    status: u16,
    body: Value,
    headers: Vec<(&'static str, &'static str)>,
}

impl Route {
    fn new(method: &'static str, path_prefix: &'static str, status: u16, body: Value) -> Self {
        Route { method, path_prefix, status, body, headers: Vec::new() }
    }

    fn with_header(mut self, name: &'static str, val: &'static str) -> Self {
        self.headers.push((name, val));
        self
    }
}

struct Mock {
    url: String,
    log: Arc<Mutex<Vec<Recorded>>>,
}

impl Mock {
    fn start(routes: Vec<Route>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}");
        let log = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let routes = routes.clone();
                let log = log2.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = line.split_whitespace();
                    let method = parts.next().unwrap_or("").to_string();
                    let full_path = parts.next().unwrap_or("").to_string();

                    let mut headers = Vec::new();
                    let mut len = 0usize;
                    loop {
                        let mut h = String::new();
                        reader.read_line(&mut h).unwrap();
                        let h = h.trim_end();
                        if h.is_empty() {
                            break;
                        }
                        if let Some((k, v)) = h.split_once(':') {
                            let (k, v) = (k.trim().to_lowercase(), v.trim().to_string());
                            if k == "content-length" {
                                len = v.parse().unwrap_or(0);
                            }
                            headers.push((k, v));
                        }
                    }

                    let mut buf = vec![0; len];
                    if len > 0 {
                        reader.read_exact(&mut buf).unwrap();
                    }
                    let body = (len > 0).then(|| serde_json::from_slice(&buf).unwrap_or(Value::Null));

                    log.lock().unwrap().push(Recorded {
                        method: method.clone(),
                        path: full_path.clone(),
                        headers,
                        body,
                    });

                    let matched = routes.iter().find(|r| {
                        r.method == method
                            && (full_path == r.path_prefix
                                || full_path.starts_with(r.path_prefix)
                                || full_path.trim_start_matches("/gmail/v1/users/me/").starts_with(r.path_prefix))
                    });

                    let (status, resp_body, extra_headers) = if let Some(r) = matched {
                        (r.status, r.body.clone(), r.headers.clone())
                    } else {
                        (
                            404,
                            json!({"error": {"message": format!("No route found for {method} {full_path}")}}),
                            Vec::new(),
                        )
                    };

                    let text = if status == 204 { String::new() } else { resp_body.to_string() };

                    let mut hdr_str = String::new();
                    for (k, v) in extra_headers {
                        hdr_str.push_str(&format!("{k}: {v}\r\n"));
                    }

                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\n{hdr_str}Content-Length: {}\r\nConnection: close\r\n\r\n{text}",
                        text.len()
                    );
                });
            }
        });

        Mock { url, log }
    }

    #[allow(dead_code)]
    fn requests(&self) -> Vec<Recorded> {
        self.log.lock().unwrap().clone()
    }
}

struct Env {
    dir: PathBuf,
    mock_url: String,
}

impl Env {
    fn new(mock: &Mock) -> Env {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "gmail-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Env { dir, mock_url: mock.url.clone() }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_gmail"))
            .args(args)
            .env("GMAIL_CONFIG_DIR", &self.dir)
            .env("GMAIL_SECRET_STORE", "plaintext")
            .env("GMAIL_ALLOW_PLAINTEXT_STORE", "1")
            .env("GMAIL_API_URL", format!("{}/gmail/v1/users/me/", self.mock_url))
            .env("GMAIL_TOKEN_URL", format!("{}/oauth2/token", self.mock_url))
            .env("GMAIL_PROFILE_URL", format!("{}/gmail/v1/users/me/profile", self.mock_url))
            .env("GMAIL_REVOKE_URL", format!("{}/oauth2/revoke", self.mock_url))
            .env("GMAIL_CLOCK_URL", format!("{}/oauth2/clock", self.mock_url))
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> (i32, Value, Value) {
        let mut all = args.to_vec();
        all.push("--json");
        let out = self.run(&all);
        let parse = |b: &[u8]| {
            let s = String::from_utf8_lossy(b);
            let filtered = s.lines().filter(|l| !l.starts_with("warning:")).collect::<Vec<_>>().join("\n");
            serde_json::from_str(&filtered).unwrap_or(Value::Null)
        };
        (out.status.code().unwrap_or(-1), parse(&out.stdout), parse(&out.stderr))
    }

    fn with_account(self, name: &str, email: &str, scope_profile: &str) -> Env {
        let config_yaml = format!(
            "version: 1\naccounts:\n  {name}:\n    email: {email}\n    scopeProfile: {scope_profile}\n    clientRef: default\n    addedAt: '2026-01-01T00:00:00Z'\n"
        );
        std::fs::write(self.dir.join("config.yaml"), config_yaml).unwrap();

        let mut secrets_map = HashMap::new();
        secrets_map.insert(
            "client:default".to_string(),
            json!({"clientId": "test-client-id", "clientSecret": "test-client-secret"}).to_string(),
        );

        let tokens = json!({
            "refreshToken": "test-refresh-token",
            "accessToken": "test-access-token",
            "expiresAt": "2099-01-01T00:00:00Z",
            "scope": "https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose"
        });
        secrets_map.insert(format!("account:{name}"), tokens.to_string());

        std::fs::write(self.dir.join("secrets.json"), serde_json::to_string(&secrets_map).unwrap()).unwrap();

        self
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// =========================================================================
// Tests
// =========================================================================

#[test]
fn test_agent_readme_markdown() {
    let mock = Mock::start(vec![]);
    let env = Env::new(&mock);
    let out = env.run(&["agent-readme"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# gmail — agent operating manual"));
    assert!(stdout.contains("The two rules that matter"));
    assert!(stdout.contains("Drafts are never sent."));
}

#[test]
fn test_agent_readme_json() {
    let mock = Mock::start(vec![]);
    let env = Env::new(&mock);
    let (code, out, _) = env.json(&["agent-readme", "--format", "json"]);
    assert_eq!(code, 0);
    assert_eq!(out["tool"], "gmail");
    assert_eq!(out["apiVersion"], "v1");
    assert!(out["rules"].is_array());
    assert_eq!(out["exitCodes"]["0"], "ok");
    assert_eq!(out["exitCodes"]["3"], "auth_required — stop, surface the remediation to a human");
}

#[test]
fn test_setup_show() {
    let mock = Mock::start(vec![]);
    let env = Env::new(&mock);
    let out = env.run(&["setup", "--show"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Setting up Gmail API access"));
}

#[test]
fn test_setup_scripted() {
    let mock = Mock::start(vec![]);
    let env = Env::new(&mock);
    let (code, out, _) = env.json(&["setup", "--client-id", "custom-client-id", "--client-secret", "custom-secret"]);
    assert_eq!(code, 0);
    assert_eq!(out["status"], "configured");
    assert_eq!(out["clientId"], "custom-client-id");
}

#[test]
fn test_account_list_and_test() {
    let mock = Mock::start(vec![Route::new(
        "GET",
        "/gmail/v1/users/me/profile",
        200,
        json!({
            "emailAddress": "work@example.com",
            "messagesTotal": 42,
            "threadsTotal": 10,
            "historyId": 1001
        }),
    )]);
    let env = Env::new(&mock).with_account("work", "work@example.com", "draft");

    // Account list
    let (code, out, _) = env.json(&["account", "list"]);
    assert_eq!(code, 0);
    assert_eq!(out["count"], 1);
    assert_eq!(out["accounts"][0]["name"], "work");
    assert_eq!(out["accounts"][0]["email"], "work@example.com");

    // Account test
    let (code, out, _) = env.json(&["account", "test", "work"]);
    assert_eq!(code, 0);
    assert_eq!(out["tokenStatus"], "valid");
    assert_eq!(out["name"], "work");
    assert_eq!(out["email"], "work@example.com");
    assert_eq!(out["messagesTotal"], 42);
}

#[test]
fn test_account_remove() {
    let mock = Mock::start(vec![]);
    let env = Env::new(&mock).with_account("work", "work@example.com", "draft");

    let (code, out, _) = env.json(&["account", "remove", "work", "--local-only", "--yes"]);
    assert_eq!(code, 0);
    assert_eq!(out["status"], "removed");
    assert_eq!(out["name"], "work");

    // Verify it is gone
    let (code, out, _) = env.json(&["account", "list"]);
    assert_eq!(code, 0);
    assert_eq!(out["count"], 0);
}

#[test]
fn test_search_and_hydration() {
    let mock = Mock::start(vec![
        Route::new(
            "GET",
            "messages?maxResults=20&q=is%3Aunread",
            200,
            json!({
                "messages": [
                    {"id": "msg1", "threadId": "th1"},
                    {"id": "msg2", "threadId": "th2"}
                ],
                "resultSizeEstimate": 2
            }),
        ),
        Route::new(
            "GET",
            "messages/msg1?format=full",
            200,
            json!({
                "id": "msg1",
                "threadId": "th1",
                "snippet": "Test snippet 1",
                "internalDate": "1700000000000",
                "payload": {
                    "headers": [
                        {"name": "From", "value": "alice@example.com"},
                        {"name": "To", "value": "work@example.com"},
                        {"name": "Subject", "value": "Invoice #1"}
                    ]
                }
            }),
        ),
        Route::new(
            "GET",
            "messages/msg2?format=full",
            200,
            json!({
                "id": "msg2",
                "threadId": "th2",
                "snippet": "Test snippet 2",
                "internalDate": "1700000050000",
                "payload": {
                    "headers": [
                        {"name": "From", "value": "bob@example.com"},
                        {"name": "To", "value": "work@example.com"},
                        {"name": "Subject", "value": "Invoice #2"}
                    ]
                }
            }),
        ),
    ]);
    let env = Env::new(&mock).with_account("work", "work@example.com", "draft");

    let (code, out, _) = env.json(&["search", "is:unread", "-a", "work"]);
    assert_eq!(code, 0);
    assert_eq!(out["account"], "work");
    assert_eq!(out["count"], 2);
    assert_eq!(out["messages"][0]["id"], "msg1");
    assert_eq!(out["messages"][0]["from"], "alice@example.com");
    assert_eq!(out["messages"][0]["subject"], "Invoice #1");
    assert_eq!(out["messages"][1]["id"], "msg2");
    assert_eq!(out["messages"][1]["from"], "bob@example.com");
    assert_eq!(out["messages"][1]["subject"], "Invoice #2");
}

#[test]
fn test_message_get_and_raw_save() {
    let raw_email = concat!(
        "From: sender@example.com\r\n",
        "To: work@example.com\r\n",
        "Subject: Meeting Notes\r\n",
        "Date: Wed, 15 Nov 2023 12:00:00 +0000\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n\r\n",
        "Here are the meeting notes from today.\r\n"
    );
    let base64_raw = b64url(raw_email.as_bytes());

    let mock = Mock::start(vec![Route::new(
        "GET",
        "messages/msg999?format=raw",
        200,
        json!({
            "id": "msg999",
            "threadId": "th999",
            "raw": base64_raw
        }),
    )]);
    let env = Env::new(&mock).with_account("work", "work@example.com", "draft");
    let raw_file = env.dir.join("saved_msg.eml");

    let (code, out, _) =
        env.json(&["message", "get", "msg999", "-a", "work", "--save-raw", raw_file.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_eq!(out["id"], "msg999");
    assert_eq!(out["subject"], "Meeting Notes");
    assert_eq!(out["from"], "sender@example.com");
    assert!(out["body"].as_str().unwrap().contains("Here are the meeting notes"));

    // Verify saved file
    assert!(raw_file.exists());
    let saved_content = std::fs::read_to_string(&raw_file).unwrap();
    assert_eq!(saved_content, raw_email);
}

#[test]
fn test_message_attachments_and_download() {
    let mock = Mock::start(vec![
        Route::new(
            "GET",
            "messages/msg_att?format=full",
            200,
            json!({
                "id": "msg_att",
                "threadId": "th_att",
                "payload": {
                    "mimeType": "multipart/mixed",
                    "parts": [
                        {
                            "filename": "report.pdf",
                            "mimeType": "application/pdf",
                            "body": {
                                "attachmentId": "att_123",
                                "size": 13
                            }
                        }
                    ]
                }
            }),
        ),
        Route::new(
            "GET",
            "messages/msg_att/attachments/att_123",
            200,
            json!({
                "size": 13,
                "data": b64url(b"PDF-Mock-Data")
            }),
        ),
    ]);
    let env = Env::new(&mock).with_account("work", "work@example.com", "draft");

    // List attachments
    let (code, out, _) = env.json(&["message", "attachments", "msg_att", "-a", "work"]);
    assert_eq!(code, 0);
    assert_eq!(out["count"], 1);
    assert_eq!(out["attachments"][0]["filename"], "report.pdf");
    assert_eq!(out["attachments"][0]["attachmentId"], "att_123");

    // Download attachments
    let out_dir = env.dir.join("downloads");
    let (code, out, _) =
        env.json(&["attachment", "download", "msg_att", "-a", "work", "--all", "--out-dir", out_dir.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_eq!(out["savedCount"], 1);
    assert_eq!(out["saved"][0]["filename"], "report.pdf");

    let downloaded_file = out_dir.join("report.pdf");
    assert!(downloaded_file.exists());
    assert_eq!(std::fs::read(&downloaded_file).unwrap(), b"PDF-Mock-Data");
}

#[test]
fn test_thread_get() {
    let raw1 = concat!(
        "From: alice@example.com\r\n",
        "To: work@example.com\r\n",
        "Subject: Project Alpha\r\n\r\n",
        "Message 1 in thread\r\n"
    );
    let raw2 = concat!(
        "From: work@example.com\r\n",
        "To: alice@example.com\r\n",
        "Subject: Re: Project Alpha\r\n\r\n",
        "Message 2 in thread\r\n"
    );

    let mock = Mock::start(vec![
        Route::new(
            "GET",
            "threads/th_alpha?format=minimal",
            200,
            json!({
                "id": "th_alpha",
                "messages": [
                    {"id": "m1"},
                    {"id": "m2"}
                ]
            }),
        ),
        Route::new(
            "GET",
            "messages/m1?format=raw",
            200,
            json!({
                "id": "m1",
                "threadId": "th_alpha",
                "raw": b64url(raw1.as_bytes())
            }),
        ),
        Route::new(
            "GET",
            "messages/m2?format=raw",
            200,
            json!({
                "id": "m2",
                "threadId": "th_alpha",
                "raw": b64url(raw2.as_bytes())
            }),
        ),
    ]);
    let env = Env::new(&mock).with_account("work", "work@example.com", "draft");

    let (code, out, _) = env.json(&["thread", "get", "th_alpha", "-a", "work"]);
    assert_eq!(code, 0);
    assert_eq!(out["threadId"], "th_alpha");
    assert_eq!(out["messageCount"], 2);
    assert_eq!(out["returned"], 2);
    assert_eq!(out["messages"][0]["from"], "alice@example.com");
    assert_eq!(out["messages"][1]["from"], "work@example.com");
}

#[test]
fn test_draft_create_list_get_update_delete() {
    let mock = Mock::start(vec![
        Route::new(
            "POST",
            "drafts",
            200,
            json!({
                "id": "draft_new_1",
                "message": {
                    "id": "msg_draft_1",
                    "threadId": "th_draft_1"
                }
            }),
        ),
        Route::new(
            "GET",
            "drafts?maxResults=20",
            200,
            json!({
                "drafts": [
                    {
                        "id": "draft_new_1",
                        "message": {"id": "msg_draft_1"}
                    }
                ]
            }),
        ),
        Route::new(
            "GET",
            "messages/msg_draft_1?format=metadata",
            200,
            json!({
                "id": "msg_draft_1",
                "internalDate": "1700000000000",
                "payload": {
                    "headers": [
                        {"name": "To", "value": "recipient@example.com"},
                        {"name": "Subject", "value": "Test Subject"}
                    ]
                }
            }),
        ),
        Route::new(
            "GET",
            "drafts/draft_new_1?format=raw",
            200,
            json!({
                "id": "draft_new_1",
                "message": {
                    "id": "msg_draft_1",
                    "threadId": "th_draft_1",
                    "raw": b64url(concat!(
                        "From: work@example.com\r\n",
                        "To: recipient@example.com\r\n",
                        "Subject: Test Subject\r\n\r\n",
                        "Original Draft Body\r\n"
                    ).as_bytes())
                }
            }),
        ),
        Route::new(
            "PUT",
            "drafts/draft_new_1",
            200,
            json!({
                "id": "draft_new_1",
                "message": {
                    "id": "msg_draft_1",
                    "threadId": "th_draft_1"
                }
            }),
        ),
        Route::new("DELETE", "drafts/draft_new_1", 204, Value::Null),
    ]);
    let env = Env::new(&mock).with_account("work", "work@example.com", "draft");

    // 1. Create draft
    let (code, out, _) = env.json(&[
        "draft",
        "create",
        "-a",
        "work",
        "--to",
        "recipient@example.com",
        "--subject",
        "Test Subject",
        "--body-text",
        "Hello from draft!",
    ]);
    assert_eq!(code, 0);
    assert_eq!(out["status"], "draft_created_not_sent");
    assert_eq!(out["draftId"], "draft_new_1");
    assert_eq!(out["to"][0], "recipient@example.com");
    assert_eq!(out["webUrl"], "https://mail.google.com/mail/u/0/#drafts?compose=draft_new_1");

    // 2. List drafts
    let (code, out, _) = env.json(&["draft", "list", "-a", "work"]);
    assert_eq!(code, 0);
    assert_eq!(out["count"], 1);
    assert_eq!(out["drafts"][0]["draftId"], "draft_new_1");
    assert_eq!(out["drafts"][0]["to"][0], "recipient@example.com");

    // 3. Get draft
    let (code, out, _) = env.json(&["draft", "get", "draft_new_1", "-a", "work"]);
    assert_eq!(code, 0);
    assert_eq!(out["draftId"], "draft_new_1");
    assert_eq!(out["status"], "draft_not_sent");
    assert!(out["body"].as_str().unwrap().contains("Original Draft Body"));

    // 4. Update draft
    let (code, out, _) = env.json(&[
        "draft",
        "update",
        "draft_new_1",
        "-a",
        "work",
        "--subject",
        "Updated Subject",
        "--body-text",
        "Updated Draft Body",
    ]);
    assert_eq!(code, 0);
    assert_eq!(out["status"], "draft_updated_not_sent");
    assert_eq!(out["subject"], "Updated Subject");

    // 5. Delete draft
    let (code, out, _) = env.json(&["draft", "delete", "draft_new_1", "-a", "work", "--yes"]);
    assert_eq!(code, 0);
    assert_eq!(out["status"], "deleted");
    assert_eq!(out["draftId"], "draft_new_1");
}

#[test]
fn test_label_list() {
    let mock = Mock::start(vec![Route::new(
        "GET",
        "labels",
        200,
        json!({
            "labels": [
                {"id": "INBOX", "name": "INBOX", "type": "system"},
                {"id": "UNREAD", "name": "UNREAD", "type": "system"},
                {"id": "Label_1", "name": "Projects", "type": "user"}
            ]
        }),
    )]);
    let env = Env::new(&mock).with_account("work", "work@example.com", "draft");

    let (code, out, _) = env.json(&["label", "list", "-a", "work"]);
    assert_eq!(code, 0);
    assert_eq!(out["count"], 3);
    assert_eq!(out["labels"][0]["id"], "INBOX");
    assert_eq!(out["labels"][2]["name"], "Projects");
}

#[test]
fn test_doctor_command() {
    let mock = Mock::start(vec![
        Route::new("GET", "/oauth2/clock", 200, Value::Null).with_header("Date", "Wed, 15 Nov 2023 12:00:00 GMT"),
        Route::new(
            "GET",
            "/gmail/v1/users/me/profile",
            200,
            json!({
                "emailAddress": "work@example.com",
                "messagesTotal": 100,
                "threadsTotal": 20,
                "historyId": 500
            }),
        ),
    ]);
    let env = Env::new(&mock).with_account("work", "work@example.com", "draft");

    let (code, out, _) = env.json(&["doctor"]);
    assert_eq!(code, 0);
    assert_eq!(out["secretStore"], "plaintext");
    assert_eq!(out["accountCount"], 1);
    assert!(out["checks"].is_array());
    let checks = out["checks"].as_array().unwrap();
    assert!(checks.iter().any(|c| c["check"] == "secret_store" && c["status"] == "ok"));
    assert!(checks.iter().any(|c| c["check"] == "oauth_client" && c["status"] == "ok"));
    assert!(checks.iter().any(|c| c["check"] == "account:work" && c["status"] == "ok"));
}

#[test]
fn test_error_exit_codes() {
    let mock = Mock::start(vec![]);
    let env = Env::new(&mock);

    // 1. Missing account on mailbox command -> Exit code 7 (NoAccount)
    let (code, _, err) = env.json(&["search", "test"]);
    assert_eq!(code, 7);
    assert_eq!(err["code"], "no_account");
    assert!(err["remediation"].as_str().unwrap().contains("gmail account list"));

    // 2. Invalid input (e.g. limit 0) -> Exit code 6 (InvalidInput)
    let (code, _, err) = env.json(&["search", "test", "-a", "work", "--limit", "0"]);
    assert_eq!(code, 6);
    assert_eq!(err["code"], "invalid_input");

    // 3. Unknown flag -> Clap error envelope with exit code 6
    let (code, _, err) = env.json(&["search", "--non-existent-flag"]);
    assert_eq!(code, 6);
    assert_eq!(err["code"], "invalid_input");
}

#[test]
fn test_login_subcommand() {
    let mock = Mock::start(vec![]);
    let env = Env::new(&mock);

    // Verify `gmail login --help` succeeds
    let output = env.run(&["login", "--help"]);
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Log in to a Google account and store its credentials"));
}

