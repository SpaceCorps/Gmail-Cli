use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "gmail",
    author = "SpaceCorps",
    version,
    about = "Gmail CLI built for LLM agents - search, messages, threads, attachments, drafts, and accounts",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[arg(long, global = true, help = "Emit JSON instead of YAML")]
    pub json: bool,

    #[arg(long, global = true, value_name = "FORMAT", help = "Output format: yaml or json")]
    pub format: Option<String>,

    #[arg(long, global = true, help = "Print HTTP method, URL and status to stderr (credentials redacted)")]
    pub verbose: bool,

    #[arg(long, global = true, help = "Disable colored output")]
    pub no_color: bool,

    #[arg(long, global = true, value_name = "SECONDS", help = "HTTP timeout in seconds (default: 100)")]
    pub timeout: Option<u64>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    #[command(about = "Set up Google Cloud OAuth credentials, with a walkthrough")]
    Setup(SetupArgs),

    #[command(about = "Log in to a Google account and store its credentials")]
    Login(AccountAddArgs),

    #[command(about = "Manage named Gmail accounts", alias = "accounts")]

    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },

    #[command(about = "Search a mailbox and return message summaries")]
    Search(SearchArgs),

    #[command(about = "Read individual messages")]
    Message {
        #[command(subcommand)]
        command: MessageCommand,
    },

    #[command(about = "Read whole conversations")]
    Thread {
        #[command(subcommand)]
        command: ThreadCommand,
    },

    #[command(about = "Download attachments")]
    Attachment {
        #[command(subcommand)]
        command: AttachmentCommand,
    },

    #[command(about = "Mailbox labels")]
    Label {
        #[command(subcommand)]
        command: LabelCommand,
    },

    #[command(about = "Create and manage drafts (nothing is ever sent)")]
    Draft {
        #[command(subcommand)]
        command: DraftCommand,
    },

    #[command(name = "agent-readme", about = "Print the operating manual for an LLM agent")]
    AgentReadme(AgentReadmeArgs),

    #[command(about = "Diagnose configuration, credentials and connectivity")]
    Doctor(DoctorArgs),
}

#[derive(Args, Debug)]
pub struct SetupArgs {
    #[arg(long, help = "Print the Google Cloud walkthrough and exit without storing anything")]
    pub show: bool,

    #[arg(long, value_name = "ID", help = "OAuth client id (skips the prompt)")]
    pub client_id: Option<String>,

    #[arg(long, value_name = "SECRET", help = "OAuth client secret (skips the prompt)")]
    pub client_secret: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum AccountCommand {
    #[command(about = "Log in to a Google account and store its credentials")]
    Add(AccountAddArgs),

    #[command(about = "List configured accounts and their token status")]
    List(AccountListArgs),

    #[command(about = "Verify one account's credentials against Google")]
    Test(AccountTestArgs),

    #[command(about = "Re-run consent for an existing account")]
    Reauth(AccountReauthArgs),

    #[command(about = "Revoke the grant at Google and delete local credentials")]
    Remove(AccountRemoveArgs),
}

#[derive(Args, Debug)]
pub struct AccountAddArgs {
    #[arg(value_name = "NAME", help = "Short name for this account (prompted if omitted)")]
    pub name: Option<String>,

    #[arg(
        long,
        value_name = "PROFILE",
        default_value = "draft",
        help = "read (search and read only) or draft (also create drafts)"
    )]
    pub scope_profile: String,

    #[arg(
        long,
        value_name = "PORT",
        default_value_t = 0,
        help = "Fixed loopback port for the OAuth redirect (default: ephemeral)"
    )]
    pub port: u16,
}

#[derive(Args, Debug)]
pub struct AccountListArgs {
    #[arg(long, help = "Probe each account against Google instead of reporting cached state")]
    pub check: bool,
}

#[derive(Args, Debug)]
pub struct AccountTestArgs {
    #[arg(value_name = "NAME", help = "Account name or email address")]
    pub name: String,
}

#[derive(Args, Debug)]
pub struct AccountReauthArgs {
    #[arg(value_name = "NAME", help = "Account name or email address")]
    pub name: String,

    #[arg(long, value_name = "PROFILE", help = "Change the scope profile while re-authorizing: read or draft")]
    pub scope_profile: Option<String>,

    #[arg(long, value_name = "PORT", default_value_t = 0, help = "Fixed loopback port for the OAuth redirect")]
    pub port: u16,
}

#[derive(Args, Debug)]
pub struct AccountRemoveArgs {
    #[arg(value_name = "NAME", help = "Account name or email address")]
    pub name: String,

    #[arg(long, help = "Delete local credentials but leave the grant active in the Google account")]
    pub local_only: bool,

    #[arg(long, help = "Skip the confirmation prompt")]
    pub yes: bool,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    #[arg(value_name = "QUERY", help = "Gmail search query, e.g. \"from:stripe.com has:attachment newer_than:30d\"")]
    pub query: Option<String>,

    #[arg(short = 'a', long, value_name = "ACCOUNT", help = "Account name or email address (required)")]
    pub account: Option<String>,

    #[arg(long, value_name = "N", default_value_t = 20, help = "Maximum messages to return (1-500)")]
    pub limit: usize,

    #[arg(long, value_name = "TOKEN", help = "Continue from a previous result's nextPageToken")]
    pub page_token: Option<String>,

    #[arg(long = "label", value_name = "LABEL", help = "Restrict to a label id, e.g. INBOX (repeatable)")]
    pub labels: Vec<String>,

    #[arg(long, value_name = "ADDRESS")]
    pub from: Option<String>,

    #[arg(long, value_name = "ADDRESS")]
    pub to: Option<String>,

    #[arg(long, value_name = "TEXT")]
    pub subject: Option<String>,

    #[arg(long, help = "Only unread messages")]
    pub unread: bool,

    #[arg(long, help = "Only messages with attachments")]
    pub has_attachment: bool,

    #[arg(long, value_name = "DATE", help = "Messages after this date (YYYY-MM-DD)")]
    pub after: Option<String>,

    #[arg(long, value_name = "DATE", help = "Messages before this date (YYYY-MM-DD)")]
    pub before: Option<String>,

    #[arg(long, help = "Include spam and trash in search results")]
    pub include_spam_trash: bool,

    #[arg(long, help = "Collapse to one entry per thread")]
    pub group_threads: bool,

    #[arg(long, value_name = "N", default_value_t = 5, help = "Parallel message fetches (1-20)")]
    pub concurrency: usize,
}

#[derive(Subcommand, Debug)]
pub enum MessageCommand {
    #[command(about = "Fetch one message with headers and body")]
    Get(MessageGetArgs),

    #[command(about = "List a message's attachments without downloading them")]
    Attachments(MessageAttachmentsArgs),
}

#[derive(Args, Debug)]
pub struct MessageGetArgs {
    #[arg(value_name = "MESSAGE-ID")]
    pub message_id: String,

    #[arg(short = 'a', long, value_name = "ACCOUNT", help = "Account name or email address (required)")]
    pub account: Option<String>,

    #[arg(long, value_name = "MODE", default_value = "markdown", help = "markdown, text, html, none or snippet")]
    pub body: String,

    #[arg(
        long,
        value_name = "N",
        default_value_t = 20000,
        help = "Truncate the body at this many characters (0 = no limit)"
    )]
    pub max_chars: usize,

    #[arg(long, help = "Keep the quoted parent message in a reply body")]
    pub keep_quotes: bool,

    #[arg(long, help = "Include every RFC 5322 header")]
    pub headers: bool,

    #[arg(long, value_name = "PATH", help = "Also write the raw RFC 5322 message to this file")]
    pub save_raw: Option<String>,
}

#[derive(Args, Debug)]
pub struct MessageAttachmentsArgs {
    #[arg(value_name = "MESSAGE-ID")]
    pub message_id: String,

    #[arg(short = 'a', long, value_name = "ACCOUNT", help = "Account name or email address (required)")]
    pub account: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum ThreadCommand {
    #[command(about = "Fetch a thread's messages in order")]
    Get(ThreadGetArgs),
}

#[derive(Args, Debug)]
pub struct ThreadGetArgs {
    #[arg(value_name = "THREAD-ID")]
    pub thread_id: String,

    #[arg(short = 'a', long, value_name = "ACCOUNT", help = "Account name or email address (required)")]
    pub account: Option<String>,

    #[arg(long, value_name = "MODE", default_value = "markdown", help = "markdown, text, html, none or snippet")]
    pub body: String,

    #[arg(
        long,
        value_name = "N",
        default_value_t = 8000,
        help = "Truncate each message body at this many characters (0 = no limit)"
    )]
    pub max_chars: usize,

    #[arg(long, value_name = "N", default_value_t = 20, help = "Return at most this many messages, most recent last")]
    pub max_messages: usize,

    #[arg(long, help = "Keep the quoted parent message in a reply body")]
    pub keep_quotes: bool,
}

#[derive(Subcommand, Debug)]
pub enum AttachmentCommand {
    #[command(about = "Save a message's attachments to disk")]
    Download(AttachmentDownloadArgs),
}

#[derive(Args, Debug)]
pub struct AttachmentDownloadArgs {
    #[arg(value_name = "MESSAGE-ID")]
    pub message_id: String,

    #[arg(short = 'a', long, value_name = "ACCOUNT", help = "Account name or email address (required)")]
    pub account: Option<String>,

    #[arg(long, help = "Download every attachment on the message")]
    pub all: bool,

    #[arg(long, value_name = "ID", help = "Download one specific attachment part")]
    pub attachment_id: Option<String>,

    #[arg(long, value_name = "PATTERN", help = "Download attachments whose filename contains this text")]
    pub name: Option<String>,

    #[arg(long, value_name = "DIR", default_value = ".", help = "Directory to write into")]
    pub out_dir: String,

    #[arg(long, help = "Replace existing files instead of adding a (2) suffix")]
    pub overwrite: bool,

    #[arg(
        long,
        value_name = "BYTES",
        default_value_t = 26214400,
        help = "Skip attachments larger than this (default: 25MB)"
    )]
    pub max_size: i64,

    #[arg(long, help = "Include inline images, which --all skips by default")]
    pub include_inline: bool,
}

#[derive(Subcommand, Debug)]
pub enum LabelCommand {
    #[command(about = "List labels with their ids")]
    List(LabelListArgs),
}

#[derive(Args, Debug)]
pub struct LabelListArgs {
    #[arg(short = 'a', long, value_name = "ACCOUNT", help = "Account name or email address (required)")]
    pub account: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum DraftCommand {
    #[command(about = "Create a new draft")]
    Create(DraftCreateArgs),

    #[command(about = "Create a correctly threaded reply draft")]
    Reply(DraftReplyArgs),

    #[command(about = "List existing drafts")]
    List(DraftListArgs),

    #[command(about = "Read a draft back")]
    Get(DraftGetArgs),

    #[command(about = "Replace a draft's content")]
    Update(DraftUpdateArgs),

    #[command(about = "Discard a draft")]
    Delete(DraftDeleteArgs),
}

#[derive(Args, Debug)]
pub struct DraftCreateArgs {
    #[arg(short = 'a', long, value_name = "ACCOUNT", help = "Account name or email address (required)")]
    pub account: Option<String>,

    #[arg(long = "to", value_name = "ADDRESS", help = "Recipient (repeatable, or comma-separated)")]
    pub to: Vec<String>,

    #[arg(long = "cc", value_name = "ADDRESS", help = "Cc recipient (repeatable, or comma-separated)")]
    pub cc: Vec<String>,

    #[arg(long = "bcc", value_name = "ADDRESS", help = "Bcc recipient (repeatable, or comma-separated)")]
    pub bcc: Vec<String>,

    #[arg(long, value_name = "SUBJECT", default_value = "")]
    pub subject: String,

    #[arg(long, value_name = "PATH", help = "Path to a file holding the message body, or - to read stdin")]
    pub body: Option<String>,

    #[arg(long, value_name = "TEXT", help = "Literal body text, for one-liners")]
    pub body_text: Option<String>,

    #[arg(long, help = "Treat the body as HTML rather than Markdown")]
    pub html: bool,

    #[arg(long, help = "Send a plain-text body only, with no HTML alternative")]
    pub plain: bool,

    #[arg(long = "attach", value_name = "PATH", help = "Attach a file (repeatable)")]
    pub attach: Vec<String>,

    #[arg(long, value_name = "DRAFT-ID", help = "Overwrite an existing draft instead of creating a new one")]
    pub replace_draft: Option<String>,
}

#[derive(Args, Debug)]
pub struct DraftReplyArgs {
    #[arg(value_name = "MESSAGE-ID", help = "The message being replied to")]
    pub message_id: String,

    #[arg(short = 'a', long, value_name = "ACCOUNT", help = "Account name or email address (required)")]
    pub account: Option<String>,

    #[arg(long, help = "Reply to everyone: the parent's To and Cc, minus your own address")]
    pub all: bool,

    #[arg(long, help = "Do not append the quoted parent message")]
    pub no_quote: bool,

    #[arg(long = "to", value_name = "ADDRESS", help = "Override the derived recipients")]
    pub to: Vec<String>,

    #[arg(long = "cc", value_name = "ADDRESS", help = "Add to the derived Cc list")]
    pub cc: Vec<String>,

    #[arg(long, value_name = "PATH", help = "Path to a file holding the message body, or - to read stdin")]
    pub body: Option<String>,

    #[arg(long, value_name = "TEXT", help = "Literal body text, for one-liners")]
    pub body_text: Option<String>,

    #[arg(long, help = "Treat the body as HTML rather than Markdown")]
    pub html: bool,

    #[arg(long, help = "Send a plain-text body only, with no HTML alternative")]
    pub plain: bool,

    #[arg(long = "attach", value_name = "PATH", help = "Attach a file (repeatable)")]
    pub attach: Vec<String>,
}

#[derive(Args, Debug)]
pub struct DraftListArgs {
    #[arg(short = 'a', long, value_name = "ACCOUNT", help = "Account name or email address (required)")]
    pub account: Option<String>,

    #[arg(long, value_name = "N", default_value_t = 20, help = "Maximum drafts to return")]
    pub limit: usize,

    #[arg(long, value_name = "TOKEN", help = "Continue from a previous result's nextPageToken")]
    pub page_token: Option<String>,
}

#[derive(Args, Debug)]
pub struct DraftGetArgs {
    #[arg(value_name = "DRAFT-ID")]
    pub draft_id: String,

    #[arg(short = 'a', long, value_name = "ACCOUNT", help = "Account name or email address (required)")]
    pub account: Option<String>,

    #[arg(long, value_name = "MODE", default_value = "markdown", help = "markdown, text, html or none")]
    pub body: String,

    #[arg(long, value_name = "N", default_value_t = 20000, help = "Truncate body at this many characters")]
    pub max_chars: usize,
}

#[derive(Args, Debug)]
pub struct DraftUpdateArgs {
    #[arg(value_name = "DRAFT-ID")]
    pub draft_id: String,

    #[arg(short = 'a', long, value_name = "ACCOUNT", help = "Account name or email address (required)")]
    pub account: Option<String>,

    #[arg(long = "to", value_name = "ADDRESS", help = "Recipient (repeatable, or comma-separated)")]
    pub to: Vec<String>,

    #[arg(long = "cc", value_name = "ADDRESS", help = "Cc recipient (repeatable, or comma-separated)")]
    pub cc: Vec<String>,

    #[arg(long = "bcc", value_name = "ADDRESS", help = "Bcc recipient (repeatable, or comma-separated)")]
    pub bcc: Vec<String>,

    #[arg(long, value_name = "SUBJECT")]
    pub subject: Option<String>,

    #[arg(long, value_name = "PATH", help = "Path to a file holding the message body, or - to read stdin")]
    pub body: Option<String>,

    #[arg(long, value_name = "TEXT", help = "Literal body text, for one-liners")]
    pub body_text: Option<String>,

    #[arg(long, help = "Treat the body as HTML rather than Markdown")]
    pub html: bool,

    #[arg(long, help = "Send a plain-text body only, with no HTML alternative")]
    pub plain: bool,

    #[arg(long = "attach", value_name = "PATH", help = "Attach a file (repeatable)")]
    pub attach: Vec<String>,
}

#[derive(Args, Debug)]
pub struct DraftDeleteArgs {
    #[arg(value_name = "DRAFT-ID")]
    pub draft_id: String,

    #[arg(short = 'a', long, value_name = "ACCOUNT", help = "Account name or email address (required)")]
    pub account: Option<String>,

    #[arg(long, help = "Skip the confirmation prompt")]
    pub yes: bool,
}

#[derive(Args, Debug)]
pub struct AgentReadmeArgs {
    #[arg(long, value_name = "FORMAT", help = "md (default), yaml or json")]
    pub format: Option<String>,
}

#[derive(Args, Debug)]
pub struct DoctorArgs {}
