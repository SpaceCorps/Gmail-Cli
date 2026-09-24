//! Parsing and building RFC 5322 / MIME messages.

use std::fs;
use std::path::Path;

use crate::error::{Error, Result};
use crate::util;

#[derive(Clone, Debug, Default)]
pub struct MimeAttachment {
    pub filename: String,
    pub mime_type: String,
    #[allow(dead_code)]
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct ParsedMessage {
    pub headers: Vec<(String, String)>,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub reply_to: Vec<String>,
    pub subject: Option<String>,
    pub date: Option<String>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub text_body: Option<String>,
    pub html_body: Option<String>,
    pub attachments: Vec<MimeAttachment>,
}

impl ParsedMessage {
    #[allow(dead_code)]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

pub struct DraftContent {
    pub from_email: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    pub body_format: String,
    pub attachment_paths: Vec<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

pub fn parse_rfc5322(raw: &[u8]) -> Result<ParsedMessage> {
    let (header_bytes, body_bytes) = split_headers_and_body(raw);
    let header_text = String::from_utf8_lossy(header_bytes);
    let raw_headers = parse_raw_headers(&header_text);

    let mut msg = ParsedMessage::default();
    let mut content_type = "text/plain".to_string();
    let mut content_transfer_encoding = "7bit".to_string();

    for (name, val) in raw_headers {
        let decoded_val = decode_rfc2047(&val);
        msg.headers.push((name.clone(), decoded_val.clone()));

        match name.to_lowercase().as_str() {
            "from" => msg.from = decoded_val,
            "to" => msg.to = parse_address_list(&decoded_val),
            "cc" => msg.cc = parse_address_list(&decoded_val),
            "bcc" => msg.bcc = parse_address_list(&decoded_val),
            "reply-to" => msg.reply_to = parse_address_list(&decoded_val),
            "subject" => msg.subject = Some(decoded_val),
            "date" => msg.date = Some(decoded_val),
            "message-id" => {
                let id = decoded_val.trim().trim_start_matches('<').trim_end_matches('>').to_string();
                msg.message_id = Some(id);
            }
            "in-reply-to" => {
                let id = decoded_val.trim().trim_start_matches('<').trim_end_matches('>').to_string();
                msg.in_reply_to = Some(id);
            }
            "references" => {
                msg.references = parse_references(&decoded_val);
            }
            "content-type" => content_type = decoded_val,
            "content-transfer-encoding" => content_transfer_encoding = decoded_val.to_lowercase(),
            _ => {}
        }
    }

    parse_body_parts(body_bytes, &content_type, &content_transfer_encoding, &mut msg);
    Ok(msg)
}

fn split_headers_and_body(raw: &[u8]) -> (&[u8], &[u8]) {
    for i in 0..raw.len() {
        if i + 3 < raw.len() && &raw[i..i + 4] == b"\r\n\r\n" {
            return (&raw[..i], &raw[i + 4..]);
        }
        if i + 1 < raw.len() && &raw[i..i + 2] == b"\n\n" {
            return (&raw[..i], &raw[i + 2..]);
        }
    }
    (raw, &[])
}

fn parse_raw_headers(text: &str) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_val = String::new();

    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if current_name.is_some() {
                current_val.push(' ');
                current_val.push_str(line.trim());
            }
        } else if let Some((name, val)) = line.split_once(':') {
            if let Some(n) = current_name.take() {
                headers.push((n, current_val.trim().to_string()));
            }
            current_name = Some(name.trim().to_string());
            current_val = val.trim().to_string();
        }
    }
    if let Some(n) = current_name {
        headers.push((n, current_val.trim().to_string()));
    }
    headers
}

fn parse_body_parts(body: &[u8], content_type: &str, transfer_encoding: &str, msg: &mut ParsedMessage) {
    let lower_ct = content_type.to_lowercase();
    if lower_ct.starts_with("multipart/")
        && let Some(boundary) = extract_parameter(content_type, "boundary")
    {
        let delimiter = format!("--{boundary}");
        let close_delimiter = format!("--{boundary}--");

        let body_str = String::from_utf8_lossy(body);
        let mut parts = Vec::new();

        for chunk in body_str.split(&delimiter) {
            let trimmed = chunk.trim();
            if trimmed.is_empty() || trimmed == "--" || chunk.starts_with(&close_delimiter) {
                continue;
            }
            parts.push(chunk.as_bytes());
        }

        for part in parts {
            let (part_hdr_bytes, part_body_bytes) = split_headers_and_body(part);
            let part_hdr_text = String::from_utf8_lossy(part_hdr_bytes);
            let part_headers = parse_raw_headers(&part_hdr_text);

            let mut part_ct = "text/plain".to_string();
            let mut part_cte = "7bit".to_string();
            let mut part_filename = None;
            let mut part_inline = false;

            for (hn, hv) in part_headers {
                let hnl = hn.to_lowercase();
                if hnl == "content-type" {
                    part_ct = hv.clone();
                    if let Some(fn_val) = extract_parameter(&hv, "name") {
                        part_filename = Some(decode_rfc2047(&fn_val));
                    }
                } else if hnl == "content-transfer-encoding" {
                    part_cte = hv.to_lowercase();
                } else if hnl == "content-disposition" {
                    if hv.to_lowercase().starts_with("inline") {
                        part_inline = true;
                    }
                    if let Some(fn_val) = extract_parameter(&hv, "filename") {
                        part_filename = Some(decode_rfc2047(&fn_val));
                    }
                }
            }

            if part_filename.is_some()
                || (!part_ct.to_lowercase().starts_with("text/") && !part_ct.to_lowercase().starts_with("multipart/"))
            {
                let decoded_bytes = decode_transfer_encoding(part_body_bytes, &part_cte);
                let fn_str = part_filename.unwrap_or_else(|| "attachment.bin".to_string());
                let mime_clean = part_ct.split(';').next().unwrap_or("application/octet-stream").trim().to_string();
                if !part_inline || !mime_clean.starts_with("text/") {
                    msg.attachments.push(MimeAttachment {
                        filename: fn_str,
                        mime_type: mime_clean,
                        data: decoded_bytes,
                    });
                }
            } else if part_ct.to_lowercase().starts_with("multipart/") {
                parse_body_parts(part_body_bytes, &part_ct, &part_cte, msg);
            } else {
                let decoded = decode_transfer_encoding_to_string(part_body_bytes, &part_cte);
                if part_ct.to_lowercase().starts_with("text/html") {
                    if msg.html_body.is_none() {
                        msg.html_body = Some(decoded);
                    }
                } else if msg.text_body.is_none() {
                    msg.text_body = Some(decoded);
                }
            }
        }
        return;
    }

    let decoded = decode_transfer_encoding_to_string(body, transfer_encoding);
    if lower_ct.starts_with("text/html") {
        msg.html_body = Some(decoded);
    } else {
        msg.text_body = Some(decoded);
    }
}

fn extract_parameter(header: &str, param_name: &str) -> Option<String> {
    for part in header.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=')
            && k.trim().eq_ignore_ascii_case(param_name)
        {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            return Some(v.to_string());
        }
    }
    None
}

fn decode_transfer_encoding(data: &[u8], encoding: &str) -> Vec<u8> {
    match encoding.trim().to_lowercase().as_str() {
        "base64" => {
            let s = String::from_utf8_lossy(data);
            util::base64_decode(&s).unwrap_or_else(|_| data.to_vec())
        }
        "quoted-printable" => decode_quoted_printable(data),
        _ => data.to_vec(),
    }
}

fn decode_transfer_encoding_to_string(data: &[u8], encoding: &str) -> String {
    let decoded = decode_transfer_encoding(data, encoding);
    String::from_utf8_lossy(&decoded).to_string()
}

fn decode_quoted_printable(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'=' {
            if i + 2 < data.len() && data[i + 1] == b'\r' && data[i + 2] == b'\n' {
                i += 3;
                continue;
            }
            if i + 1 < data.len() && data[i + 1] == b'\n' {
                i += 2;
                continue;
            }
            if i + 2 < data.len()
                && let Ok(b) = u8::from_str_radix(std::str::from_utf8(&data[i + 1..i + 2 + 1]).unwrap_or(""), 16)
            {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(data[i]);
        i += 1;
    }
    out
}

pub fn decode_rfc2047(s: &str) -> String {
    let mut result = String::new();
    let mut remainder = s;

    while let Some(start) = remainder.find("=?") {
        result.push_str(&remainder[..start]);
        remainder = &remainder[start..];

        if let Some(end) = remainder[2..].find("?=") {
            let full_encoded = &remainder[..end + 4];
            let inner = &remainder[2..end + 2];
            let parts: Vec<&str> = inner.split('?').collect();
            if parts.len() == 3 {
                let encoding = parts[1].to_lowercase();
                let encoded_text = parts[2];
                let decoded = match encoding.as_str() {
                    "b" => util::base64_decode(encoded_text)
                        .map(|b| String::from_utf8_lossy(&b).to_string())
                        .unwrap_or_else(|_| full_encoded.to_string()),
                    "q" => {
                        let qp_bytes = encoded_text.replace('_', " ").into_bytes();
                        let dec_bytes = decode_quoted_printable(&qp_bytes);
                        String::from_utf8_lossy(&dec_bytes).to_string()
                    }
                    _ => full_encoded.to_string(),
                };
                result.push_str(&decoded);
                remainder = &remainder[end + 4..];
                continue;
            }
        }
        result.push_str(&remainder[..2]);
        remainder = &remainder[2..];
    }
    result.push_str(remainder);
    result
}

pub fn parse_address_list(s: &str) -> Vec<String> {
    let mut addrs = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for c in s.chars() {
        if c == '"' {
            in_quotes = !in_quotes;
            current.push(c);
        } else if c == ',' && !in_quotes {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                addrs.push(trimmed.to_string());
            }
            current.clear();
        } else {
            current.push(c);
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        addrs.push(trimmed.to_string());
    }
    addrs
}

pub fn parse_references(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|tok| tok.trim().trim_start_matches('<').trim_end_matches('>').to_string())
        .filter(|tok| !tok.is_empty())
        .collect()
}

pub fn build_rfc5322(content: &DraftContent) -> Result<String> {
    let mut to_all = Vec::new();
    for a in &content.to {
        to_all.extend(parse_address_list(a));
    }
    let mut cc_all = Vec::new();
    for a in &content.cc {
        cc_all.extend(parse_address_list(a));
    }
    let mut bcc_all = Vec::new();
    for a in &content.bcc {
        bcc_all.extend(parse_address_list(a));
    }

    if to_all.is_empty() && cc_all.is_empty() && bcc_all.is_empty() {
        return Err(Error::invalid("A draft needs at least one recipient.").fix("Pass --to <address>."));
    }

    let has_attachments = !content.attachment_paths.is_empty();
    let is_markdown = content.body_format == "markdown";
    let is_html = content.body_format == "html";

    let mut out = String::new();
    out.push_str(&format!("From: {}\r\n", content.from_email));
    if !to_all.is_empty() {
        out.push_str(&format!("To: {}\r\n", to_all.join(", ")));
    }
    if !cc_all.is_empty() {
        out.push_str(&format!("Cc: {}\r\n", cc_all.join(", ")));
    }
    if !bcc_all.is_empty() {
        out.push_str(&format!("Bcc: {}\r\n", bcc_all.join(", ")));
    }

    out.push_str(&format!("Subject: {}\r\n", content.subject));
    out.push_str("MIME-Version: 1.0\r\n");

    if let Some(ref in_reply_to) = content.in_reply_to {
        out.push_str(&format!("In-Reply-To: <{in_reply_to}>\r\n"));
    }
    if !content.references.is_empty() {
        let refs: Vec<String> = content.references.iter().map(|r| format!("<{r}>")).collect();
        out.push_str(&format!("References: {}\r\n", refs.join(" ")));
    }

    let mixed_boundary = format!("----=_Mixed_{}", generate_id());
    let alt_boundary = format!("----=_Alt_{}", generate_id());

    if has_attachments {
        out.push_str(&format!("Content-Type: multipart/mixed; boundary=\"{mixed_boundary}\"\r\n\r\n"));
        out.push_str(&format!("--{mixed_boundary}\r\n"));
    }

    if is_markdown || is_html {
        out.push_str(&format!("Content-Type: multipart/alternative; boundary=\"{alt_boundary}\"\r\n\r\n"));

        let (plain_part, html_part) = if is_html {
            (strip_html(&content.body), content.body.clone())
        } else {
            (content.body.clone(), markdown_to_html(&content.body))
        };

        out.push_str(&format!("--{alt_boundary}\r\n"));
        out.push_str("Content-Type: text/plain; charset=utf-8\r\n");
        out.push_str("Content-Transfer-Encoding: base64\r\n\r\n");
        out.push_str(&format_base64_lines(&util::base64_encode(plain_part.as_bytes())));
        out.push_str("\r\n\r\n");

        out.push_str(&format!("--{alt_boundary}\r\n"));
        out.push_str("Content-Type: text/html; charset=utf-8\r\n");
        out.push_str("Content-Transfer-Encoding: base64\r\n\r\n");
        out.push_str(&format_base64_lines(&util::base64_encode(html_part.as_bytes())));
        out.push_str("\r\n\r\n");

        out.push_str(&format!("--{alt_boundary}--\r\n"));
    } else {
        out.push_str("Content-Type: text/plain; charset=utf-8\r\n");
        out.push_str("Content-Transfer-Encoding: base64\r\n\r\n");
        out.push_str(&format_base64_lines(&util::base64_encode(content.body.as_bytes())));
        out.push_str("\r\n");
    }

    if has_attachments {
        for path_str in &content.attachment_paths {
            let p = Path::new(path_str);
            if !p.is_file() {
                return Err(Error::invalid(format!("Attachment not found: {path_str}")));
            }
            let bytes =
                fs::read(p).map_err(|e| Error::other(format!("Failed reading attachment '{path_str}': {e}")))?;
            let filename = p.file_name().and_then(|s| s.to_str()).unwrap_or("attachment.bin");
            let mime_type = guess_mime(filename);

            out.push_str(&format!("\r\n--{mixed_boundary}\r\n"));
            out.push_str(&format!("Content-Type: {mime_type}; name=\"{filename}\"\r\n"));
            out.push_str(&format!("Content-Disposition: attachment; filename=\"{filename}\"\r\n"));
            out.push_str("Content-Transfer-Encoding: base64\r\n\r\n");
            out.push_str(&format_base64_lines(&util::base64_encode(&bytes)));
            out.push_str("\r\n");
        }
        out.push_str(&format!("\r\n--{mixed_boundary}--\r\n"));
    }

    Ok(out)
}

fn generate_id() -> String {
    let mut buf = [0u8; 8];
    util::random_bytes(&mut buf);
    format!("{:016x}", u64::from_ne_bytes(buf))
}

fn format_base64_lines(b64: &str) -> String {
    let mut out = String::with_capacity(b64.len() + b64.len() / 76 * 2);
    for chunk in b64.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        out.push_str("\r\n");
    }
    if out.ends_with("\r\n") {
        out.pop();
        out.pop();
    }
    out
}

fn guess_mime(filename: &str) -> &'static str {
    if let Some(ext) = Path::new(filename).extension().and_then(|s| s.to_str()) {
        match ext.to_lowercase().as_str() {
            "pdf" => "application/pdf",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "txt" => "text/plain",
            "html" | "htm" => "text/html",
            "csv" => "text/csv",
            "json" => "application/json",
            "zip" => "application/zip",
            _ => "application/octet-stream",
        }
    } else {
        "application/octet-stream"
    }
}

pub fn strip_html(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            text.push(c);
        }
    }
    text
}

pub fn markdown_to_html(md: &str) -> String {
    let mut html = String::from("<html><body>\n");
    let mut in_list = false;

    for para in md.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }

        if para.starts_with("- ") || para.starts_with("* ") {
            if !in_list {
                html.push_str("<ul>\n");
                in_list = true;
            }
            for line in para.lines() {
                let item = line.trim_start_matches(['-', '*', ' ']);
                html.push_str(&format!("  <li>{}</li>\n", render_inline_markdown(item)));
            }
            continue;
        }

        if in_list {
            html.push_str("</ul>\n");
            in_list = false;
        }

        if let Some(stripped) = para.strip_prefix("# ") {
            html.push_str(&format!("<h1>{}</h1>\n", render_inline_markdown(stripped)));
        } else if let Some(stripped) = para.strip_prefix("## ") {
            html.push_str(&format!("<h2>{}</h2>\n", render_inline_markdown(stripped)));
        } else if let Some(stripped) = para.strip_prefix("### ") {
            html.push_str(&format!("<h3>{}</h3>\n", render_inline_markdown(stripped)));
        } else if let Some(stripped) = para.strip_prefix("> ") {
            html.push_str(&format!("<blockquote><p>{}</p></blockquote>\n", render_inline_markdown(stripped)));
        } else {
            let lines: Vec<String> = para.lines().map(render_inline_markdown).collect();
            html.push_str(&format!("<p>{}</p>\n", lines.join("<br>\n")));
        }
    }

    if in_list {
        html.push_str("</ul>\n");
    }

    html.push_str("</body></html>");
    html
}

fn render_inline_markdown(s: &str) -> String {
    // Links: [text](url)
    let mut res = String::new();
    let mut cur = s;

    while let Some(start_bracket) = cur.find('[') {
        res.push_str(&cur[..start_bracket]);
        cur = &cur[start_bracket..];

        if let Some(end_bracket) = cur.find(']')
            && cur.get(end_bracket + 1..end_bracket + 2) == Some("(")
            && let Some(end_paren) = cur[end_bracket + 2..].find(')')
        {
            let label = &cur[1..end_bracket];
            let href = &cur[end_bracket + 2..end_bracket + 2 + end_paren];
            res.push_str(&format!("<a href=\"{href}\">{label}</a>"));
            cur = &cur[end_bracket + 2 + end_paren + 1..];
            continue;
        }
        res.push('[');
        cur = &cur[1..];
    }
    res.push_str(cur);
    res
}
