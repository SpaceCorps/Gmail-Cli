//! Renders command output as YAML by default, or raw JSON with --json.
//!
//! YAML reads well but is awkward to script against: pulling an id out of a listing means
//! line-pairing or a regex, and both break quietly when the shape changes. --json exists so
//! callers can pipe into jq instead.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value};

use crate::error::Error;

/// Seeded from the raw args before parsing, because the error envelope can be rendered before
/// any command has run.
static USE_JSON: AtomicBool = AtomicBool::new(false);

pub fn set_json(on: bool) {
    USE_JSON.store(on, Ordering::Relaxed);
}

pub fn json() -> bool {
    USE_JSON.load(Ordering::Relaxed)
}

/// Recursively removes null entries from maps and arrays so absent optional values
/// do not emit bare keys like `cc: null` or `detail: null`.
pub fn prune(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut pruned = Map::new();
            for (k, v) in map {
                if !v.is_null() {
                    let pv = prune(v);
                    if !pv.is_null() {
                        pruned.insert(k.clone(), pv);
                    }
                }
            }
            Value::Object(pruned)
        }
        Value::Array(arr) => {
            let pruned: Vec<Value> = arr.iter().filter(|v| !v.is_null()).map(prune).collect();
            Value::Array(pruned)
        }
        _ => value.clone(),
    }
}

pub fn render(value: &Value) -> String {
    let pruned = prune(value);
    if json() { serde_json::to_string_pretty(&pruned).unwrap_or_default() } else { yaml(&pruned) }
}

pub fn yaml(value: &Value) -> String {
    let mut s = serde_norway::to_string(value).unwrap_or_default();
    while s.ends_with('\n') {
        s.pop();
    }
    s
}

pub fn write(value: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", render(value));
}

/// Human-facing chatter always goes to stderr so stdout stays machine-readable.
pub fn status(message: impl AsRef<str>) {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "{}", message.as_ref());
}

/// The error envelope on stderr. Returns the exit code, which is the same thing as `code:`.
pub fn write_error(e: &Error) -> i32 {
    let mut payload = Map::new();
    payload.insert("error".into(), Value::String(e.message.clone()));
    payload.insert("code".into(), Value::String(e.code.name().into()));
    if let Some(d) = e.detail.as_deref().filter(|d| !d.trim().is_empty()) {
        payload.insert("detail".into(), Value::String(d.into()));
    }
    if let Some(r) = e.remediation.as_deref().filter(|r| !r.trim().is_empty()) {
        payload.insert("remediation".into(), Value::String(r.into()));
    }
    let _ = writeln!(std::io::stderr().lock(), "{}", render(&Value::Object(payload)));
    e.code as i32
}

/// Builds a JSON object from `key => value` pairs, keeping their order.
#[macro_export]
macro_rules! obj {
    ($($k:expr => $v:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut m = ::serde_json::Map::new();
        $( m.insert(($k).into(), ::serde_json::json!($v)); )*
        ::serde_json::Value::Object(m)
    }};
}
