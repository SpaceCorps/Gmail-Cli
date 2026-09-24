# Gmail CLI

[![Release](https://img.shields.io/github/v/release/SpaceCorps/Gmail-Cli?color=blue&label=version)](https://github.com/SpaceCorps/Gmail-Cli/releases/latest)
[![CI](https://github.com/SpaceCorps/Gmail-Cli/actions/workflows/ci.yml/badge.svg)](https://github.com/SpaceCorps/Gmail-Cli/actions/workflows/ci.yml)
[![Docs](https://img.shields.io/badge/docs-online-success)](https://spacecorps.github.io/Gmail-Cli/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A blazing fast, native command-line tool and agent interface for the [Google Gmail API](https://developers.google.com/gmail/api) (v1). Built in Rust 2024 for developers and autonomous AI workflows.

---

## Highlights

- ⚡ **Sub-3ms Startup**: Compiled as a native static binary with zero runtime dependencies. Executes in ~1–3 ms (compared to ~75 ms for managed runtimes).
- 🛡️ **Draft Safety Guarantee**: The CLI strictly creates, updates, and replies to drafts; it never sends emails. All drafts provide a direct Google Mail webcompose link for human verification.
- 🔐 **OS Keystore Integration**: Stores OAuth credentials and refresh tokens securely in native OS vaults (macOS Keychain, Linux Secret Service / libsecret, Windows DPAPI).
- 🤖 **Prompt-Injection Defense**: Demarcates external email content (subjects, bodies, snippets, senders) with standardized delimiters (`--- untrusted email content begins ---`) to protect LLM agents.
- 📂 **Attachment Traversal Protection**: Sanitizes attachment filenames and rejects directory escape attempts (`../`) during extraction.
- 🎯 **Strict Account Scoping**: Requires an explicit `--account <name>` on every mailbox command to prevent accidental cross-account actions.
- 📊 **Machine-Readable Output**: Clean YAML stdout by default, raw JSON via `--json`, and structured error envelopes with stable exit codes.

---

## Installation

### Using Cargo

```bash
cargo install --git https://github.com/SpaceCorps/Gmail-Cli --locked
```

### Pre-built Standalone Binaries

Download standalone binary archives directly from the [GitHub Releases](https://github.com/SpaceCorps/Gmail-Cli/releases/latest) page:

| Platform | Architecture | Binary Package |
|:---|:---|:---|
| **macOS** | Apple Silicon (`aarch64`) | [`gmail-v1.0.0-aarch64-apple-darwin.tar.gz`](https://github.com/SpaceCorps/Gmail-Cli/releases/download/v1.0.0/gmail-v1.0.0-aarch64-apple-darwin.tar.gz) |
| **macOS** | Intel (`x86_64`) | [`gmail-v1.0.0-x86_64-apple-darwin.tar.gz`](https://github.com/SpaceCorps/Gmail-Cli/releases/download/v1.0.0/gmail-v1.0.0-x86_64-apple-darwin.tar.gz) |
| **Linux** | x86_64 (musl static) | [`gmail-v1.0.0-x86_64-unknown-linux-musl.tar.gz`](https://github.com/SpaceCorps/Gmail-Cli/releases/download/v1.0.0/gmail-v1.0.0-x86_64-unknown-linux-musl.tar.gz) |
| **Windows**| x64 (MSVC) | [`gmail-v1.0.0-x86_64-pc-windows-msvc.zip`](https://github.com/SpaceCorps/Gmail-Cli/releases/download/v1.0.0/gmail-v1.0.0-x86_64-pc-windows-msvc.zip) |

---

## Quickstart

### 1. Authenticate via Google OAuth

Run `gmail setup` to launch your browser, complete the PKCE OAuth consent flow, verify your identity against the Gmail Profile API, and save credentials in your OS vault:

```bash
# Interactive PKCE OAuth setup
gmail setup --account work

# Setup with existing Google Cloud OAuth Client credentials
gmail setup --account work --client-id "$CLIENT_ID" --client-secret "$CLIENT_SECRET"

# Headless / SSH server setup (manual code paste)
gmail setup --account ci --manual
```

> **Note on 7-Day Expiration:** Google Cloud projects with "Testing" publishing status automatically expire refresh tokens after exactly 7 days. Move your Google Cloud app status to **"In Production"** to avoid weekly token expiration.

### 2. Search Messages

```bash
# Search unread messages
gmail search "is:unread newer_than:7d" --account work

# Limit hydrated results (default 10)
gmail search "from:boss@example.com" --limit 5 --account work

# Fast query without message body hydration
gmail search "label:INBOX" --no-hydrate --account work
```

### 3. Read Messages & Threads

```bash
# Read message content with untrusted boundary protection
gmail message get 18e4f1a2b3c4d5e6 --account work

# Inspect an entire conversation thread
gmail thread get 18e4f1a2b3c4d5e6 --account work

# Download an attachment safely
gmail attachment get 18e4f1a2b3c4d5e6 att_987654 --output ./downloads --account work
```

### 4. Create and Reply to Drafts

```bash
# Create a new draft (created, never sent!)
gmail draft create --to "partner@example.com" --subject "Project Update" --body "Draft notes" --account work

# Compose a draft reply (auto-populates In-Reply-To and References)
gmail draft reply 18e4f1a2b3c4d5e6 --body "Confirmed, see you there." --account work

# Reply-All to all thread participants
gmail draft reply 18e4f1a2b3c4d5e6 --reply-all --body "Acknowledged by team." --account work
```

---

## Command Reference

Every command accessing the Gmail API accepts `--account <name>` (short `-a <name>`).

### Setup & Accounts

| Command | Description |
|:---|:---|
| `gmail setup --account <name>` | Authenticate via PKCE OAuth 2.0 loopback and store credentials in OS keystore |
| `gmail account list` | List configured accounts with email identities and scope profiles |
| `gmail account remove <name>` | Remove an account and purge its tokens from the local keystore |
| `gmail doctor [--account <name>]` | Run diagnostics on keystore, clock skew, and API reachability |

### Mailbox Operations

| Command | Description |
|:---|:---|
| `gmail search <query> [--limit <n>]` | Search messages with concurrent bounded body hydration |
| `gmail message get <id>` | Fetch message content (modes: `clean`, `full`, `html`, `raw`) |
| `gmail thread get <id>` | Retrieve an entire conversation thread with message chain |
| `gmail attachment get <msg-id> <att-id>` | Download an attachment with path sanitization |
| `gmail label list` | List all system and user mailbox labels |

### Draft Operations (Draft Safety Guarantee)

| Command | Description |
|:---|:---|
| `gmail draft create --to <t> --subject <s> --body <b>` | Create a new draft message |
| `gmail draft reply <msg-id> --body <b> [--reply-all]` | Compose a reply draft with RFC 5322 threading |
| `gmail draft list` | List existing drafts |
| `gmail draft get <draft-id>` | Inspect an existing draft message |
| `gmail draft update <draft-id> --body <b>` | Update an existing draft's contents |
| `gmail draft delete <draft-id>` | Delete a draft message |

---

## Output Formats & AI Agent Readiness

Commands format stdout as clean YAML by default. Pass `--json` when parsing outputs with `jq`, Python, or LLM agent tool-calling loops:

```bash
gmail search "is:unread" --account work --json | jq .results[0].snippet
```

### Machine-Readable Error Envelopes

Errors are output to `stderr` as structured envelopes with stable exit codes:

```json
{
  "error": "OAuth refresh token invalid or revoked",
  "code": "auth_required",
  "remediation": "Re-authenticate by running 'gmail setup --account work'. Note: Google Cloud projects in 'Testing' status expire refresh tokens after 7 days; switch to 'In Production' to avoid weekly expiry."
}
```

| Exit Code | Error Symbol | Description |
|:---|:---|:---|
| `0` | `ok` | Command completed successfully |
| `1` | `error` | General or unclassified error |
| `2` | `network` | Network connectivity failure (retry with backoff) |
| `3` | `auth_required` | Authentication missing, invalid, or expired |
| `4` | `not_found` | Requested message, thread, draft, or attachment not found |
| `5` | `rate_limited` | Google API rate limit reached (HTTP 429 / 403 quota) |
| `6` | `invalid_input` | Parameter schema or argument validation failed |
| `7` | `no_account` | Specified account not configured in keystore |

### Agent Discovery

Inspect built-in agent manuals directly from the CLI:

```bash
gmail agent-readme          # Human-readable markdown guide
gmail agent-readme --json   # Machine-readable schemas and rules
```

For web-based LLMs and crawlers, refer to [llms.txt](https://spacecorps.github.io/Gmail-Cli/llms.txt) and [llms-full.txt](https://spacecorps.github.io/Gmail-Cli/llms-full.txt).

---

## Configuration & Environment Variables

| Variable | Description | Default |
|:---|:---|:---|
| `GMAIL_CONFIG_DIR` | Custom directory path for `config.yaml` | `~/.config/gmail` (or OS equivalent) |
| `GMAIL_SECRET_STORE` | Force specific credential store: `keychain`, `libsecret`, `dpapi`, `plaintext` | Auto-detected |
| `GMAIL_ALLOW_PLAINTEXT_STORE` | Set to `1` to allow a chmod 0600 file store on headless Linux | `0` |
| `GMAIL_API_URL` | Override Gmail REST base URL (useful for testing against mocks) | `https://gmail.googleapis.com` |
| `GMAIL_TOKEN_URL` | Override OAuth token exchange URL | `https://oauth2.googleapis.com/token` |

---

## Contributing & License

Ported from the original .NET console utility by [Niels Bosma](https://github.com/nielsbosma/Gmail.Console).

Maintained by [SpaceCorps](https://github.com/SpaceCorps). Released under the [MIT License](LICENSE).
