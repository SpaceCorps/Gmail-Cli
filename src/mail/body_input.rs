use std::fs;
use std::io::Read;
use std::path::Path;

use crate::error::{Error, Result};

pub struct BodyInput;

impl BodyInput {
    pub fn resolve(body_path: Option<&str>, body_text: Option<&str>) -> Result<String> {
        if body_path.is_some() && body_text.is_some() {
            return Err(Error::invalid("Pass either --body or --body-text, not both.")
                .fix("Use --body <path> for a file, or --body-text for literal inline text."));
        }

        if let Some(text) = body_text {
            return Ok(text.to_string());
        }

        let Some(path) = body_path else {
            return Err(Error::invalid("No body supplied.")
                .fix("Write the body to a file and pass its path: --body ./reply.md (or --body - to read stdin, or --body-text for a one-liner)."));
        };

        if path == "-" {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| Error::other(format!("Failed reading stdin: {e}")))?;
            return Ok(buf);
        }

        let p = Path::new(path);
        if !p.is_file() {
            let elided = elide(path);
            return Err(Error::invalid(format!("--body expects a file path; '{elided}' is not a file."))
                .fix("Write the body to a file and pass its path, or use --body-text for literal text."));
        }

        let text = fs::read_to_string(p).map_err(|e| Error::other(format!("Failed reading file '{path}': {e}")))?;

        // Strip UTF-8 BOM if present
        if text.starts_with('\u{feff}') { Ok(text['\u{feff}'.len_utf8()..].to_string()) } else { Ok(text) }
    }
}

fn elide(value: &str) -> String {
    if value.len() <= 60 { value.to_string() } else { format!("{}...", &value[..57]) }
}
