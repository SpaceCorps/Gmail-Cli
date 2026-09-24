use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyMode {
    Markdown,
    Text,
    Html,
    None,
    Snippet,
}

pub struct BodyModes;

impl BodyModes {
    pub const NAMES: &'static [&'static str] = &["markdown", "text", "html", "none", "snippet"];

    pub fn parse(value: Option<&str>) -> Result<BodyMode> {
        match value.unwrap_or("markdown").to_lowercase().as_str() {
            "markdown" | "md" => Ok(BodyMode::Markdown),
            "text" | "plain" => Ok(BodyMode::Text),
            "html" => Ok(BodyMode::Html),
            "none" => Ok(BodyMode::None),
            "snippet" => Ok(BodyMode::Snippet),
            other => Err(Error::invalid(format!("Unknown body mode '{other}'."))
                .fix(format!("Use --body one of: {}", Self::NAMES.join(", ")))),
        }
    }

    #[allow(dead_code)]
    pub fn is_valid(value: Option<&str>) -> bool {
        match value {
            None => true,
            Some(v) => {
                let v = v.to_lowercase();
                Self::NAMES.contains(&v.as_str()) || v == "md" || v == "plain"
            }
        }
    }
}
