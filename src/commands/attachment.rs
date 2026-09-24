//! Attachment commands: save message attachments to disk.

use serde_json::{Map, Value};

use crate::account;
use crate::cli::{AttachmentCommand, AttachmentDownloadArgs};
use crate::client::GmailClient;
use crate::error::{Error, Result};
use crate::mail::AttachmentWriter;
use crate::secrets::Store;

pub fn run(cmd: AttachmentCommand, verbose: bool, timeout: u64) -> Result<Value> {
    match cmd {
        AttachmentCommand::Download(args) => run_download(args, verbose, timeout),
    }
}

pub fn run_download(args: AttachmentDownloadArgs, verbose: bool, timeout: u64) -> Result<Value> {
    if !args.all && args.attachment_id.is_none() && args.name.is_none() {
        return Err(Error::invalid("Pass --all, --attachment-id <id> or --name <pattern>."));
    }

    let resolved = account::resolve(args.account.as_deref())?;
    let store = Store::open()?;
    let client = GmailClient::new(&resolved, store, verbose, timeout);

    let parts = crate::commands::message::fetch_attachments(&client, &args.message_id)?;

    let wanted = if let Some(ref att_id) = args.attachment_id {
        let found = parts
            .into_iter()
            .find(|p| p.attachment_id == *att_id)
            .ok_or_else(|| Error::not_found(format!("No attachment with id '{att_id}' on this message.")))?;
        vec![found]
    } else if let Some(ref pat) = args.name {
        let pat_lower = pat.to_lowercase();
        parts.into_iter().filter(|p| p.filename.to_lowercase().contains(&pat_lower)).collect()
    } else {
        parts.into_iter().filter(|p| args.include_inline || !p.inline).collect()
    };

    std::fs::create_dir_all(&args.out_dir)
        .map_err(|e| Error::other(format!("Could not create directory '{}': {e}", args.out_dir)))?;

    let mut saved = Vec::new();
    let mut skipped = Vec::new();

    for (i, part) in wanted.into_iter().enumerate() {
        let index = i + 1;

        if part.size > args.max_size {
            let mut skip_item = Map::new();
            skip_item.insert("filename".into(), Value::String(part.filename.clone()));
            skip_item.insert("size".into(), Value::Number(part.size.into()));
            skip_item
                .insert("reason".into(), Value::String(format!("larger than --max-size ({} bytes)", args.max_size)));
            skipped.push(Value::Object(skip_item));
            continue;
        }

        if part.attachment_id.is_empty() {
            let mut skip_item = Map::new();
            skip_item.insert("filename".into(), Value::String(part.filename.clone()));
            skip_item.insert("reason".into(), Value::String("the part carries no attachment id".into()));
            skipped.push(Value::Object(skip_item));
            continue;
        }

        let safe_name = AttachmentWriter::sanitize(Some(&part.filename), index);
        let path = AttachmentWriter::resolve_path(&args.out_dir, &safe_name, args.overwrite)?;

        let bytes = client.get_attachment(&args.message_id, &part.attachment_id)?;
        std::fs::write(&path, &bytes)
            .map_err(|e| Error::other(format!("Could not write attachment to '{}': {e}", path.display())))?;

        let mut save_item = Map::new();
        save_item.insert("filename".into(), Value::String(part.filename));
        save_item.insert("path".into(), Value::String(path.to_string_lossy().to_string()));
        save_item.insert("mimeType".into(), Value::String(part.mime_type));
        save_item.insert("size".into(), Value::Number((bytes.len() as i64).into()));
        saved.push(Value::Object(save_item));
    }

    let mut result = Map::new();
    result.insert("account".into(), Value::String(client.account_name.clone()));
    result.insert("messageId".into(), Value::String(args.message_id));
    result.insert("outDir".into(), Value::String(full_path(&args.out_dir)));
    result.insert("savedCount".into(), Value::Number(saved.len().into()));
    result.insert("saved".into(), Value::Array(saved.clone()));

    if !skipped.is_empty() {
        result.insert("skipped".into(), Value::Array(skipped.clone()));
    }
    if saved.is_empty() && skipped.is_empty() {
        result.insert("note".into(), Value::String("No attachments matched.".into()));
    }

    Ok(Value::Object(result))
}

fn full_path(path: &str) -> String {
    std::path::Path::new(path).canonicalize().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| {
        let mut cur = std::env::current_dir().unwrap_or_default();
        cur.push(path);
        cur.to_string_lossy().to_string()
    })
}
