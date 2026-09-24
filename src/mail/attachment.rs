use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

#[derive(Clone, Debug)]
pub struct AttachmentPart {
    pub attachment_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub inline: bool,
}

pub struct AttachmentWriter;

const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

impl AttachmentWriter {
    pub fn sanitize(filename: Option<&str>, index: usize) -> String {
        let raw = filename.unwrap_or("");
        // Take leaf under both slash conventions
        let leaf = raw.replace('\\', "/");
        let leaf = leaf.rsplit('/').next().unwrap_or("");

        // Keep only [A-Za-z0-9._ -]
        let filtered: String = leaf
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == ' ' || c == '-' { c } else { '_' })
            .collect();

        // Trim spaces and dots
        let trimmed = filtered.trim_matches(|c| c == ' ' || c == '.');
        let mut result = trimmed.to_string();

        if result.is_empty() {
            return format!("attachment-{index}.bin");
        }

        // Check Windows reserved device names on stem
        let stem = Path::new(&result).file_stem().and_then(|s| s.to_str()).unwrap_or("");

        if RESERVED_NAMES.iter().any(|&r| r.eq_ignore_ascii_case(stem)) {
            result = format!("_{result}");
        }

        if result.len() > 180 {
            result.truncate(180);
        }

        result
    }

    pub fn resolve_path(out_dir: &str, safe_name: &str, overwrite: bool) -> Result<PathBuf> {
        let root = std::fs::canonicalize(out_dir).unwrap_or_else(|_| {
            let p = Path::new(out_dir);
            if p.is_relative() {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(p)
            } else {
                p.to_path_buf()
            }
        });

        let candidate = root.join(safe_name);

        // Security check: ensure candidate does not escape root
        let root_str = root.to_string_lossy();
        let cand_str = candidate.to_string_lossy();
        let prefix = format!("{}{}", root_str, std::path::MAIN_SEPARATOR);
        if !cand_str.starts_with(&prefix) && cand_str != root_str {
            return Err(Error::other(format!("Refusing to write '{safe_name}' outside {root_str}.")));
        }

        if overwrite || !candidate.exists() {
            return Ok(candidate);
        }

        let stem = candidate.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let ext = candidate.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}")).unwrap_or_default();

        for n in 2..1000 {
            let next = root.join(format!("{stem} ({n}){ext}"));
            if !next.exists() {
                return Ok(next);
            }
        }

        Err(Error::other(format!("Too many files named like '{safe_name}' in {root_str}.")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_reduces_to_safe_leaf() {
        assert_eq!(AttachmentWriter::sanitize(Some("invoice.pdf"), 1), "invoice.pdf");
        assert_eq!(AttachmentWriter::sanitize(Some("../../.ssh/authorized_keys"), 1), "authorized_keys");
        assert_eq!(AttachmentWriter::sanitize(Some("..\\..\\windows\\system32\\evil.dll"), 1), "evil.dll");
        assert_eq!(AttachmentWriter::sanitize(Some("/etc/passwd"), 1), "passwd");
        assert_eq!(AttachmentWriter::sanitize(Some("C:\\Windows\\notepad.exe"), 1), "notepad.exe");
        assert_eq!(AttachmentWriter::sanitize(Some("report:2026*final?.pdf"), 1), "report_2026_final_.pdf");
        assert_eq!(AttachmentWriter::sanitize(Some("  spaced  .pdf  "), 1), "spaced  .pdf");
    }

    #[test]
    fn sanitize_fallbacks() {
        assert_eq!(AttachmentWriter::sanitize(Some(""), 7), "attachment-7.bin");
        assert_eq!(AttachmentWriter::sanitize(None, 7), "attachment-7.bin");
        assert_eq!(AttachmentWriter::sanitize(Some(".."), 7), "attachment-7.bin");
        assert_eq!(AttachmentWriter::sanitize(Some("..."), 7), "attachment-7.bin");
        assert_eq!(AttachmentWriter::sanitize(Some("/"), 7), "attachment-7.bin");
    }

    #[test]
    fn sanitize_escapes_reserved_device_names() {
        assert!(AttachmentWriter::sanitize(Some("CON.pdf"), 1).starts_with('_'));
        assert!(AttachmentWriter::sanitize(Some("nul.txt"), 1).starts_with('_'));
        assert!(AttachmentWriter::sanitize(Some("LPT1.doc"), 1).starts_with('_'));
    }

    #[test]
    fn sanitize_caps_length() {
        let name = format!("{}.pdf", "a".repeat(500));
        assert_eq!(AttachmentWriter::sanitize(Some(&name), 1).len(), 180);
    }
}
