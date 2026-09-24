//! Thread commands: get whole conversation in order.

use serde_json::{Map, Value};

use crate::account;
use crate::cli::{ThreadCommand, ThreadGetArgs};
use crate::client::GmailClient;
use crate::error::{Error, Result};
use crate::mail::{BodyModes, MessageRenderer, parse_rfc5322};
use crate::secrets::Store;
use crate::util;

pub fn run(cmd: ThreadCommand, verbose: bool, timeout: u64) -> Result<Value> {
    match cmd {
        ThreadCommand::Get(args) => run_get(args, verbose, timeout),
    }
}

pub fn run_get(args: ThreadGetArgs, verbose: bool, timeout: u64) -> Result<Value> {
    let mode = BodyModes::parse(Some(&args.body))?;
    if args.max_messages < 1 {
        return Err(Error::invalid("--max-messages must be at least 1."));
    }

    let resolved = account::resolve(args.account.as_deref())?;
    let store = Store::open()?;
    let client = GmailClient::new(&resolved, store, verbose, timeout);

    let doc = client.get(&format!("threads/{}?format=minimal", args.thread_id))?;
    let messages = doc
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::not_found(format!("Thread '{}' has no messages.", args.thread_id)))?;

    let all_ids: Vec<String> = messages
        .iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()))
        .map(String::from)
        .collect();

    let total = all_ids.len();
    if total == 0 {
        return Err(Error::not_found(format!("Thread '{}' has no messages.", args.thread_id)));
    }

    let selected = if total > args.max_messages { &all_ids[total - args.max_messages..] } else { &all_ids[..] };

    let mut rendered = Vec::new();
    for id in selected {
        let msg_doc = match client.get(&format!("messages/{id}?format=raw")) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let raw = msg_doc.get("raw").and_then(Value::as_str);
        let Some(raw_str) = raw.filter(|s| !s.is_empty()) else {
            continue;
        };

        let raw_bytes = match util::base64_url_decode(raw_str) {
            Ok(b) => b,
            Err(_) => continue,
        };

        let mime = match parse_rfc5322(&raw_bytes) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let item = MessageRenderer::full(&msg_doc, &mime, mode, args.max_chars, args.keep_quotes, false);
        rendered.push(item);
    }

    let mut result = Map::new();
    result.insert("account".into(), Value::String(client.account_name.clone()));
    result.insert("threadId".into(), Value::String(args.thread_id));
    result.insert("messageCount".into(), Value::Number(total.into()));
    result.insert("returned".into(), Value::Number(rendered.len().into()));
    result.insert("messages".into(), Value::Array(rendered.clone()));

    if rendered.len() < total {
        result.insert(
            "note".into(),
            Value::String(format!(
                "Showing the {} most recent of {} messages. Raise --max-messages for more.",
                rendered.len(),
                total
            )),
        );
    }

    Ok(Value::Object(result))
}
