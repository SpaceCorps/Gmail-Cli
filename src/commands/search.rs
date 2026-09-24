//! Search command: search mailbox and return message summaries with fan-out hydration.

use serde_json::{Map, Value};

use crate::account;
use crate::cli::SearchArgs;
use crate::client::GmailClient;
use crate::error::{Error, Result};
use crate::mail::{GmailFields, MessageRenderer};
use crate::secrets::Store;
use crate::util;

pub fn run(args: SearchArgs, verbose: bool, timeout: u64) -> Result<Value> {
    if args.limit < 1 || args.limit > 500 {
        return Err(Error::invalid("--limit must be between 1 and 500."));
    }
    if args.concurrency < 1 || args.concurrency > 20 {
        return Err(Error::invalid("--concurrency must be between 1 and 20."));
    }

    let resolved = account::resolve(args.account.as_deref())?;
    let store = Store::open()?;
    let client = GmailClient::new(&resolved, store, verbose, timeout);

    let query = build_query(&args);

    let mut url = format!("messages?maxResults={}", args.limit);
    if !query.is_empty() {
        url.push_str(&format!("&q={}", util::url_encode(&query)));
    }
    if let Some(ref pt) = args.page_token
        && !pt.is_empty()
    {
        url.push_str(&format!("&pageToken={}", util::url_encode(pt)));
    }
    if args.include_spam_trash {
        url.push_str("&includeSpamTrash=true");
    }
    for label in &args.labels {
        url.push_str(&format!("&labelIds={}", util::url_encode(label)));
    }

    let listing = client.get(&url)?;

    let mut ids: Vec<(String, String)> = Vec::new();
    if let Some(messages) = listing.get("messages").and_then(Value::as_array) {
        for msg in messages {
            if let Some(id) = msg.get("id").and_then(Value::as_str) {
                let thread_id = msg.get("threadId").and_then(Value::as_str).unwrap_or(id).to_string();

                if args.group_threads && ids.iter().any(|(_, tid)| tid == &thread_id) {
                    continue;
                }
                ids.push((id.to_string(), thread_id));
            }
        }
    }

    let raw_ids: Vec<String> = ids.into_iter().map(|(id, _)| id).collect();
    let hydrated = hydrate(&client, &raw_ids, args.concurrency);

    let mut result = Map::new();
    result.insert("query".into(), Value::String(query));
    result.insert("account".into(), Value::String(client.account_name.clone()));
    result.insert("count".into(), Value::Number(hydrated.len().into()));
    result.insert("messages".into(), Value::Array(hydrated));

    if let Some(next_token) = listing.get("nextPageToken").and_then(Value::as_str)
        && !next_token.is_empty()
    {
        result.insert("nextPageToken".into(), Value::String(next_token.to_string()));
    }

    if let Some(estimate) = listing.get("resultSizeEstimate").and_then(Value::as_i64) {
        result.insert("resultSizeEstimate".into(), Value::Number(estimate.into()));
    }

    Ok(Value::Object(result))
}

fn hydrate(client: &GmailClient, ids: &[String], concurrency: usize) -> Vec<Value> {
    if ids.is_empty() {
        return Vec::new();
    }

    let total = ids.len();
    let ids_with_idx: Vec<(usize, String)> = ids.iter().cloned().enumerate().collect();
    let queue = std::sync::Mutex::new(ids_with_idx);
    let results = std::sync::Mutex::new(vec![Value::Null; total]);

    let num_workers = concurrency.min(total).max(1);

    std::thread::scope(|s| {
        for _ in 0..num_workers {
            s.spawn(|| {
                loop {
                    let next = {
                        let mut q = queue.lock().unwrap();
                        q.pop()
                    };
                    let Some((idx, id)) = next else { break };
                    let url =
                        format!("messages/{}?format=full&fields={}", id, util::url_encode(GmailFields::STRUCTURE));
                    let item = match client.get(&url) {
                        Ok(doc) => MessageRenderer::summary(&doc),
                        Err(e) => {
                            let mut err_obj = Map::new();
                            err_obj.insert("id".into(), Value::String(id));
                            err_obj.insert("error".into(), Value::String("fetch_failed".into()));
                            err_obj.insert("detail".into(), Value::String(e.message));
                            Value::Object(err_obj)
                        }
                    };
                    let mut res = results.lock().unwrap();
                    res[idx] = item;
                }
            });
        }
    });

    results.into_inner().unwrap()
}

fn build_query(args: &SearchArgs) -> String {
    let mut parts = Vec::new();

    if let Some(ref q) = args.query {
        let trimmed = q.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }

    if let Some(ref from) = args.from {
        let trimmed = from.trim();
        if !trimmed.is_empty() {
            parts.push(format!("from:{}", quote(trimmed)));
        }
    }

    if let Some(ref to) = args.to {
        let trimmed = to.trim();
        if !trimmed.is_empty() {
            parts.push(format!("to:{}", quote(trimmed)));
        }
    }

    if let Some(ref subj) = args.subject {
        let trimmed = subj.trim();
        if !trimmed.is_empty() {
            parts.push(format!("subject:{}", quote(trimmed)));
        }
    }

    if args.unread {
        parts.push("is:unread".to_string());
    }

    if args.has_attachment {
        parts.push("has:attachment".to_string());
    }

    if let Some(ref after) = args.after {
        let trimmed = after.trim();
        if !trimmed.is_empty() {
            parts.push(format!("after:{}", trimmed.replace('-', "/")));
        }
    }

    if let Some(ref before) = args.before {
        let trimmed = before.trim();
        if !trimmed.is_empty() {
            parts.push(format!("before:{}", trimmed.replace('-', "/")));
        }
    }

    parts.join(" ")
}

fn quote(val: &str) -> String {
    if val.contains(' ') { format!("\"{val}\"") } else { val.to_string() }
}
