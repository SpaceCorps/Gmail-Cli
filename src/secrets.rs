//! Where credentials and tokens live: the OS keystore.
//!
//! DPAPI on Windows, Keychain on macOS, libsecret on Linux. When none is available the tool
//! refuses to start rather than silently writing a file - `GMAIL_ALLOW_PLAINTEXT_STORE=1` is
//! the explicit opt-out. Service names, entry keys and file formats match the .NET CLI this
//! replaced, so keys stored by it are found here.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::config;
use crate::error::{Error, Result};
use crate::output;

const SERVICE: &str = "gmail-cli";

#[allow(dead_code)]
pub fn default_client_key() -> &'static str {
    "client:default"
}

pub fn client_key(client_ref: &str) -> String {
    if client_ref == "default" { "client:default".to_string() } else { format!("client:{}", client_ref.to_lowercase()) }
}

pub fn account_key(name: &str) -> String {
    format!("account:{}", name.to_lowercase())
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClientCredentials {
    pub client_id: String,
    pub client_secret: String,
}

impl ClientCredentials {
    pub fn load(store: Store, client_ref: &str) -> Result<ClientCredentials> {
        let key = client_key(client_ref);
        let json = store.get(&key)?.ok_or_else(|| {
            Error::auth("No OAuth client credentials are configured.")
                .detail("The Google Cloud client id and secret have not been set up on this machine.")
                .fix("gmail setup")
        })?;

        serde_json::from_str(&json).map_err(|e| {
            Error::other("Stored client credentials are corrupt.").detail(e.to_string()).fix("gmail setup")
        })
    }

    pub fn save(&self, store: Store, client_ref: &str) -> Result<()> {
        let key = client_key(client_ref);
        let json = serde_json::to_string(self).map_err(|e| Error::other(e.to_string()))?;
        store.set(&key, &json)
    }

    pub fn exists(store: Store, client_ref: &str) -> Result<bool> {
        let key = client_key(client_ref);
        Ok(store.get(&key)?.is_some())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoredTokens {
    pub refresh_token: String,
    pub access_token: Option<String>,
    pub expires_at: Option<String>,
    pub scope: Option<String>,
}

impl StoredTokens {
    pub fn access_token_usable(&self) -> bool {
        let Some(ref token) = self.access_token else {
            return false;
        };
        if token.trim().is_empty() {
            return false;
        }

        let Some(ref exp_str) = self.expires_at else {
            return false;
        };

        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);

        // Parse either ISO-8601 string or epoch timestamp
        let exp_secs = if let Ok(n) = exp_str.parse::<i64>() { n } else { parse_iso8601(exp_str).unwrap_or(0) };

        // 60-second buffer
        exp_secs > now + 60
    }

    pub fn load(store: Store, account_name: &str) -> Result<Option<StoredTokens>> {
        let key = account_key(account_name);
        let Some(json) = store.get(&key)? else {
            return Ok(None);
        };
        let tokens: StoredTokens = serde_json::from_str(&json).map_err(|e| {
            Error::other(format!("Stored tokens for '{account_name}' are corrupt.")).detail(e.to_string())
        })?;
        Ok(Some(tokens))
    }

    pub fn save(&self, store: Store, account_name: &str) -> Result<()> {
        let key = account_key(account_name);
        let json = serde_json::to_string(self).map_err(|e| Error::other(e.to_string()))?;
        store.set(&key, &json)
    }

    pub fn delete(store: Store, account_name: &str) -> Result<()> {
        let key = account_key(account_name);
        store.delete(&key)
    }
}

fn parse_iso8601(s: &str) -> Option<i64> {
    // Basic parser for YYYY-MM-DDTHH:MM:SSZ
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: i64 = s.get(5..7)?.parse().ok()?;
    let day: i64 = s.get(8..10)?.parse().ok()?;
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let min: i64 = s.get(14..16)?.parse().ok()?;
    let sec: i64 = s.get(17..19)?.parse().ok()?;

    // Howard Hinnant's days_from_civil
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + min * 60 + sec)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Dpapi,
    Keychain,
    LibSecret,
    Plaintext,
}

pub fn store() -> Result<Store> {
    static STORE: OnceLock<std::result::Result<Store, (String, String, String)>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            build().map_err(|e| (e.message, e.detail.unwrap_or_default(), e.remediation.unwrap_or_default()))
        })
        .clone()
        .map_err(|(m, d, r)| {
            let mut e = Error::other(m);
            if !d.is_empty() {
                e = e.detail(d);
            }
            if !r.is_empty() {
                e = e.fix(r);
            }
            e
        })
}

fn build() -> Result<Store> {
    if let Some(forced) = std::env::var("GMAIL_SECRET_STORE").ok().filter(|s| !s.trim().is_empty()) {
        return match forced.to_lowercase().as_str() {
            "dpapi" if cfg!(windows) => Ok(Store::Dpapi),
            "keychain" => Ok(Store::Keychain),
            "libsecret" => Ok(Store::LibSecret),
            "plaintext" => Ok(Store::Plaintext),
            _ => Err(Error::invalid(format!(
                "GMAIL_SECRET_STORE='{forced}' is not a backend available on this platform."
            ))
            .fix("Unset GMAIL_SECRET_STORE, or set it to one of: dpapi, keychain, libsecret, plaintext.")),
        };
    }

    if cfg!(windows) {
        return Ok(Store::Dpapi);
    }

    if cfg!(target_os = "macos") {
        if Path::new("/usr/bin/security").exists() || which("security").is_some() {
            return Ok(Store::Keychain);
        }
        return fallback("The macOS 'security' command was not found.");
    }

    if which("secret-tool").is_some() {
        return Ok(Store::LibSecret);
    }

    fallback(
        "libsecret is not installed, so there is no OS keystore to hold your refresh tokens. \
         Install it with: sudo apt install libsecret-tools  (or the equivalent for your distro).",
    )
}

fn fallback(reason: &str) -> Result<Store> {
    if std::env::var("GMAIL_ALLOW_PLAINTEXT_STORE").as_deref() == Ok("1") {
        return Ok(Store::Plaintext);
    }
    Err(Error::other("No secure credential store is available on this machine.")
        .detail(reason)
        .fix("Install a keystore, or set GMAIL_ALLOW_PLAINTEXT_STORE=1 to store credentials in a 0600 file instead."))
}

fn which(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(cmd)).find(|p| p.is_file())
}

static WARNED_PLAINTEXT: AtomicBool = AtomicBool::new(false);

fn warn_plaintext() {
    if !WARNED_PLAINTEXT.swap(true, Ordering::Relaxed) {
        output::status(format!(
            "warning: credentials are stored unencrypted in {} (GMAIL_ALLOW_PLAINTEXT_STORE=1).",
            plaintext_path().display()
        ));
    }
}

fn plaintext_path() -> PathBuf {
    config::config_dir().join("secrets.json")
}

impl Store {
    pub fn open() -> Result<Store> {
        store()
    }

    pub fn name(self) -> &'static str {
        match self {
            Store::Dpapi => "dpapi",
            Store::Keychain => "keychain",
            Store::LibSecret => "libsecret",
            Store::Plaintext => "plaintext",
        }
    }

    pub fn get(self, key: &str) -> Result<Option<String>> {
        match self {
            Store::Keychain => {
                let (ok, out, _) = run("security", &["find-generic-password", "-s", SERVICE, "-a", key, "-w"], None)?;
                Ok(ok.then(|| out.trim_end_matches('\n').to_string()).filter(|s| !s.is_empty()))
            }
            Store::LibSecret => {
                let (ok, out, _) = run("secret-tool", &["lookup", "service", SERVICE, "account", key], None)?;
                Ok(ok.then(|| out.trim_end_matches('\n').to_string()).filter(|s| !s.is_empty()))
            }
            Store::Plaintext => {
                warn_plaintext();
                Ok(file_read(self)?.remove(key))
            }
            Store::Dpapi => Ok(file_read(self)?.remove(key)),
        }
    }

    pub fn set(self, key: &str, value: &str) -> Result<()> {
        match self {
            Store::Keychain => {
                let (ok, _, err) =
                    run("security", &["add-generic-password", "-U", "-s", SERVICE, "-a", key, "-w", value], None)?;
                if !ok {
                    return Err(Error::other("Could not write to the macOS Keychain.").detail(err.trim()));
                }
                Ok(())
            }
            Store::LibSecret => {
                let label = format!("{SERVICE}: {key}");
                let (ok, _, err) = run(
                    "secret-tool",
                    &["store", &format!("--label={label}"), "service", SERVICE, "account", key],
                    Some(value),
                )?;
                if !ok {
                    return Err(Error::other("Could not write to the system keyring.").detail(err.trim()));
                }
                Ok(())
            }
            Store::Plaintext => {
                warn_plaintext();
                let mut map = file_read(self)?;
                map.insert(key.to_string(), value.to_string());
                file_write(self, &map)
            }
            Store::Dpapi => {
                let mut map = file_read(self)?;
                map.insert(key.to_string(), value.to_string());
                file_write(self, &map)
            }
        }
    }

    pub fn delete(self, key: &str) -> Result<()> {
        match self {
            Store::Keychain => {
                let _ = run("security", &["delete-generic-password", "-s", SERVICE, "-a", key], None);
                Ok(())
            }
            Store::LibSecret => {
                let _ = run("secret-tool", &["clear", "service", SERVICE, "account", key], None);
                Ok(())
            }
            Store::Plaintext | Store::Dpapi => {
                let mut map = file_read(self)?;
                if map.remove(key).is_some() {
                    file_write(self, &map)?;
                }
                Ok(())
            }
        }
    }
}

fn run(cmd: &str, args: &[&str], stdin: Option<&str>) -> Result<(bool, String, String)> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::other(format!("Could not start '{cmd}': {e}")))?;

    if let Some(s) = stdin
        && let Some(mut in_pipe) = child.stdin.take()
    {
        let _ = in_pipe.write_all(s.as_bytes());
    }

    let out = child.wait_with_output().map_err(|e| Error::other(format!("'{cmd}' failed: {e}")))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

fn file_path(store: Store) -> PathBuf {
    match store {
        Store::Dpapi => config::config_dir().join("secrets.dpapi"),
        _ => plaintext_path(),
    }
}

fn file_read(store: Store) -> Result<BTreeMap<String, String>> {
    let path = file_path(store);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e.into()),
    };
    if bytes.is_empty() {
        return Ok(BTreeMap::new());
    }

    let clear = match store {
        Store::Dpapi => dpapi_unprotect(&bytes)?,
        _ => bytes,
    };

    serde_json::from_slice(&clear).map_err(|e| {
        Error::other(format!("{} is corrupt.", path.display()))
            .detail(e.to_string())
            .fix("Delete the file and re-run gmail setup and gmail account add.")
    })
}

fn file_write(store: Store, map: &BTreeMap<String, String>) -> Result<()> {
    config::ensure_dir()?;
    let path = file_path(store);
    let json = serde_json::to_vec_pretty(map).map_err(|e| Error::other(e.to_string()))?;

    let blob = match store {
        Store::Dpapi => dpapi_protect(&json)?,
        _ => json,
    };

    config::atomic_write(&path, &blob)
}

#[cfg(windows)]
fn dpapi_protect(clear: &[u8]) -> Result<Vec<u8>> {
    use std::ptr;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData};

    let mut in_blob = CRYPT_INTEGER_BLOB { cbData: clear.len() as u32, pbData: clear.as_ptr() as *mut u8 };
    let mut out_blob = CRYPT_INTEGER_BLOB { cbData: 0, pbData: ptr::null_mut() };

    let ok = unsafe {
        CryptProtectData(
            &mut in_blob,
            ptr::null(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        )
    };

    if ok == 0 {
        return Err(Error::other("DPAPI CryptProtectData failed."));
    }

    let slice = unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize) };
    let vec = slice.to_vec();
    unsafe {
        LocalFree(out_blob.pbData as _);
    }
    Ok(vec)
}

#[cfg(not(windows))]
fn dpapi_protect(_clear: &[u8]) -> Result<Vec<u8>> {
    Err(Error::other("DPAPI is available only on Windows."))
}

#[cfg(windows)]
fn dpapi_unprotect(cipher: &[u8]) -> Result<Vec<u8>> {
    use std::ptr;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData,
    };

    let mut in_blob = CRYPT_INTEGER_BLOB { cbData: cipher.len() as u32, pbData: cipher.as_ptr() as *mut u8 };
    let mut out_blob = CRYPT_INTEGER_BLOB { cbData: 0, pbData: ptr::null_mut() };

    let ok = unsafe {
        CryptUnprotectData(
            &mut in_blob,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out_blob,
        )
    };

    if ok == 0 {
        return Err(Error::other("DPAPI CryptUnprotectData failed."));
    }

    let slice = unsafe { std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize) };
    let vec = slice.to_vec();
    unsafe {
        LocalFree(out_blob.pbData as _);
    }
    Ok(vec)
}

#[cfg(not(windows))]
fn dpapi_unprotect(_cipher: &[u8]) -> Result<Vec<u8>> {
    Err(Error::other("DPAPI is available only on Windows."))
}
