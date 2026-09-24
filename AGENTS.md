# AGENTS.md

Notes for whoever extends this next.

`gmail` is a native Rust 2024 CLI over the Google Gmail v1 REST API, built to be driven by an LLM agent.
It replaced a .NET global tool (`Gmail.Console`) by Niels Bosma and was re-architected under the SpaceCorps
organization to deliver native sub-3ms startup, zero runtime dependencies, robust OS keystore protection,
and hardened prompt-injection defenses.

For the manual the *agent* reads, run `gmail agent-readme` — that text lives in `src/readme.rs` and is the
tool's actual interface for its main audience. This file is for the human editing the source.

## Developer Commands

```bash
cargo build --release              # target/release/gmail
cargo test --locked                # unit tests + tests/cli.rs against offline in-process mock
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --check
cargo install --path . --locked    # put it on PATH
```

Use a throwaway config directory when testing so you never touch real credentials:

```bash
export GMAIL_CONFIG_DIR=$(mktemp -d) GMAIL_SECRET_STORE=plaintext GMAIL_ALLOW_PLAINTEXT_STORE=1
```

| Variable | Effect |
| --- | --- |
| `GMAIL_CONFIG_DIR` | Overrides the config/secrets directory location |
| `GMAIL_SECRET_STORE` | Forces a backend: `keychain`, `libsecret`, `dpapi`, `plaintext` |
| `GMAIL_ALLOW_PLAINTEXT_STORE=1` | Permits plaintext file store when no OS keystore is available |
| `GMAIL_API_URL` | Overrides Gmail REST base URL (how `tests/cli.rs` points at its mock) |
| `GMAIL_TOKEN_URL` | Overrides OAuth token exchange URL for testing |
| `GMAIL_PROFILE_URL` | Overrides Gmail profile endpoint for testing |
| `GMAIL_REVOKE_URL` | Overrides OAuth revocation URL for testing |
| `GMAIL_CLOCK_URL` | Overrides HTTP clock endpoint for doctor diagnostics testing |

There is deliberately **no** `GMAIL_ACCOUNT` environment variable. See the invariants below.

## Layout

```
src/
  main.rs          argument parsing, --json pre-scan, Clap errors -> structured invalid_input envelopes
  cli.rs           command hierarchy (clap derive); help text lives here
  commands/
    mod.rs         command routing and dispatch
    setup.rs       PKCE OAuth loopback listener, manual token exchange, profile verification
    account.rs     account list, remove
    search.rs      query search with bounded concurrent thread pool hydration
    message.rs     message get (clean/full/html/raw), untrusted content boundaries
    thread.rs      thread get, message chain rendering
    attachment.rs  attachment get, base64url decoding, directory traversal defense
    label.rs       label list
    draft.rs       draft create, reply, list, get, update, delete (Draft Safety Guarantee)
    doctor.rs      diagnostics: keystore probe, clock skew, reachability
  mail/
    rfc5322.rs     RFC 5322 / MIME builder and multipart parser
    body_input.rs  body input resolution (inline string vs file path with @)
    body_mode.rs   body extraction modes (clean, full, html, raw)
    attachment.rs  attachment path sanitization & directory escape checks
    renderer.rs    untrusted boundary delimiters & quoted text trimming
    reply.rs       reply recipient and reference math (References / In-Reply-To)
  client.rs        blocking HTTP (ureq 3.4 + rustls), backoff retries, status -> ErrorCode
  auth.rs          PKCE OAuth 2.0 flow, ScopeProfiles (draft, read), TokenManager
  account.rs       --account -> Resolved (name, email, client credentials, tokens)
  config.rs        config.yaml, atomic writes, cross-process lock via File::try_lock
  secrets.rs       OS keystores (Keychain, LibSecret, DPAPI, plaintext fallback)
  error.rs         ErrorCode enum (0-7) and structured Error envelope
  output.rs        YAML by default, JSON with --json, write_error, obj! macro
  util.rs          zero-dependency Base64/Base64Url, SHA-256 (FIPS 180-4), random bytes
  readme.rs        agent-readme text, commands summary, agent schemas
tests/cli.rs       drives the binary against an in-process TCP mock HTTP server (100% offline)
```

## Architectural Principles & Invariants

**1. Draft Safety Guarantee — Never Send Emails.**
The CLI strictly refuses to implement a send command. All draft operations create, update, or reply to drafts. Output reports `draft_created_not_sent` or `draft_updated_not_sent` and outputs the web URL `https://mail.google.com/mail/u/0/#drafts?compose={draftId}` so a human must review and manually dispatch the email.

**2. Prompt-Injection Defense.**
External email content is untrusted and can contain adversarial instructions designed to hijack LLM agents. All message bodies, snippets, and rendered emails are framed with delimiters:
```
--- untrusted email content begins ---
...
--- untrusted email content ends ---
```

**3. Strict Account Scoping.**
Every mailbox command strictly requires `--account <name>`. There is no implicit default account. A convenience default is how an agent working from a summarized transcript acts on the wrong inbox.

**4. OS Keystore Security.**
OAuth refresh tokens and client credentials are saved to the OS vault (`/usr/bin/security` on macOS, `secret-tool` on Linux, DPAPI on Windows). Plaintext fallback requires explicit `GMAIL_ALLOW_PLAINTEXT_STORE=1` and enforces POSIX permissions `0600`.

**5. Deterministic Offline Testing.**
`cargo test --locked` runs completely offline with zero network access. All integration tests use an in-process TCP mock server redirected via environment overrides (`GMAIL_API_URL`, etc.).

**6. Blocking HTTP without Async Overhead.**
`ureq 3.4` with `rustls` provides immediate execution and minimal binary size without the overhead of Tokio. Hydration concurrency in `search` is managed via scoped OS threads.

## Releasing

CI (`.github/workflows/ci.yml`) runs formatting, Clippy, and tests on Linux, macOS, and Windows. Releases are created via:
```bash
gh release create v1.0.0 --title v1.0.0 --notes "..."
```
Ensure `version` in `Cargo.toml` matches the release tag.
