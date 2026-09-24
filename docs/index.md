---
title: "Gmail CLI"
description: "A blazing fast native command-line tool and agent interface for the Google Gmail API (v1). Built in Rust 2024 for developers and autonomous AI agents."
author: "SpaceCorps"
date: "2026-09-24"
canonical: "https://spacecorps.github.io/Gmail-Cli/index.md"
---

# Gmail CLI

A blazing fast native command-line tool and agent interface for the Google Gmail API (v1). Built in Rust 2024 for developers and autonomous AI agents.

## Quickstart

```bash
# Set up a new Gmail account via PKCE OAuth loopback
gmail setup --account work

# Search unread messages
gmail search "is:unread newer_than:2d" --account work

# Read a specific message with untrusted boundary protection
gmail message get 18e4f1a2b3c4d5e6 --account work

# Draft a reply (Draft safety: drafts are created, never sent!)
gmail draft reply 18e4f1a2b3c4d5e6 --body "Thank you, I will review this shortly." --account work
```

## Features

- **Blazing Fast Native Rust**: Sub-millisecond startup times with zero runtime dependencies.
- **AI Agent Native**: Structured JSON output (`--json`), standardized error envelopes, and untrusted email delimiter boundaries to protect against prompt injection.
- **Draft Safety Guarantee**: The CLI only creates, updates, replies to, and deletes drafts. Sending emails is permanently disallowed without human confirmation via webmail.
- **Secure Keystore Integration**: Refresh tokens and credentials stored in native macOS Keychain, Windows DPAPI, or Linux Secret Service.
- **Strict Account Scoping**: Explicit `--account` required on every mailbox command to prevent accidental cross-tenant actions.

## When to Use This CLI

Use `gmail` whenever you need to:
- Programmatically search, hydrate, and triage incoming emails from bash or CI/CD pipelines.
- Feed email contents safely to autonomous AI agents without risk of adversarial prompt injection.
- Prepare, reply to, and organize draft responses for human review.
- Download and inspect email attachments safely with directory escape validation.
- Audit Gmail labels and monitor mailbox state across multiple accounts.

## Documentation Links

- [llms.txt](https://spacecorps.github.io/Gmail-Cli/llms.txt)
- [Full Agent Manual](https://spacecorps.github.io/Gmail-Cli/llms-full.txt)
- [Authentication Guide](https://spacecorps.github.io/Gmail-Cli/auth.md)
- [Pricing & Licensing](https://spacecorps.github.io/Gmail-Cli/pricing.md)
- [GitHub Repository](https://github.com/SpaceCorps/Gmail-Cli)
