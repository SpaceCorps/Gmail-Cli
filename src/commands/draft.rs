//! Draft commands: create, reply, list, get, update, and delete drafts.
//!
//! Safety guarantee: drafts are only ever created, updated, or deleted — nothing is ever sent.

use std::io::Write;

use serde_json::{Map, Value};

use crate::account;
use crate::auth::ScopeProfiles;
use crate::cli::{
    DraftCommand, DraftCreateArgs, DraftDeleteArgs, DraftGetArgs, DraftListArgs, DraftReplyArgs, DraftUpdateArgs,
};
use crate::client::GmailClient;
use crate::error::{Error, Result};
use crate::mail::body_mode::BodyMode;
use crate::mail::renderer::{extract_headers_map, html_to_markdown, internal_date, normalize, truncate};
use crate::mail::rfc5322::{DraftContent, ParsedMessage, build_rfc5322, parse_address_list, parse_rfc5322};
use crate::mail::{BodyInput, BodyModes, ReplyBuilder};
use crate::secrets::Store;
use crate::util;

pub fn run(cmd: DraftCommand, verbose: bool, timeout: u64) -> Result<Value> {
    match cmd {
        DraftCommand::Create(args) => run_create(args, verbose, timeout),
        DraftCommand::Reply(args) => run_reply(args, verbose, timeout),
        DraftCommand::List(args) => run_list(args, verbose, timeout),
        DraftCommand::Get(args) => run_get(args, verbose, timeout),
        DraftCommand::Update(args) => run_update(args, verbose, timeout),
        DraftCommand::Delete(args) => run_delete(args, verbose, timeout),
    }
}

pub fn run_create(args: DraftCreateArgs, verbose: bool, timeout: u64) -> Result<Value> {
    let resolved = account::resolve(args.account.as_deref())?;
    ScopeProfiles::require_draft(&resolved.name, resolved.scope_profile())?;

    let store = Store::open()?;
    let client = GmailClient::new(&resolved, store, verbose, timeout);

    let body = BodyInput::resolve(args.body.as_deref(), args.body_text.as_deref())?;
    let fmt = body_format(args.html, args.plain);

    let content = DraftContent {
        from_email: client.account_email.clone(),
        to: args.to,
        cc: args.cc,
        bcc: args.bcc,
        subject: args.subject,
        body,
        body_format: fmt,
        attachment_paths: args.attach,
        in_reply_to: None,
        references: Vec::new(),
    };

    let mime_str = build_rfc5322(&content)?;
    let raw = util::base64_url_encode(mime_str.as_bytes());
    let parsed = parse_rfc5322(mime_str.as_bytes())?;

    let (response, status) = match args.replace_draft {
        Some(ref draft_id) => {
            let resp = client.put(&format!("drafts/{draft_id}"), &draft_body(&raw, None))?;
            (resp, "draft_updated_not_sent")
        }
        None => {
            let resp = client.post("drafts", &draft_body(&raw, None))?;
            (resp, "draft_created_not_sent")
        }
    };

    Ok(describe_draft(&response, &parsed, &client.account_name, status))
}

pub fn run_reply(args: DraftReplyArgs, verbose: bool, timeout: u64) -> Result<Value> {
    let resolved = account::resolve(args.account.as_deref())?;
    ScopeProfiles::require_draft(&resolved.name, resolved.scope_profile())?;

    let store = Store::open()?;
    let client = GmailClient::new(&resolved, store, verbose, timeout);

    let doc = client.get(&format!("messages/{}?format=raw", args.message_id))?;
    let raw_str = doc
        .get("raw")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::not_found(format!("Message '{}' returned no content.", args.message_id)))?;

    let raw_bytes = util::base64_url_decode(raw_str)?;
    let parent = parse_rfc5322(&raw_bytes)?;
    let thread_id = doc.get("threadId").and_then(Value::as_str).unwrap_or("");

    let reply = ReplyBuilder::build(&parent, &client.account_email, args.all);

    let body = BodyInput::resolve(args.body.as_deref(), args.body_text.as_deref())?;
    let final_body = if args.no_quote { body } else { format!("{}\n\n{}", body.trim_end(), reply.quoted_parent) };

    let to = if !args.to.is_empty() { args.to } else { reply.to };

    let mut cc = reply.cc;
    cc.extend(args.cc);

    let fmt = body_format(args.html, args.plain);

    let content = DraftContent {
        from_email: client.account_email.clone(),
        to,
        cc,
        bcc: Vec::new(),
        subject: reply.subject,
        body: final_body,
        body_format: fmt,
        attachment_paths: args.attach,
        in_reply_to: reply.in_reply_to.clone(),
        references: reply.references,
    };

    let mime_str = build_rfc5322(&content)?;
    let raw = util::base64_url_encode(mime_str.as_bytes());
    let parsed = parse_rfc5322(mime_str.as_bytes())?;

    let response = client.post("drafts", &draft_body(&raw, Some(thread_id)))?;
    let mut result = describe_draft(&response, &parsed, &client.account_name, "draft_created_not_sent");

    if let Value::Object(ref mut map) = result {
        if let Some(ref irt) = reply.in_reply_to {
            map.insert("inReplyTo".into(), Value::String(format!("<{irt}>")));
        } else {
            map.insert("inReplyTo".into(), Value::Null);
        }
        map.insert("replyAll".into(), Value::Bool(args.all));
    }

    Ok(result)
}

pub fn run_list(args: DraftListArgs, verbose: bool, timeout: u64) -> Result<Value> {
    let resolved = account::resolve(args.account.as_deref())?;
    let store = Store::open()?;
    let client = GmailClient::new(&resolved, store, verbose, timeout);

    let mut url = format!("drafts?maxResults={}", args.limit);
    if let Some(ref pt) = args.page_token
        && !pt.is_empty()
    {
        url.push_str(&format!("&pageToken={}", util::url_encode(pt)));
    }

    let listing = client.get(&url)?;

    let mut drafts = Vec::new();
    if let Some(list) = listing.get("drafts").and_then(Value::as_array) {
        for draft in list {
            let draft_id = draft.get("id").and_then(Value::as_str).unwrap_or("");
            let message_id = draft.get("message").and_then(|m| m.get("id")).and_then(Value::as_str);

            let mut entry = Map::new();
            entry.insert("draftId".into(), Value::String(draft_id.to_string()));
            entry.insert("messageId".into(), message_id.map(|s| Value::String(s.to_string())).unwrap_or(Value::Null));
            entry.insert(
                "webUrl".into(),
                Value::String(format!("https://mail.google.com/mail/u/0/#drafts?compose={draft_id}")),
            );

            if let Some(mid) = message_id {
                let meta_url = format!(
                    "messages/{mid}?format=metadata&metadataHeaders=From&metadataHeaders=To&metadataHeaders=Cc&metadataHeaders=Subject&metadataHeaders=Date&metadataHeaders=Message-ID&metadataHeaders=Reply-To"
                );
                if let Ok(msg_doc) = client.get(&meta_url) {
                    let headers = extract_headers_map(&msg_doc);
                    let to_list = headers.get("to").map(|s| parse_address_list(s)).unwrap_or_default();
                    entry.insert("to".into(), Value::Array(to_list.into_iter().map(Value::String).collect()));

                    let subject = headers.get("subject").cloned().unwrap_or_else(|| "(no subject)".to_string());
                    entry.insert("subject".into(), Value::String(subject));

                    entry.insert("updated".into(), internal_date(&msg_doc).map(Value::String).unwrap_or(Value::Null));
                }
            }

            drafts.push(Value::Object(entry));
        }
    }

    let mut result = Map::new();
    result.insert("account".into(), Value::String(client.account_name.clone()));
    result.insert("count".into(), Value::Number(drafts.len().into()));
    result.insert("drafts".into(), Value::Array(drafts));

    if let Some(token) = listing.get("nextPageToken").and_then(Value::as_str)
        && !token.is_empty()
    {
        result.insert("nextPageToken".into(), Value::String(token.to_string()));
    }

    Ok(Value::Object(result))
}

pub fn run_get(args: DraftGetArgs, verbose: bool, timeout: u64) -> Result<Value> {
    let mode = BodyModes::parse(Some(&args.body))?;

    let resolved = account::resolve(args.account.as_deref())?;
    let store = Store::open()?;
    let client = GmailClient::new(&resolved, store, verbose, timeout);

    let doc = client.get(&format!("drafts/{}?format=raw", args.draft_id))?;
    let envelope =
        doc.get("message").ok_or_else(|| Error::not_found(format!("Draft '{}' has no message.", args.draft_id)))?;

    let raw_str = envelope
        .get("raw")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::not_found(format!("Draft '{}' returned no content.", args.draft_id)))?;

    let raw_bytes = util::base64_url_decode(raw_str)?;
    let mime = parse_rfc5322(&raw_bytes)?;

    let body_opt = match mode {
        BodyMode::None => None,
        BodyMode::Html => mime.html_body.clone(),
        _ => mime.text_body.clone().or_else(|| mime.html_body.as_deref().map(html_to_markdown)),
    };

    let mut result = Map::new();
    result.insert("account".into(), Value::String(client.account_name.clone()));
    result.insert("draftId".into(), Value::String(args.draft_id.clone()));
    result.insert("messageId".into(), envelope.get("id").cloned().unwrap_or(Value::Null));
    result.insert("threadId".into(), envelope.get("threadId").cloned().unwrap_or(Value::Null));
    result.insert("to".into(), Value::Array(mime.to.iter().cloned().map(Value::String).collect()));
    result.insert("subject".into(), Value::String(mime.subject.clone().unwrap_or_default()));
    result.insert(
        "webUrl".into(),
        Value::String(format!("https://mail.google.com/mail/u/0/#drafts?compose={}", args.draft_id)),
    );
    result.insert("status".into(), Value::String("draft_not_sent".into()));

    if !mime.cc.is_empty() {
        result.insert("cc".into(), Value::Array(mime.cc.iter().cloned().map(Value::String).collect()));
    }
    if !mime.bcc.is_empty() {
        result.insert("bcc".into(), Value::Array(mime.bcc.iter().cloned().map(Value::String).collect()));
    }

    if let Some(body_text) = body_opt {
        let (mut text, omitted, total) = truncate(&normalize(&body_text), args.max_chars);
        if omitted > 0 {
            text.push_str(&format!("\n\n[truncated: {omitted} of {total} characters omitted]"));
            result.insert("truncated".into(), Value::Bool(true));
        }
        result.insert("body".into(), Value::String(text));
    }

    Ok(Value::Object(result))
}

pub fn run_update(args: DraftUpdateArgs, verbose: bool, timeout: u64) -> Result<Value> {
    let resolved = account::resolve(args.account.as_deref())?;
    ScopeProfiles::require_draft(&resolved.name, resolved.scope_profile())?;

    let store = Store::open()?;
    let client = GmailClient::new(&resolved, store, verbose, timeout);

    let existing = client.get(&format!("drafts/{}?format=raw", args.draft_id))?;
    let envelope = existing
        .get("message")
        .ok_or_else(|| Error::not_found(format!("Draft '{}' has no message.", args.draft_id)))?;

    let raw_existing = envelope
        .get("raw")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::not_found(format!("Draft '{}' returned no content.", args.draft_id)))?;

    let raw_bytes = util::base64_url_decode(raw_existing)?;
    let current = parse_rfc5322(&raw_bytes)?;
    let thread_id = envelope.get("threadId").and_then(Value::as_str).unwrap_or("");

    let body = if args.body.is_none() && args.body_text.is_none() {
        current.text_body.clone().or_else(|| current.html_body.as_deref().map(html_to_markdown)).unwrap_or_default()
    } else {
        BodyInput::resolve(args.body.as_deref(), args.body_text.as_deref())?
    };

    let to = if !args.to.is_empty() { args.to } else { current.to };

    let cc = if !args.cc.is_empty() { args.cc } else { current.cc };

    let bcc = if !args.bcc.is_empty() { args.bcc } else { current.bcc };

    let subject = args.subject.unwrap_or_else(|| current.subject.unwrap_or_default());
    let fmt = body_format(args.html, args.plain);

    let content = DraftContent {
        from_email: client.account_email.clone(),
        to,
        cc,
        bcc,
        subject,
        body,
        body_format: fmt,
        attachment_paths: args.attach,
        in_reply_to: current.in_reply_to,
        references: current.references,
    };

    let mime_str = build_rfc5322(&content)?;
    let raw = util::base64_url_encode(mime_str.as_bytes());
    let parsed = parse_rfc5322(mime_str.as_bytes())?;

    let response = client.put(&format!("drafts/{}", args.draft_id), &draft_body(&raw, Some(thread_id)))?;

    Ok(describe_draft(&response, &parsed, &client.account_name, "draft_updated_not_sent"))
}

pub fn run_delete(args: DraftDeleteArgs, verbose: bool, timeout: u64) -> Result<Value> {
    let resolved = account::resolve(args.account.as_deref())?;
    ScopeProfiles::require_draft(&resolved.name, resolved.scope_profile())?;

    let store = Store::open()?;
    let client = GmailClient::new(&resolved, store, verbose, timeout);

    if !args.yes {
        eprint!("Delete draft '{}' from {}? [y/N]: ", args.draft_id, client.account_email);
        std::io::stderr().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map_err(|e| Error::other(e.to_string()))?;
        let trimmed = line.trim();
        if !trimmed.eq_ignore_ascii_case("y") && !trimmed.eq_ignore_ascii_case("yes") {
            return Err(Error::invalid("Cancelled."));
        }
    }

    client.delete(&format!("drafts/{}", args.draft_id))?;

    let mut result = Map::new();
    result.insert("account".into(), Value::String(client.account_name.clone()));
    result.insert("draftId".into(), Value::String(args.draft_id));
    result.insert("status".into(), Value::String("deleted".into()));

    Ok(Value::Object(result))
}

fn describe_draft(response: &Value, message: &ParsedMessage, account_name: &str, status: &str) -> Value {
    let draft_id = response.get("id").and_then(Value::as_str);

    let (message_id, thread_id) = if let Some(inner) = response.get("message") {
        (inner.get("id").and_then(Value::as_str), inner.get("threadId").and_then(Value::as_str))
    } else {
        (None, None)
    };

    let mut m = Map::new();
    m.insert("account".into(), Value::String(account_name.to_string()));
    m.insert("draftId".into(), draft_id.map(|s| Value::String(s.to_string())).unwrap_or(Value::Null));
    m.insert("messageId".into(), message_id.map(|s| Value::String(s.to_string())).unwrap_or(Value::Null));
    m.insert("threadId".into(), thread_id.map(|s| Value::String(s.to_string())).unwrap_or(Value::Null));
    m.insert("to".into(), Value::Array(message.to.iter().cloned().map(Value::String).collect()));
    m.insert("subject".into(), Value::String(message.subject.clone().unwrap_or_default()));

    if !message.cc.is_empty() {
        m.insert("cc".into(), Value::Array(message.cc.iter().cloned().map(Value::String).collect()));
    }
    if !message.bcc.is_empty() {
        m.insert("bcc".into(), Value::Array(message.bcc.iter().cloned().map(Value::String).collect()));
    }

    let attachments: Vec<Value> = message.attachments.iter().map(|a| Value::String(a.filename.clone())).collect();
    m.insert("attachments".into(), Value::Array(attachments));

    if let Some(id) = draft_id {
        m.insert("webUrl".into(), Value::String(format!("https://mail.google.com/mail/u/0/#drafts?compose={id}")));
    }

    m.insert("status".into(), Value::String(status.to_string()));

    Value::Object(m)
}

fn draft_body(raw: &str, thread_id: Option<&str>) -> Value {
    let mut msg = Map::new();
    msg.insert("raw".into(), Value::String(raw.to_string()));
    if let Some(tid) = thread_id.filter(|s| !s.is_empty()) {
        msg.insert("threadId".into(), Value::String(tid.to_string()));
    }

    let mut root = Map::new();
    root.insert("message".into(), Value::Object(msg));
    Value::Object(root)
}

fn body_format(html: bool, plain: bool) -> String {
    if html {
        "html".to_string()
    } else if plain {
        "plain".to_string()
    } else {
        "markdown".to_string()
    }
}
