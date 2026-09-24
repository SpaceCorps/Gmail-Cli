---
title: "Authentication Guide"
description: "Authentication methods, Google Cloud OAuth 2.0 setup, credential storage, and error remediation for developers and AI agents using the Gmail CLI."
author: "SpaceCorps"
date: "2026-09-24"
---

# Authentication Guide for Gmail CLI

This document outlines authentication methods, Google Cloud OAuth 2.0 configuration, secure OS credential storage, and error remediation for developers and autonomous AI agents using `gmail`.

## Overview
Gmail CLI interfaces directly with the official Google Gmail REST API (v1). Authentication is conducted via OAuth 2.0 Authorization Code flow with Proof Key for Code Exchange (PKCE, RFC 7636). OAuth client credentials and refresh tokens are stored in the host operating system's native credential vault or in an owner-restricted file (chmod 0600) with explicit opt-in.

## Prerequisites
1. A Google Account with Gmail enabled.
2. A Google Cloud Platform project with the **Gmail API** enabled.
3. An **OAuth 2.0 Client ID** configured as a **Desktop Application**.
4. Gmail CLI installed on your machine (`cargo install --git https://github.com/SpaceCorps/Gmail-Cli --locked`).

## Step 1: Google Cloud Console Setup

1. Navigate to the [Google Cloud Console](https://console.cloud.google.com/).
2. Create a new project (e.g. `Gmail CLI Automation`).
3. Enable the **Gmail API** under **APIs & Services > Library**.
4. Configure the **OAuth consent screen**:
   - User Type: **External** (or Internal for Google Workspace organizations).
   - App Name: `Gmail CLI`.
   - Add your Gmail address under **Test users** if keeping the app in Testing mode.
   - **Crucial Warning:** In "Testing" publishing status, Google automatically expires OAuth refresh tokens after exactly **7 days**. Move the app status to **"In Production"** (even without submitting for public verification) to obtain persistent, non-expiring refresh tokens.
5. Create credentials under **APIs & Services > Credentials**:
   - Click **Create Credentials > OAuth client ID**.
   - Application type: **Desktop app**.
   - Name: `Gmail CLI Client`.
   - Download or copy the **Client ID** and **Client Secret**.

## Step 2: Running Setup

Execute the interactive setup command:
```bash
gmail setup --account work
```
1. Paste your **Client ID** and **Client Secret** (or pass them via flags: `--client-id` and `--client-secret`).
2. Select your desired scope profile:
   - `draft` (default): `gmail.compose`, `gmail.modify`, `gmail.labels` (allows reading and creating drafts).
   - `read`: `gmail.readonly` (strict read-only access).
3. The CLI starts a local loopback HTTP listener on `127.0.0.1` and opens your default browser with a PKCE code challenge.
4. Authorize your account in Google.
5. Google redirects back to `http://127.0.0.1:<port>/?code=...&state=...`.
6. The CLI exchanges the authorization code for tokens, verifies your email address via `GET /gmail/v1/users/me/profile`, and securely stores the refresh token and credentials in your OS keystore.

## Headless & CI Environments

For headless Linux runners or Docker containers without a graphical browser:
1. Provide credentials and run with `--manual`:
```bash
gmail setup --account ci \
  --client-id "$GOOGLE_CLIENT_ID" \
  --client-secret "$GOOGLE_CLIENT_SECRET" \
  --manual
```
2. The CLI prints the authorization URL. Open it on any machine, complete the consent flow, and copy the resulting authorization code from the redirect URL back into the terminal prompt.

## OS Keystore Security

By default, Gmail CLI refuses to write plaintext secrets to disk:
- **macOS:** Stored in macOS Keychain via `/usr/bin/security`.
- **Linux:** Stored in Secret Service daemon via `secret-tool` (libsecret).
- **Windows:** Stored in Windows Data Protection API (DPAPI).
- **Plaintext Fallback:** On headless Linux servers without DBus/SecretService, set:
  ```bash
  export GMAIL_ALLOW_PLAINTEXT_STORE=1
  ```
  Secrets are saved in `~/.config/gmail/secrets.json` with strict POSIX permissions `0600` (owner read/write only).

## Multi-Account Scoping

To prevent catastrophic operations in the wrong inbox, every mailbox command strictly requires an explicit `--account <name>` (short `-a <name>`):
```bash
gmail search "is:unread" -a personal
gmail search "label:urgent" -a work
```
There is deliberately **no fallback default account** and no global environment variable to inject an active account.

## Token Refresh & Expiry Remediation

Tokens are refreshed automatically when expired. If a refresh fails because the token was revoked or expired (HTTP 400 `invalid_grant`):
```json
{
  "error": "OAuth refresh token invalid or revoked",
  "code": "auth_required",
  "remediation": "Re-authenticate by running 'gmail setup --account <name>'. Note: Google Cloud projects in 'Testing' status expire refresh tokens after 7 days; switch to 'In Production' to avoid weekly expiry."
}
```

## Diagnostic Verification

Verify clock skew, credential storage integrity, and Google endpoint reachability at any time:
```bash
gmail doctor
gmail doctor -a work
```
