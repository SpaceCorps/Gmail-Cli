#![allow(unused_imports)]

pub mod attachment;
pub mod body_input;
pub mod body_mode;
pub mod renderer;
pub mod reply;
pub mod rfc5322;

pub use attachment::{AttachmentPart, AttachmentWriter};
pub use body_input::BodyInput;
pub use body_mode::{BodyMode, BodyModes};
pub use renderer::MessageRenderer;
pub use reply::{Reply, ReplyBuilder};
pub use rfc5322::{DraftContent, ParsedMessage, build_rfc5322, parse_rfc5322};

pub struct GmailFields;

impl GmailFields {
    pub const STRUCTURE: &'static str = "id,threadId,labelIds,snippet,internalDate,sizeEstimate,\
payload(mimeType,filename,headers,body/size,body/attachmentId,\
parts(partId,mimeType,filename,headers,body/size,body/attachmentId,\
parts(partId,mimeType,filename,headers,body/size,body/attachmentId,\
parts(partId,mimeType,filename,headers,body/size,body/attachmentId))))";

    #[allow(dead_code)]
    pub const SUMMARY_HEADERS: &'static [&'static str] =
        &["From", "To", "Cc", "Subject", "Date", "Message-ID", "Reply-To"];
}
