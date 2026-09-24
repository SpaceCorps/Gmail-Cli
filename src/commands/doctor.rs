//! Doctor command: diagnose configuration, credentials, keystore, and connectivity.

use std::time::Duration;

use serde_json::{Map, Value};

use crate::auth::{GmailProfile, TokenManager};
use crate::cli::DoctorArgs;
use crate::config;
use crate::error::Result;
use crate::secrets::{ClientCredentials, Store};

pub fn run(_args: DoctorArgs, _verbose: bool, _timeout: u64) -> Result<Value> {
    let mut checks = Vec::new();
    let mut problems = 0;

    let store_res = Store::open();
    let store_opt = match store_res {
        Ok(store) => {
            checks.push(check("secret_store", "ok", &format!("Using the {} backend.", store.name()), None));
            Some(store)
        }
        Err(e) => {
            problems += 1;
            checks.push(check("secret_store", "fail", &e.message, e.remediation.as_deref()));
            None
        }
    };

    let config_path = config::config_path();
    let config_exists = config_path.exists();
    if config_exists {
        checks.push(check("config_file", "ok", &config_path.display().to_string(), None));
    } else {
        checks.push(check(
            "config_file",
            "warn",
            &format!("No config yet at {}.", config_path.display()),
            Some("gmail setup"),
        ));
    }

    let cfg = if config_exists { config::load().unwrap_or_default() } else { config::Config::default() };

    if let Some(store) = store_opt {
        let has_client = ClientCredentials::exists(store, "default").unwrap_or(false);
        if has_client {
            checks.push(check("oauth_client", "ok", "Client id and secret are stored.", None));
        } else {
            problems += 1;
            checks.push(check("oauth_client", "fail", "No OAuth client credentials configured.", Some("gmail setup")));
        }

        if cfg.accounts.is_empty() {
            checks.push(check("accounts", "warn", "No accounts configured.", Some("gmail account add <name>")));
        } else {
            let mut sorted_accounts: Vec<_> = cfg.accounts.iter().collect();
            sorted_accounts.sort_by_key(|(name, _)| name.to_lowercase());

            for (name, account) in sorted_accounts {
                match TokenManager::get_access_token(name, &account.client_ref, store) {
                    Ok(token) => match GmailProfile::fetch(&token) {
                        Ok(profile) => {
                            checks.push(check(
                                &format!("account:{name}"),
                                "ok",
                                &format!(
                                    "{} — {} messages, scope profile '{}'.",
                                    profile.email_address, profile.messages_total, account.scope_profile
                                ),
                                None,
                            ));
                        }
                        Err(e) => {
                            problems += 1;
                            checks.push(check(
                                &format!("account:{name}"),
                                "fail",
                                &e.message,
                                e.remediation.as_deref(),
                            ));
                        }
                    },
                    Err(e) => {
                        problems += 1;
                        checks.push(check(&format!("account:{name}"), "fail", &e.message, e.remediation.as_deref()));
                    }
                }
            }
        }
    }

    if let Some(skew) = clock_skew() {
        let seconds = skew.abs();
        if seconds < 60.0 {
            checks.push(check(
                "clock",
                "ok",
                &format!("Local clock differs from Google's by {seconds:.0} seconds."),
                None,
            ));
        } else {
            checks.push(check(
                "clock",
                "warn",
                &format!("Local clock differs from Google's by {seconds:.0} seconds."),
                Some("Sync the system clock — OAuth rejects tokens with large skew."),
            ));
        }
    }

    checks.push(check(
        "consent_screen",
        "unverifiable",
        "Whether your app is published to 'In production' cannot be read from the API. \
         If accounts stop working with invalid_grant after about a week, this is the cause: \
         a publishing status of Testing expires refresh tokens after 7 days.",
        Some("https://console.cloud.google.com/auth/audience -> Publish app"),
    ));

    let mut result = Map::new();
    result.insert("configDir".into(), Value::String(config::config_dir().display().to_string()));
    result.insert("secretStore".into(), store_opt.map(|s| Value::String(s.name().to_string())).unwrap_or(Value::Null));
    result.insert("accountCount".into(), Value::Number(cfg.accounts.len().into()));
    result.insert("problems".into(), Value::Number(problems.into()));
    result.insert("checks".into(), Value::Array(checks));

    Ok(Value::Object(result))
}

fn check(name: &str, status: &str, detail: &str, remediation: Option<&str>) -> Value {
    let mut m = Map::new();
    m.insert("check".into(), Value::String(name.to_string()));
    m.insert("status".into(), Value::String(status.to_string()));
    m.insert("detail".into(), Value::String(detail.to_string()));
    if let Some(rem) = remediation {
        m.insert("remediation".into(), Value::String(rem.to_string()));
    }
    Value::Object(m)
}

fn clock_skew() -> Option<f64> {
    let url = std::env::var("GMAIL_CLOCK_URL").unwrap_or_else(|_| "https://oauth2.googleapis.com/".to_string());
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .http_status_as_error(false)
        .build()
        .into();

    let resp = agent.get(&url).call().ok()?;
    let date_hdr = resp.headers().get("Date")?.to_str().ok()?;
    let server_time = parse_http_date(date_hdr)?;

    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64();

    Some(now - server_time)
}

fn parse_http_date(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 5 {
        return None;
    }
    let day: i64 = parts.get(1)?.parse().ok()?;
    let month_str = parts.get(2)?;
    let year: i64 = parts.get(3)?.parse().ok()?;
    let time_parts: Vec<&str> = parts.get(4)?.split(':').collect();
    if time_parts.len() < 3 {
        return None;
    }
    let hour: i64 = time_parts.first()?.parse().ok()?;
    let min: i64 = time_parts.get(1)?.parse().ok()?;
    let sec: i64 = time_parts.get(2)?.parse().ok()?;

    let month = match *month_str {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };

    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hour * 3600 + min * 60 + sec;
    Some(secs as f64)
}
