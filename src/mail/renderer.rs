use serde_json::{Map, Value};

use crate::mail::attachment::AttachmentPart;
use crate::mail::body_mode::BodyMode;
use crate::mail::rfc5322::ParsedMessage;

pub const UNTRUSTED_OPEN: &str = "--- untrusted email content begins ---";
pub const UNTRUSTED_CLOSE: &str = "--- untrusted email content ends ---";

pub struct MessageRenderer;

impl MessageRenderer {
    pub fn summary(message: &Value) -> Value {
        let headers = extract_headers_map(message);
        let attachments = describe_attachments(message);

        let mut m = Map::new();
        m.insert("id".into(), message.get("id").cloned().unwrap_or(Value::Null));
        m.insert("threadId".into(), message.get("threadId").cloned().unwrap_or(Value::Null));
        m.insert("date".into(), internal_date(message).map(Value::String).unwrap_or(Value::Null));
        m.insert("from".into(), headers.get("from").cloned().map(Value::String).unwrap_or(Value::Null));

        let to_list = headers.get("to").map(|s| crate::mail::rfc5322::parse_address_list(s)).unwrap_or_default();
        m.insert("to".into(), Value::Array(to_list.into_iter().map(Value::String).collect()));

        let subject = headers.get("subject").cloned().unwrap_or_else(|| "(no subject)".to_string());
        m.insert("subject".into(), Value::String(subject));

        let snippet = message.get("snippet").and_then(Value::as_str).map(html_decode);
        m.insert("snippet".into(), snippet.map(Value::String).unwrap_or(Value::Null));

        m.insert("labels".into(), labels(message));
        m.insert("hasAttachments".into(), Value::Bool(!attachments.is_empty()));
        m.insert("attachmentCount".into(), Value::Number(attachments.len().into()));

        if let Some(cc) = headers.get("cc") {
            let cc_list = crate::mail::rfc5322::parse_address_list(cc);
            if !cc_list.is_empty() {
                m.insert("cc".into(), Value::Array(cc_list.into_iter().map(Value::String).collect()));
            }
        }

        if let Some(size) = message.get("sizeEstimate").and_then(Value::as_i64) {
            m.insert("sizeEstimate".into(), Value::Number(size.into()));
        }

        Value::Object(m)
    }

    pub fn full(
        envelope: &Value,
        mime: &ParsedMessage,
        mode: BodyMode,
        max_chars: usize,
        keep_quotes: bool,
        all_headers: bool,
    ) -> Value {
        let mut m = Map::new();
        m.insert("id".into(), envelope.get("id").cloned().unwrap_or(Value::Null));
        m.insert("threadId".into(), envelope.get("threadId").cloned().unwrap_or(Value::Null));

        let date_str = mime.date.clone().or_else(|| internal_date(envelope));
        m.insert("date".into(), date_str.map(Value::String).unwrap_or(Value::Null));
        m.insert("from".into(), Value::String(mime.from.clone()));
        m.insert("to".into(), Value::Array(mime.to.iter().cloned().map(Value::String).collect()));

        let subject = mime.subject.clone().unwrap_or_else(|| "(no subject)".to_string());
        m.insert("subject".into(), Value::String(subject));
        m.insert("labels".into(), labels(envelope));

        if !mime.cc.is_empty() {
            m.insert("cc".into(), Value::Array(mime.cc.iter().cloned().map(Value::String).collect()));
        }

        if !mime.reply_to.is_empty() {
            m.insert("replyTo".into(), Value::Array(mime.reply_to.iter().cloned().map(Value::String).collect()));
        }

        if let Some(ref mid) = mime.message_id {
            m.insert("messageIdHeader".into(), Value::String(format!("<{mid}>")));
        }

        let has_attach = !mime.attachments.is_empty();
        m.insert("hasAttachments".into(), Value::Bool(has_attach));
        if has_attach {
            let mut attach_list = Vec::new();
            for a in &mime.attachments {
                let mut am = Map::new();
                am.insert("filename".into(), Value::String(a.filename.clone()));
                am.insert("mimeType".into(), Value::String(a.mime_type.clone()));
                attach_list.push(Value::Object(am));
            }
            m.insert("attachments".into(), Value::Array(attach_list));
        }

        if all_headers {
            let mut hm = Map::new();
            for (k, v) in &mime.headers {
                hm.insert(k.clone(), Value::String(v.clone()));
            }
            m.insert("headers".into(), Value::Object(hm));
        }

        if mode == BodyMode::Snippet {
            let snippet = envelope.get("snippet").and_then(Value::as_str).map(html_decode).unwrap_or_default();
            m.insert("body".into(), Value::String(snippet));
            return Value::Object(m);
        }

        if mode == BodyMode::None {
            return Value::Object(m);
        }

        let mut body = match mode {
            BodyMode::Html => mime.html_body.clone().or_else(|| mime.text_body.clone()).unwrap_or_default(),
            BodyMode::Text => {
                mime.text_body.clone().or_else(|| mime.html_body.as_deref().map(html_to_markdown)).unwrap_or_default()
            }
            _ => mime.text_body.clone().or_else(|| mime.html_body.as_deref().map(html_to_markdown)).unwrap_or_default(),
        };

        if !keep_quotes {
            body = trim_quoted_reply(&body);
        }
        body = normalize(&body);

        let (mut truncated, omitted, total) = truncate(&body, max_chars);
        if omitted > 0 {
            truncated.push_str(&format!(
                "\n\n[truncated: {} of {} characters omitted — rerun with --max-chars 0 for the full body]",
                format_number(omitted),
                format_number(total)
            ));
            m.insert("truncated".into(), Value::Bool(true));
        }

        let wrapped = format!("{UNTRUSTED_OPEN}\n{truncated}\n{UNTRUSTED_CLOSE}");
        m.insert("body".into(), Value::String(wrapped));

        Value::Object(m)
    }
}

pub fn describe_attachments(message: &Value) -> Vec<AttachmentPart> {
    let mut parts = Vec::new();
    if let Some(payload) = message.get("payload") {
        walk_parts(payload, &mut parts);
    }
    parts
}

fn walk_parts(part: &Value, found: &mut Vec<AttachmentPart>) {
    let filename = part.get("filename").and_then(Value::as_str).unwrap_or("");
    if !filename.is_empty() {
        let body = part.get("body");
        let attachment_id = body.and_then(|b| b.get("attachmentId")).and_then(Value::as_str).unwrap_or("").to_string();
        let size = body.and_then(|b| b.get("size")).and_then(Value::as_i64).unwrap_or(0);
        let mime_type = part.get("mimeType").and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
        let inline = is_inline_part(part);

        found.push(AttachmentPart { attachment_id, filename: filename.to_string(), mime_type, size, inline });
    }

    if let Some(subparts) = part.get("parts").and_then(Value::as_array) {
        for sp in subparts {
            walk_parts(sp, found);
        }
    }
}

fn is_inline_part(part: &Value) -> bool {
    if let Some(headers) = part.get("headers").and_then(Value::as_array) {
        for h in headers {
            let name = h.get("name").and_then(Value::as_str).unwrap_or("");
            let val = h.get("value").and_then(Value::as_str).unwrap_or("");
            if name.eq_ignore_ascii_case("content-disposition") && val.to_lowercase().starts_with("inline") {
                return true;
            }
            if name.eq_ignore_ascii_case("content-id") {
                return true;
            }
        }
    }
    false
}

pub fn labels(message: &Value) -> Value {
    if let Some(arr) = message.get("labelIds").and_then(Value::as_array) {
        Value::Array(arr.clone())
    } else {
        Value::Array(Vec::new())
    }
}

pub fn internal_date(message: &Value) -> Option<String> {
    let id_val = message.get("internalDate")?;
    let ms: i64 = if let Some(n) = id_val.as_i64() {
        n
    } else {
        let s = id_val.as_str()?;
        s.parse().ok()?
    };

    let secs = ms / 1000;
    Some(format_epoch_utc(secs))
}

fn format_epoch_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

pub fn extract_headers_map(message: &Value) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Some(payload) = message.get("payload")
        && let Some(headers) = payload.get("headers").and_then(Value::as_array)
    {
        for h in headers {
            if let (Some(n), Some(v)) = (h.get("name").and_then(Value::as_str), h.get("value").and_then(Value::as_str))
            {
                map.insert(n.to_lowercase(), v.to_string());
            }
        }
    }
    map
}

pub fn truncate(body: &str, max_chars: usize) -> (String, usize, usize) {
    if max_chars == 0 || body.len() <= max_chars {
        return (body.to_string(), 0, body.len());
    }

    let window = &body[..max_chars];
    let boundary = if let Some(idx) = window.rfind("\n\n") {
        if idx >= max_chars / 2 {
            idx
        } else if let Some(idx_nl) = window.rfind('\n') {
            if idx_nl >= max_chars / 2 { idx_nl } else { max_chars }
        } else {
            max_chars
        }
    } else if let Some(idx_nl) = window.rfind('\n') {
        if idx_nl >= max_chars / 2 { idx_nl } else { max_chars }
    } else {
        max_chars
    };

    let kept = body[..boundary].trim_end().to_string();
    let omitted = body.len() - kept.len();
    (kept, omitted, body.len())
}

pub fn trim_quoted_reply(body: &str) -> String {
    if body.is_empty() {
        return String::new();
    }

    let normalized = body.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut cut = lines.len();

    for (i, line) in lines.iter().enumerate() {
        if is_attribution_line(line) || is_original_message_line(line) {
            cut = i;
            break;
        }
    }

    while cut > 0 && (lines[cut - 1].starts_with('>') || lines[cut - 1].trim().is_empty()) {
        cut -= 1;
    }

    if cut == lines.len() { body.to_string() } else { lines[..cut].join("\n").trim_end().to_string() }
}

fn is_attribution_line(line: &str) -> bool {
    let trimmed = line.trim();
    let lower = trimmed.to_lowercase();
    let starts_match = lower.starts_with("on ")
        || lower.starts_with("den ")
        || lower.starts_with("am ")
        || lower.starts_with("le ")
        || lower.starts_with("el ");
    let ends_match = trimmed.ends_with(':')
        && (lower.ends_with("wrote:")
            || lower.ends_with("skrev:")
            || lower.ends_with("schrieb:")
            || lower.ends_with("a écrit:")
            || lower.ends_with("escribió:"));
    starts_match && ends_match && trimmed.len() <= 250
}

fn is_original_message_line(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with("--") || !trimmed.ends_with("--") {
        return false;
    }
    let inner = trimmed.trim_matches('-').trim();
    inner.eq_ignore_ascii_case("original message")
        || inner.eq_ignore_ascii_case("ursprungligt meddelande")
        || inner.eq_ignore_ascii_case("forwarded message")
}

pub fn normalize(body: &str) -> String {
    let replaced = body.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    let mut blanks = 0;

    for line in replaced.split('\n') {
        let detab = line.replace('\t', "    ");
        let trimmed_line = detab.trim_end();

        if trimmed_line.is_empty() {
            blanks += 1;
            if blanks > 2 {
                continue;
            }
        } else {
            blanks = 0;
        }

        out.push(trimmed_line.to_string());
    }

    let joined = out.join("\n");
    joined.trim_matches('\n').to_string()
}

pub fn html_to_markdown(html: &str) -> String {
    if html.trim().is_empty() {
        return String::new();
    }

    // Strip script and style blocks
    let stripped = strip_script_and_style(html);

    // Convert HTML elements to markdown
    let mut md = String::new();
    let mut i = 0;
    let bytes = stripped.as_bytes();

    while i < bytes.len() {
        if bytes[i] == b'<'
            && let Some(close_idx) = stripped[i..].find('>')
        {
            let tag = &stripped[i + 1..i + close_idx];
            let tag_lower = tag.to_lowercase();
            let tag_name = tag_lower.split_whitespace().next().unwrap_or("");

            match tag_name {
                "p" | "/p" => md.push_str("\n\n"),
                "br" | "br/" => md.push('\n'),
                "h1" | "/h1" | "h2" | "/h2" | "h3" | "/h3" => md.push_str("\n\n"),
                "li" => md.push_str("\n- "),
                "ul" | "/ul" | "ol" | "/ol" => md.push('\n'),
                "blockquote" | "/blockquote" => md.push_str("\n> "),
                _ if tag_name.starts_with('a') && !tag_name.starts_with("/a") => {
                    if let Some(href) = extract_attr(tag, "href") {
                        // Find matching </a>
                        if let Some(end_a) = stripped[i + close_idx + 1..].to_lowercase().find("</a>") {
                            let inner_text = &stripped[i + close_idx + 1..i + close_idx + 1 + end_a];
                            let clean_inner = strip_all_tags(inner_text);
                            md.push_str(&format!("[{clean_inner}]({href})"));
                            i += close_idx + 1 + end_a + 4;
                            continue;
                        }
                    }
                }
                _ => {}
            }
            i += close_idx + 1;
            continue;
        }
        md.push(bytes[i] as char);
        i += 1;
    }

    html_decode(&normalize(&md))
}

fn strip_script_and_style(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut cur = html;

    while let Some(start) = cur.to_lowercase().find("<style") {
        out.push_str(&cur[..start]);
        cur = &cur[start..];
        if let Some(end) = cur.to_lowercase().find("</style>") {
            cur = &cur[end + 8..];
        } else {
            break;
        }
    }
    out.push_str(cur);
    let mut final_out = String::with_capacity(out.len());
    cur = &out;

    while let Some(start) = cur.to_lowercase().find("<script") {
        final_out.push_str(&cur[..start]);
        cur = &cur[start..];
        if let Some(end) = cur.to_lowercase().find("</script>") {
            cur = &cur[end + 9..];
        } else {
            break;
        }
    }
    final_out.push_str(cur);
    final_out
}

fn strip_all_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(c);
        }
    }
    out
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let pattern = format!("{attr}=");
    let idx = lower.find(&pattern)?;
    let val_start = &tag[idx + pattern.len()..];
    let trimmed = val_start.trim();
    if let Some(stripped) = trimmed.strip_prefix('"') {
        let end = stripped.find('"')?;
        Some(stripped[..end].to_string())
    } else if let Some(stripped) = trimmed.strip_prefix('\'') {
        let end = stripped.find('\'')?;
        Some(stripped[..end].to_string())
    } else {
        let end = trimmed.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(trimmed.len());
        Some(trimmed[..end].to_string())
    }
}

pub fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut res = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        res.push(*c);
        let rem = chars.len() - 1 - i;
        if rem > 0 && rem.is_multiple_of(3) {
            res.push(',');
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_body() {
        let (text, omitted, total) = truncate("short", 100);
        assert_eq!(text, "short");
        assert_eq!(omitted, 0);
        assert_eq!(total, 5);
    }

    #[test]
    fn truncate_disabled_by_zero() {
        let body = "x".repeat(5000);
        let (text, omitted, total) = truncate(&body, 0);
        assert_eq!(text, body);
        assert_eq!(omitted, 0);
        assert_eq!(total, 5000);
    }

    #[test]
    fn truncate_on_paragraph_boundary() {
        let body = format!("First paragraph.\n\n{}\n\n{}", "a".repeat(60), "b".repeat(400));
        let (text, omitted, total) = truncate(&body, 120);
        assert!(text.ends_with(&"a".repeat(60)));
        assert!(omitted > 0);
        assert_eq!(total, body.len());
    }

    #[test]
    fn trim_quoted_reply_attribution() {
        let body = "Thanks, that works for me.\n\nOn Fri, 28 Aug 2026 at 09:12 UTC, Alice <alice@example.com> wrote:\n> Are you free on Tuesday?\n> Alice";
        let trimmed = trim_quoted_reply(body);
        assert_eq!(trimmed, "Thanks, that works for me.");
    }

    #[test]
    fn trim_quoted_reply_outlook_separator() {
        let body = "Agreed.\n\n-----Original Message-----\nFrom: Alice\n> old text";
        assert_eq!(trim_quoted_reply(body), "Agreed.");
    }

    #[test]
    fn normalize_collapses_blank_runs() {
        let normalized = normalize("one   \r\n\r\n\r\n\r\n\r\ntwo\t\r\n");
        assert_eq!(normalized, "one\n\n\ntwo");
        assert!(!normalized.contains('\r'));
    }

    #[test]
    fn html_to_markdown_drops_style_and_script() {
        let html = "<html><head><style>.a{color:red}</style></head><body><script>alert(1)</script><p>Hello <a href=\"https://x.test\">link</a></p></body></html>";
        let md = html_to_markdown(html);
        assert!(!md.contains("color:red"));
        assert!(!md.contains("alert(1)"));
        assert!(md.contains("https://x.test"));
    }
}
