//! Label commands: list mailbox labels with their ids.

use serde_json::{Map, Value};

use crate::account;
use crate::cli::{LabelCommand, LabelListArgs};
use crate::client::GmailClient;
use crate::error::Result;
use crate::secrets::Store;

pub fn run(cmd: LabelCommand, verbose: bool, timeout: u64) -> Result<Value> {
    match cmd {
        LabelCommand::List(args) => run_list(args, verbose, timeout),
    }
}

pub fn run_list(args: LabelListArgs, verbose: bool, timeout: u64) -> Result<Value> {
    let resolved = account::resolve(args.account.as_deref())?;
    let store = Store::open()?;
    let client = GmailClient::new(&resolved, store, verbose, timeout);

    let doc = client.get("labels")?;

    let mut label_list = Vec::new();
    if let Some(arr) = doc.get("labels").and_then(Value::as_array) {
        for label in arr {
            let mut obj = Map::new();
            obj.insert("id".into(), label.get("id").cloned().unwrap_or(Value::Null));
            obj.insert("name".into(), label.get("name").cloned().unwrap_or(Value::Null));
            obj.insert("type".into(), label.get("type").cloned().unwrap_or(Value::Null));
            label_list.push(Value::Object(obj));
        }
    }

    let mut result = Map::new();
    result.insert("account".into(), Value::String(client.account_name.clone()));
    result.insert("count".into(), Value::Number(label_list.len().into()));
    result.insert("labels".into(), Value::Array(label_list));

    Ok(Value::Object(result))
}
