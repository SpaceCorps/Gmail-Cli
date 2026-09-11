# AGENTS.md

Notes for whoever extends this next. Read [spec.html](spec.html) first — it carries the
reasoning behind the design decisions, and this file assumes you know them.

`Gmail.Console` is a .NET 10 global tool (`gmail`) over the Gmail REST API, built to be driven
by an LLM agent. It follows the house conventions in `../ConsoleGuidelines.md`:
Spectre.Console.Cli, one command class per file under `src/Commands/`, YAML-first output,
no solution file, published to NuGet from a GitHub Release.

## Commands

```bash
dotnet build src/Gmail.Console.csproj -c Release
dotnet test  tests/Gmail.Console.Tests/Gmail.Console.Tests.csproj -c Release

# run without installing
dotnet src/bin/Release/net10.0/gmail.dll <args>

# install from source, or reinstall after a change
dotnet tool uninstall --global Gmail.Console
dotnet pack src/Gmail.Console.csproj -c Release -p:Version=0.1.2 --output ./nupkgs
dotnet tool install --global --add-source ./nupkgs Gmail.Console --version 0.1.2
```

Two things about that cycle, both of which have already caught someone out:

- **`dotnet build` does not update the installed `gmail`.** The global tool runs from an
  installed package, not from `src/bin`. Editing source and rebuilding changes nothing about
  the command on PATH — you must repack and reinstall, and then verify against the installed
  command (`gmail setup --show`), not the build output.
- **Bump the version every time.** Reinstalling the same version number can be served from the
  NuGet cache, silently giving you the old package back.

If `dotnet tool uninstall` fails with *"Access to the path ... is denied"*, a `gmail` process
is still running — commonly `gmail setup` or `gmail account add` sitting at a prompt. Close it
and retry.

Use a throwaway config directory when testing so you never touch real credentials:

```bash
export GMAIL_CONFIG_DIR=/tmp/gmail-test
```

| Variable | Effect |
| --- | --- |
| `GMAIL_CONFIG_DIR` | Overrides the config/secrets location |
| `GMAIL_SECRET_STORE` | Forces a backend: `dpapi`, `keychain`, `libsecret`, `plaintext` |
| `GMAIL_ALLOW_PLAINTEXT_STORE=1` | Permits the plaintext fallback where no keystore exists |

## Layout

```
src/
  Program.cs                 command registration, UTF-8 console, exception handler
  Infrastructure/
    GlobalSettings.cs        settings hierarchy (see below)
    GmailCommand.cs          GmailCommand<T> and MailboxCommand<T> base classes
    GmailApiClient.cs        HTTP, retry/backoff, HTTP status -> ErrorCode translation
    GmailException.cs        ErrorCode enum; exit codes and the `code:` field are the same thing
    OutputHelper.cs          YAML/JSON serialization, null pruning, literal block scalars
  Auth/
    OAuthFlow.cs             loopback listener + PKCE, token exchange
    TokenManager.cs          access token cache, refresh under lock, revoke
    AccountResolver.cs       --account -> ResolvedAccount, or a no_account error
    ScopeProfiles.cs         read / draft scope bundles
    Credentials.cs           ClientCredentials and StoredTokens, both keystore-backed
    GmailProfile.cs          users.getProfile with a bare access token (pre-account)
  Storage/
    ISecretStore.cs          Get/Set/Delete + SecretKeys
    SecretStoreFactory.cs    platform selection; refuses to fall back silently
    *SecretStore.cs          DPAPI (Windows), Keychain (macOS), libsecret (Linux), plaintext
    ConfigStore.cs           config.yaml, paths, atomic writes, 0600 on Unix
    FileLock.cs              cross-process lock
  Mail/
    MessageRenderer.cs       Gmail JSON + MimeKit -> output dictionaries; bodies, quotes, truncation
    MimeBuilder.cs           DraftContent -> MimeMessage -> base64url raw
    ReplyBuilder.cs          reply headers and recipient math
    AttachmentWriter.cs      filename sanitization and collision handling
    BodyInput.cs             --body path / stdin / --body-text resolution
    BodyMode.cs, GmailFields.cs
  Commands/                  one file per command, mirroring the CLI tree
tests/Gmail.Console.Tests/   51 tests over the four areas with real logic
```

## How a command is wired

Two base classes, both in `Infrastructure/GmailCommand.cs`:

```csharp
// Anything that does not touch a mailbox: setup, account *, doctor, agent-readme.
GmailCommand<TSettings>   where TSettings : GlobalSettings
    protected abstract Task<object?> RunAsync(CommandContext, TSettings, CancellationToken)

// Anything that does: it resolves --account and hands you an authenticated client.
MailboxCommand<TSettings> where TSettings : AccountSettings
    protected abstract Task<object?> RunAsync(GmailApiClient, TSettings, CancellationToken)
```

Return a `Dictionary<string, object?>` and it is serialized to stdout in the requested format.
Return `null` to write nothing (used by `agent-readme` and `setup --show`, which print text
themselves). Throw `GmailException` to fail — the base class writes the error envelope to
stderr and returns the matching exit code. Never call `Environment.Exit`, never write results
to stdout directly, and never catch an exception just to print it.

### Settings hierarchy

```
CommandSettings                     (Spectre)
└── GlobalSettings                  --no-color --verbose --timeout   (Format declared, NOT an option)
    └── OutputSettings              --format yaml|json
        └── AccountSettings         -a|--account
            └── DraftContentSettings  --body --body-text --html --plain --attach
```

`GlobalSettings.Format` is a `virtual` property with **no** `[CommandOption]`. That is
deliberate — see the trap below. Pick the lowest base class that gives you what you need.

### Adding a command

1. Create `src/Commands/<Area>/<Verb>Command.cs`, deriving from `MailboxCommand<Settings>`
   (or `GmailCommand<Settings>`), with a nested `public sealed class Settings`.
2. Options get `[CommandOption("--name <VALUE>")]` + `[Description]`, arguments get
   `[CommandArgument(0, "<NAME>")]`. Validate in `Settings.Validate()`, returning
   `ValidationResult.Error` — that path already maps to exit code 6.
3. Register it in `Program.cs` under the right branch, with a `.WithDescription`.
4. Add the row to the command table in `README.md` and to `AgentReadmeCommand.Readme`.
   **The agent-readme is the tool's actual interface for its main audience** — a command an
   agent cannot discover there effectively does not exist.
5. If the command writes, call `ScopeProfiles.RequireDraft(...)` first.

## Invariants — do not casually revert these

Each was an explicit decision, and each looks like an inconvenience until you know why.

**`--account` is required everywhere.** No stored default, no environment variable, no
fallback when only one account exists. A convenience default is exactly how an agent working
from a summarized transcript drafts from the wrong mailbox — silent locally, discovered by the
recipient. Every mailbox command also echoes `account:` in its output for the same reason.

**`--body` takes a file path, not text.** Inverted from CLI convention on purpose: prose passed
as a shell argument eventually loses a quote, a newline or an `ä`, and the damage is invisible
until a human reads the draft. `--body-text` is the explicit escape hatch. A `--body` value
that is not an existing file must fail loudly rather than be sent as the message.

**There is no send command, and there must not be one without a deliberate decision.**
`gmail.compose` grants sending and there is no draft-only scope, so the guard against an agent
emailing as the user lives in the command surface, not the OAuth grant. If it is ever added it
should be `gmail draft send <id> --confirm` — sending something already written and reviewed,
never composing and sending in one call.

**Message bodies stay wrapped in the untrusted-content delimiter.** Everything returned from a
mailbox was written by a third party who may be hostile, and it lands straight in a model's
context. `MessageRenderer.UntrustedOpen` / `UntrustedClose` and the corresponding rule in
`agent-readme` are a pair; do not remove one.

**Secrets never touch `config.yaml`.** Config holds names, addresses, scope profiles, dates.
Anything bearer-shaped goes through `ISecretStore`. When no keystore is available the tool
refuses to start rather than silently writing a file.

**`--verbose` must never print an `Authorization` header, refresh token or client secret.**
`GmailApiClient` logs method, URL, status and byte count only. Keep it that way.

## Traps in this codebase

**The `FileLock` is not reentrant, and `TokenManager` takes it.** `GetAccessTokenAsync` acquires
`ConfigStore.LockPath` when it needs to refresh. Any code holding that lock and then making an
API call will deadlock until the 15s timeout. Today `AddCommand`, `ReauthCommand` and
`RemoveCommand` hold it only around `tokens.Save` + `ConfigStore.Save`, and `RemoveCommand`
revokes *before* taking it. Keep every network call outside the lock.

**Spectre discovers options across the whole inheritance chain.** Declaring the same
`[CommandOption]` on a base and a derived settings class — with `new` *or* `override` — makes
the app refuse to start with "Option --format is duplicated". That is why `GlobalSettings`
declares `Format` unattributed and `OutputSettings` / `AgentReadmeCommand.Settings` attach the
option at their own leaf with different vocabularies (`yaml|json` vs `md|yaml`). If you need a
per-command variant of a shared option, follow that shape.

**The root namespace is `Gmail.Console`, which shadows `System.Console`.** Write
`System.Console.WriteLine` explicitly. `OutputHelper.Status(...)` is the preferred route for
anything human-facing anyway.

**Nulls are pruned recursively before serialization.** Both YamlDotNet's `OmitNull` and
`System.Text.Json`'s `WhenWritingNull` apply to object *properties*, and every payload here is
a dictionary — without `OutputHelper.Prune` an absent value renders as a bare `cc:` line that
an agent then has to interpret. Build payloads with nulls freely; they will not be emitted.

**Multi-line strings are emitted as YAML literal blocks** by `LiteralMultilineEmitter`, but
only when that round-trips exactly: no `\r`, no tabs, no trailing whitespace on any line, and
no leading space on the first line. `MessageRenderer.Normalize` produces text that satisfies
this. If you add a new multi-line field, run it through `Normalize` or accept quoted output.

**Prompts and status go to stderr.** Interactive commands build their own console:
`AnsiConsole.Create(new AnsiConsoleSettings { Out = new AnsiConsoleOutput(System.Console.Error) })`.
Never use the ambient `AnsiConsole` for prompts — it writes to stdout and corrupts the payload.

**Two different fetch shapes, for two different reasons.** `format=raw` + MimeKit is used
whenever a body is needed, because encoded-words, quoted-printable, nested multipart and
charset detection are easy to get subtly wrong by hand. `format=full` + the `GmailFields.Structure`
field mask is used when only structure is needed (search hydration, attachment listing) —
Gmail applies the mask server-side, so you get part filenames, sizes and attachment ids without
paying for the encoded body bytes. Raw responses carry no `attachmentId`, so attachment work
must use the structure path.

**`format=raw` exists on `messages.get` only.** `threads.get` takes `full`, `metadata` and
`minimal`, and answers `raw` with a `400` / `invalid_input` naming the rejected enum value — the
two endpoints do not share the format vocabulary even though the URLs look interchangeable.
`thread get` lists the thread at `format=minimal` for the message ids, then fetches each selected
message with `messages/{id}?format=raw`; `GetCommand.ThreadPath` / `MessagePath` hold those two
URLs so a test can assert on them without a network.

## Gmail API notes

Base URL `https://gmail.googleapis.com/gmail/v1/users/me/`. Quota is metered in units against a
per-user budget of **250 units/second** (moving average):

| Call | Units |
| --- | --- |
| `messages.list`, `messages.get`, `attachments.get` | 5 |
| `threads.get`, `drafts.create`, `drafts.update` | 10 |
| `thread get` end to end | 10 + 5 per message returned |
| `labels.list`, `getProfile` | 1 |
| `drafts.send`, `messages.send` | 100 |

`search --limit 50` is ~255 units, which is why hydration runs at `--concurrency 5` by default.
Rate limiting arrives as a `429` **or** a `403` with reason `rateLimitExceeded` /
`userRateLimitExceeded`; `GmailApiClient.ShouldRetry` handles both, with five attempts,
exponential backoff, full jitter and `Retry-After` when present.

`invalid_grant` on refresh covers revocation, a password change, six months of inactivity, and
the seven-day expiry that a **Testing**-mode consent screen imposes. That last one is by far
the most common and the least obvious, so `TokenManager` names it explicitly in the error
detail. Do not generalize that message away.

## Testing

`tests/Gmail.Console.Tests/` covers the four areas with logic worth asserting on —
`ReplyBuilder` (headers and recipient math), `AttachmentWriter` (a security boundary),
`MessageRenderer` (truncation, quote trimming, HTML conversion) and `OutputHelper`
(serialization shape). Everything else is thin API plumbing that a test would only restate.

There are no network tests and no mock HTTP layer. If you add one, keep it out of the default
`dotnet test` run — CI has no credentials.

## Not implemented yet

Ordered roughly by value. The first two are gaps against the current spec; the rest are
deliberate deferrals recorded in spec §02 and §13.

1. **`--body` on `search`.** Spec decision E says metadata-and-snippet is the default with
   `--body markdown` as an opt-in; the opt-in was never wired. Add the option to
   `SearchCommand.Settings` and switch hydration to `format=raw` when it is set — note that
   this changes the cost profile substantially, so keep the default as it is.
2. **`draft update` drops existing attachments.** It rebuilds the MIME from scratch, so any
   attachment not re-specified with `--attach` is lost. Either carry the parts across from the
   parsed `current` message, or refuse the update when the draft has attachments and none were
   passed.
3. **`modify` scope profile.** `ScopeProfiles` has the constant reserved. Adding it means
   `gmail.modify` in the scope bundle plus commands over `messages/{id}/modify`
   (`addLabelIds` / `removeLabelIds`), `messages/{id}/trash` and label create/update/delete.
   Anything destructive needs a `--yes` confirmation, matching `draft delete`.
4. **Headless login.** Descoped as decision I. `OAuthFlow.AuthorizeAsync` already builds and
   prints the consent URL; a `--no-browser` variant means skipping `TryOpenBrowser`, reading
   the pasted redirect URL from stdin, and parsing `code`/`state` out of it instead of from
   `HttpListener`. Roughly 40 lines, same PKCE and state validation.
5. **Incremental sync.** `history.list` with a stored `historyId` (already returned by
   `GmailProfile`), for "what changed since last time" without re-searching.
6. **Pub/Sub `watch`.** Needs a Cloud Pub/Sub topic and an endpoint to receive on, so it is a
   different shape of tool rather than another command.

## Releasing

CI builds and tests every push to `main`. Publishing to NuGet happens only on a published
GitHub Release: the workflow strips the `v` from the tag, packs with that version, and pushes
using the `NUGET_API_KEY` repository secret.

```bash
gh release create v0.1.0 --title v0.1.0 --notes "..."
```

The secret is not set yet, so the `publish` job will fail until someone adds a NuGet PAT.
Bump the patch version on every release; `Version` in the csproj stays `0.0.0-local`.
