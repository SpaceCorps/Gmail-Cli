//! The account is always explicit - no stored default, no environment variable, no implicit
//! fallback when only one account is configured.
//!
//! The failure this prevents is an agent working from a summarized transcript drafting from
//! the wrong mailbox: silent locally, and only discovered by the recipient.

use crate::config::{self, AccountConfig, Config};
use crate::error::{Error, ErrorCode, Result};
use crate::secrets;

pub struct Resolved {
    pub name: String,
    pub config: AccountConfig,
}

impl Resolved {
    pub fn email(&self) -> &str {
        &self.config.email
    }

    pub fn scope_profile(&self) -> &str {
        &self.config.scope_profile
    }

    pub fn client_ref(&self) -> &str {
        &self.config.client_ref
    }
}

pub fn resolve(requested: Option<&str>) -> Result<Resolved> {
    let config = config::load()?;

    let Some(requested) = requested.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(Error::new(ErrorCode::NoAccount, "No account specified. Pass --account <name>.")
            .detail(describe(&config))
            .fix("gmail account list"));
    };

    let Some((name, account)) = config.find(requested) else {
        return Err(Error::new(ErrorCode::NoAccount, format!("No account named '{requested}'."))
            .detail(describe(&config))
            .fix("gmail account list"));
    };

    let store = secrets::store()?;
    let tokens = secrets::StoredTokens::load(store, name)?;

    // A config entry with no tokens is an account whose credentials were lost or cleared.
    if tokens.is_none() {
        return Err(Error::new(ErrorCode::AuthRequired, format!("Account '{name}' has no stored credentials."))
            .detail("The account is listed in config but its tokens are missing from the keystore.")
            .fix(format!("gmail account reauth {name}")));
    }

    Ok(Resolved { name: name.clone(), config: account.clone() })
}

pub fn describe(config: &Config) -> String {
    if config.accounts.is_empty() {
        return "No accounts are configured yet. Run 'gmail setup' and then 'gmail account add <name>'.".into();
    }
    let listed: Vec<String> = config
        .sorted()
        .into_iter()
        .map(|(k, v)| if v.email.trim().is_empty() { k.clone() } else { format!("{k} ({})", v.email) })
        .collect();
    format!("Configured accounts: {}", listed.join(", "))
}
