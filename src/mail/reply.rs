use std::collections::HashSet;

use crate::mail::renderer::{html_to_markdown, normalize, trim_quoted_reply};
use crate::mail::rfc5322::ParsedMessage;

#[derive(Clone, Debug)]
pub struct Reply {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub quoted_parent: String,
}

pub struct ReplyBuilder;

impl ReplyBuilder {
    pub fn build(parent: &ParsedMessage, own_email: &str, reply_all: bool) -> Reply {
        let raw_to = if !parent.reply_to.is_empty() {
            &parent.reply_to
        } else if !parent.from.is_empty() {
            std::slice::from_ref(&parent.from)
        } else {
            &[]
        };

        let mut seen = HashSet::new();
        seen.insert(extract_email_address(own_email).to_lowercase());

        let mut to_addresses = Vec::new();
        for addr in raw_to {
            let email = extract_email_address(addr).to_lowercase();
            if !email.is_empty() && seen.insert(email) {
                to_addresses.push(addr.clone());
            }
        }

        // Replying to own message: keep original recipients
        if to_addresses.is_empty() {
            for addr in &parent.to {
                to_addresses.push(addr.clone());
            }
        }

        let mut cc_addresses = Vec::new();
        if reply_all {
            for addr in parent.to.iter().chain(parent.cc.iter()) {
                let email = extract_email_address(addr).to_lowercase();
                if !email.is_empty() && seen.insert(email) {
                    cc_addresses.push(addr.clone());
                }
            }
        }

        let mut references = parent.references.clone();
        if let Some(ref mid) = parent.message_id
            && !references.iter().any(|r| r.eq_ignore_ascii_case(mid))
        {
            references.push(mid.clone());
        }

        Reply {
            to: to_addresses,
            cc: cc_addresses,
            subject: Self::subject(parent.subject.as_deref()),
            in_reply_to: parent.message_id.clone(),
            references,
            quoted_parent: Self::quote(parent),
        }
    }

    /// One "Re: " prefix, never "Re: Re:". Localized prefixes (SV:, AW:, VS:, etc.) are replaced.
    pub fn subject(parent_subject: Option<&str>) -> String {
        let mut subject = parent_subject.unwrap_or("").trim();

        loop {
            let trimmed = subject.trim_start();
            if let Some(rest) = strip_reply_prefix(trimmed) {
                subject = rest.trim_start();
            } else {
                break;
            }
        }

        format!("Re: {subject}")
    }

    pub fn quote(parent: &ParsedMessage) -> String {
        let sender = if !parent.from.is_empty() { parent.from.as_str() } else { "someone" };
        let when = parent.date.as_deref().unwrap_or("an earlier message");

        let md_body;
        let raw_body = if let Some(ref text) = parent.text_body {
            text.as_str()
        } else if let Some(ref html) = parent.html_body {
            md_body = html_to_markdown(html);
            &md_body
        } else {
            ""
        };
        let source = normalize(&trim_quoted_reply(raw_body));

        let mut quoted = String::new();
        quoted.push_str(&format!("On {when}, {sender} wrote:\n"));
        for line in source.split('\n') {
            if line.is_empty() {
                quoted.push_str(">\n");
            } else {
                quoted.push_str(&format!("> {line}\n"));
            }
        }

        quoted.trim_end().to_string()
    }
}

fn strip_reply_prefix(s: &str) -> Option<&str> {
    let prefixes = ["re", "sv", "aw", "antw", "vs", "ref", "fwd", "vb"];
    for p in prefixes {
        if s.to_lowercase().starts_with(p) {
            let mut rest = &s[p.len()..];
            // Optional [2] count
            if rest.starts_with('[')
                && let Some(close_idx) = rest.find(']')
            {
                let inside = &rest[1..close_idx];
                if inside.chars().all(|c| c.is_ascii_digit()) {
                    rest = &rest[close_idx + 1..];
                }
            }
            let rest_trimmed = rest.trim_start();
            if let Some(stripped) = rest_trimmed.strip_prefix(':') {
                return Some(stripped);
            }
        }
    }
    None
}

fn extract_email_address(s: &str) -> &str {
    if let Some(start) = s.find('<')
        && let Some(end) = s[start + 1..].find('>')
    {
        return &s[start + 1..start + 1 + end];
    }
    s.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_carries_exactly_one_prefix() {
        assert_eq!(ReplyBuilder::subject(Some("Quarterly numbers")), "Re: Quarterly numbers");
        assert_eq!(ReplyBuilder::subject(Some("Re: Quarterly numbers")), "Re: Quarterly numbers");
        assert_eq!(ReplyBuilder::subject(Some("RE: Re: Quarterly numbers")), "Re: Quarterly numbers");
        assert_eq!(ReplyBuilder::subject(Some("SV: Quarterly numbers")), "Re: Quarterly numbers");
        assert_eq!(ReplyBuilder::subject(Some("AW: SV: Quarterly numbers")), "Re: Quarterly numbers");
        assert_eq!(ReplyBuilder::subject(Some("Re[2]: Quarterly numbers")), "Re: Quarterly numbers");
        assert_eq!(ReplyBuilder::subject(Some("")), "Re: ");
    }
}
