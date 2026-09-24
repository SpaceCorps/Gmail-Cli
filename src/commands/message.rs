//! Message commands: get single message and list attachments.

use serde_json::{Map, Value};

use crate::account;
use crate::cli::{MessageAttachmentsArgs, MessageCommand, MessageGetArgs};
use crate::client::GmailClient;
use crate::error::{Error, Result};
use crate::mail::attachment::AttachmentPart;
use crate::mail::{AttachmentWriter, BodyModes, GmailFields, MessageRenderer, parse_rfc5322};
use crate::secrets::Store;
use crate::util;

pub fn run(cmd: MessageCommand, verbose: bool, timeout: u64) -> Result<Value> {
    match cmd {
        MessageCommand::Get(args) => run_get(args, verbose, timeout),
        MessageCommand::Attachments(args) => run_attachments(args, verbose, timeout),
    }
}

pub fn run_get(args: MessageGetArgs, verbose: bool, timeout: u64) -> Result<Value> {
    let mode = BodyModes::parse(Some(&args.body))?;

    let resolved = account::resolve(args.account.as_deref())?;
    let store = Store::open()?;
    let client = GmailClient::new(&resolved, store, verbose, timeout);

    let envelope = client.get(&format!("messages/{}?format=raw", args.message_id))?;
    let raw = envelope.get("raw").and_then(Value::as_str);
    let Some(raw_str) = raw.filter(|s| !s.is_empty()) else {
        return Err(Error::not_found(format!("Message '{}' returned no content.", args.message_id)));
    };

    let raw_bytes = util::base64_url_decode(raw_str)?;

    if let Some(ref save_path) = args.save_raw {
        std::fs::write(save_path, &raw_bytes)
            .map_err(|e| Error::other(format!("Could not save raw message to '{save_path}': {e}")))?;
    }

    let mime = parse_rfc5322(&raw_bytes)?;
    let mut rendered = MessageRenderer::full(&envelope, &mime, mode, args.max_chars, args.keep_quotes, args.headers);

    if let Value::Object(ref mut map) = rendered {
        map.insert("account".into(), Value::String(client.account_name.clone()));
        if let Some(ref save_path) = args.save_raw {
            map.insert("savedRawTo".into(), Value::String(full_path(save_path)));
        }
    }

    Ok(rendered)
}

pub fn run_attachments(args: MessageAttachmentsArgs, verbose: bool, timeout: u64) -> Result<Value> {
    let resolved = account::resolve(args.account.as_deref())?;
    let store = Store::open()?;
    let client = GmailClient::new(&resolved, store, verbose, timeout);

    let parts = fetch_attachments(&client, &args.message_id)?;

    let mut result = Map::new();
    result.insert("account".into(), Value::String(client.account_name.clone()));
    result.insert("messageId".into(), Value::String(args.message_id));
    result.insert("count".into(), Value::Number(parts.len().into()));

    let attachment_list: Vec<Value> = parts
        .into_iter()
        .map(|p| {
            let mut obj = Map::new();
            obj.insert("attachmentId".into(), Value::String(p.attachment_id));
            obj.insert("filename".into(), Value::String(p.filename.clone()));
            obj.insert("safeFilename".into(), Value::String(AttachmentWriter::sanitize(Some(&p.filename), 1)));
            obj.insert("mimeType".into(), Value::String(p.mime_type));
            obj.insert("size".into(), Value::Number(p.size.into()));
            obj.insert("inline".into(), Value::Bool(p.inline));
            Value::Object(obj)
        })
        .collect();

    result.insert("attachments".into(), Value::Array(attachment_list));

    Ok(Value::Object(result))
}

pub fn fetch_attachments(client: &GmailClient, message_id: &str) -> Result<Vec<AttachmentPart>> {
    let url = format!("messages/{}?format=full&fields={}", message_id, util::url_encode(GmailFields::STRUCTURE));
    let doc = client.get(&url)?;
    Ok(crate::mail::renderer::describe_attachments(&doc))
}

fn full_path(path: &str) -> String {
    std::path::Path::new(path).canonicalize().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| {
        let mut cur = std::env::current_dir().unwrap_or_default();
        cur.push(path);
        cur.to_string_lossy().to_string()
    })
}
