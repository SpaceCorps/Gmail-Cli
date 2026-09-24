use std::io::Write;

use serde_json::Value;

use crate::account;
use crate::auth::{GmailProfile, OAuthFlow, ScopeProfiles, TokenManager};
use crate::cli::{
    AccountAddArgs, AccountCommand, AccountListArgs, AccountReauthArgs, AccountRemoveArgs, AccountTestArgs,
};
use crate::config::{self, AccountConfig};
use crate::error::{Error, ErrorCode, Result};
use crate::output;
use crate::secrets::{self, ClientCredentials, StoredTokens};

pub fn run(cmd: AccountCommand) -> Result<()> {
    match cmd {
        AccountCommand::Add(args) => run_add(args),
        AccountCommand::List(args) => run_list(args),
        AccountCommand::Test(args) => run_test(args),
        AccountCommand::Reauth(args) => run_reauth(args),
        AccountCommand::Remove(args) => run_remove(args),
    }
}

fn run_add(args: AccountAddArgs) -> Result<()> {
    let store = secrets::store()?;
    let client = ClientCredentials::load(store, "default")?;
    let profile_name = args.scope_profile.to_lowercase();
    let scopes = ScopeProfiles::scopes(&profile_name)?;

    let tokens = OAuthFlow::authorize(&client, &scopes, args.port)?;
    let profile = GmailProfile::fetch(tokens.access_token.as_deref().unwrap_or(""))?;

    let name = match args.name {
        Some(ref n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => prompt_for_name(&profile.email_address)?,
    };

    let mut cfg = config::load()?;
    if let Some((existing_name, existing_acct)) = cfg.find(&name) {
        return Err(Error::invalid(format!(
            "An account named '{existing_name}' already exists ({}).",
            existing_acct.email
        ))
        .fix(format!(
            "Pick a different name, or re-authorize the existing one: gmail account reauth {existing_name}"
        )));
    }

    {
        let _lock = config::lock()?;
        tokens.save(store, &name)?;

        cfg = config::load()?;
        cfg.accounts.insert(
            name.clone(),
            AccountConfig {
                email: profile.email_address.clone(),
                scope_profile: profile_name.clone(),
                client_ref: "default".into(),
                added_at: config::now_utc(),
            },
        );
        config::save(&cfg)?;
    }

    output::write(&crate::obj! {
        "status" => "added",
        "name" => name,
        "email" => profile.email_address,
        "scopeProfile" => profile_name,
        "messagesTotal" => profile.messages_total,
        "secretStore" => store.name(),
        "nextStep" => format!("gmail search \"is:unread\" --account {name}"),
    });

    Ok(())
}

fn prompt_for_name(email: &str) -> Result<String> {
    let suggestion = email.split('@').next().unwrap_or("account");
    eprint!("Name for {email} [{suggestion}]: ");
    let _ = std::io::stderr().flush();

    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| Error::other(e.to_string()))?;

    let trimmed = line.trim();
    if trimmed.is_empty() { Ok(suggestion.to_string()) } else { Ok(trimmed.to_string()) }
}

fn run_list(args: AccountListArgs) -> Result<()> {
    let store = secrets::store()?;
    let cfg = config::load()?;
    let mut accounts = Vec::new();

    for (name, acct) in cfg.sorted() {
        let token_status = if !args.check {
            let tokens = StoredTokens::load(store, name)?;
            match tokens {
                Some(ref t) if t.access_token_usable() => "valid",
                Some(_) => "unknown",
                None => "missing_credentials",
            }
        } else {
            match TokenManager::get_access_token(name, &acct.client_ref, store) {
                Ok(token) => match GmailProfile::fetch(&token) {
                    Ok(_) => "valid",
                    Err(e) if e.code == ErrorCode::AuthRequired => "needs_reauth",
                    Err(_) => "unreachable",
                },
                Err(e) if e.code == ErrorCode::AuthRequired => "needs_reauth",
                Err(_) => "unreachable",
            }
        };

        let mut item = serde_json::Map::new();
        item.insert("name".into(), Value::String(name.clone()));
        item.insert("email".into(), Value::String(acct.email.clone()));
        item.insert("scopeProfile".into(), Value::String(acct.scope_profile.clone()));
        item.insert("tokenStatus".into(), Value::String(token_status.into()));
        if !acct.added_at.is_empty() {
            item.insert("addedAt".into(), Value::String(acct.added_at.clone()));
        }
        accounts.push(Value::Object(item));
    }

    output::write(&crate::obj! {
        "accounts" => accounts,
        "count" => cfg.accounts.len(),
        "secretStore" => store.name(),
        "configDir" => config::config_dir().to_string_lossy(),
    });

    Ok(())
}

fn run_test(args: AccountTestArgs) -> Result<()> {
    let store = secrets::store()?;
    let account = account::resolve(Some(&args.name))?;

    let token = TokenManager::get_access_token(&account.name, account.client_ref(), store)?;
    let profile = GmailProfile::fetch(&token)?;

    let mut result = serde_json::Map::new();
    result.insert("name".into(), Value::String(account.name.clone()));
    result.insert("email".into(), Value::String(profile.email_address.clone()));
    result.insert("scopeProfile".into(), Value::String(account.scope_profile().into()));
    result.insert("tokenStatus".into(), Value::String("valid".into()));
    result.insert("messagesTotal".into(), Value::Number(profile.messages_total.into()));
    result.insert("threadsTotal".into(), Value::Number(profile.threads_total.into()));

    if !profile.email_address.eq_ignore_ascii_case(account.email()) {
        result.insert(
            "warning".into(),
            Value::String(format!(
                "Config records this account as {}, but the credentials belong to {}. Run: gmail account reauth {}",
                account.email(),
                profile.email_address,
                account.name
            )),
        );
    }

    output::write(&Value::Object(result));
    Ok(())
}

fn run_reauth(args: AccountReauthArgs) -> Result<()> {
    let store = secrets::store()?;
    let account = account::resolve(Some(&args.name))?;
    let profile_name = args.scope_profile.as_deref().unwrap_or(account.scope_profile()).to_lowercase();
    let scopes = ScopeProfiles::scopes(&profile_name)?;

    let client = ClientCredentials::load(store, account.client_ref())?;
    let tokens = OAuthFlow::authorize(&client, &scopes, args.port)?;
    let profile = GmailProfile::fetch(tokens.access_token.as_deref().unwrap_or(""))?;

    if !profile.email_address.eq_ignore_ascii_case(account.email()) {
        return Err(Error::invalid(format!(
            "You signed in as {}, but '{}' is {}.",
            profile.email_address,
            account.name,
            account.email()
        ))
        .fix(format!(
            "Sign in as {}, or add the other mailbox separately: gmail account add <name>",
            account.email()
        )));
    }

    {
        let _lock = config::lock()?;
        tokens.save(store, &account.name)?;

        let mut cfg = config::load()?;
        if let Some((_, acct)) = cfg.accounts.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(&account.name)) {
            acct.scope_profile = profile_name.clone();
            config::save(&cfg)?;
        }
    }

    output::write(&crate::obj! {
        "status" => "reauthorized",
        "name" => account.name,
        "email" => profile.email_address,
        "scopeProfile" => profile_name,
    });

    Ok(())
}

fn run_remove(args: AccountRemoveArgs) -> Result<()> {
    let store = secrets::store()?;
    let account = account::resolve(Some(&args.name))?;

    if !args.yes {
        eprint!("Remove account {} ({})? [y/N] ", account.name, account.email());
        let _ = std::io::stderr().flush();

        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map_err(|e| Error::other(e.to_string()))?;

        let answer = line.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            return Err(Error::invalid("Cancelled."));
        }
    }

    let tokens = StoredTokens::load(store, &account.name)?;
    let mut revoked = false;

    if !args.local_only
        && let Some(ref t) = tokens
    {
        revoked = TokenManager::revoke(t);
    }

    {
        let _lock = config::lock()?;
        StoredTokens::delete(store, &account.name)?;

        let mut cfg = config::load()?;
        let canonical_key = cfg.accounts.keys().find(|k| k.eq_ignore_ascii_case(&account.name)).cloned();
        if let Some(key) = canonical_key {
            cfg.accounts.shift_remove(&key);
            config::save(&cfg)?;
        }
    }

    let mut result = serde_json::Map::new();
    result.insert("status".into(), Value::String("removed".into()));
    result.insert("name".into(), Value::String(account.name.clone()));
    result.insert("email".into(), Value::String(account.email().into()));
    result.insert("revokedAtGoogle".into(), Value::Bool(if args.local_only { false } else { revoked }));

    if !args.local_only && !revoked {
        result.insert(
            "warning".into(),
            Value::String(
                "Local credentials were deleted but the grant could not be revoked at Google. Remove it manually at https://myaccount.google.com/permissions".into(),
            ),
        );
    }

    output::write(&Value::Object(result));
    Ok(())
}
