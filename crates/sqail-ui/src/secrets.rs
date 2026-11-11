//! Service API tokens live in the OS credential store (Secret Service on
//! Linux, Credential Manager on Windows). When none is available they fall
//! back to `tokens.toml` in the config dir, readable only by the user.
//!
//! The OS store can block (a locked keyring shows an unlock prompt, D-Bus can
//! be slow), so the app calls the `*_in` functions from its worker, passing
//! the config dir (which is per thread, see `settings::override_config_dir`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const SERVICE: &str = "sqail";
/// Credential store service name up to 1.0, when the app was called sqail2.
const SERVICE_SQAIL2: &str = "sqail2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    Keyring,
    File,
}

impl Store {
    pub fn describe(self) -> &'static str {
        match self {
            Store::Keyring => "OS credential store",
            Store::File => "tokens.toml (no OS credential store available)",
        }
    }
}

fn file_path(dir: &Path) -> PathBuf {
    dir.join("tokens.toml")
}

/// The fallback tokens. A damaged file is kept as `tokens.toml.bad` (its
/// owner-only mode goes with it) rather than replaced by the next save.
fn read_file(dir: &Path) -> BTreeMap<String, String> {
    crate::local::load_file(&file_path(dir), |t| {
        toml::from_str(t).map_err(|e| e.message().to_string())
    })
    .unwrap_or_default()
    .unwrap_or_default()
}

/// Written atomically and created owner-only (0600 on Unix) from the start,
/// so the tokens are never readable by other users, not even briefly.
fn write_file(dir: &Path, map: &BTreeMap<String, String>) -> anyhow::Result<()> {
    crate::local::write_private(&file_path(dir), toml::to_string(map)?.as_bytes())?;
    Ok(())
}

static FILE_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Never touch the OS credential store in this process (tests).
pub fn force_file_store() {
    FILE_ONLY.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// `SQAIL_TOKEN_STORE=file` skips the OS credential store (headless
/// machines, CI).
fn keyring_disabled() -> bool {
    FILE_ONLY.load(std::sync::atomic::Ordering::Relaxed)
        || std::env::var("SQAIL_TOKEN_STORE").is_ok_and(|v| v == "file")
}

pub fn save_token(url: &str, token: &str) -> anyhow::Result<Store> {
    save_token_in(&crate::settings::config_dir(), url, token)
}

/// [`save_token`] with the fallback file in `dir`.
pub fn save_token_in(dir: &Path, url: &str, token: &str) -> anyhow::Result<Store> {
    let keyring = if keyring_disabled() {
        Err(keyring::Error::NoEntry)
    } else {
        keyring::Entry::new(SERVICE, url).and_then(|e| e.set_password(token))
    };
    match keyring {
        Ok(()) => {
            // Do not leave an older copy in the fallback file.
            let mut map = read_file(dir);
            if map.remove(url).is_some() {
                write_file(dir, &map)?;
            }
            Ok(Store::Keyring)
        }
        Err(e) => {
            tracing::warn!(error = %e, "no OS credential store; saving token to tokens.toml");
            let mut map = read_file(dir);
            map.insert(url.to_string(), token.to_string());
            write_file(dir, &map)?;
            Ok(Store::File)
        }
    }
}

pub fn load_token(url: &str) -> Option<(String, Store)> {
    load_token_in(&crate::settings::config_dir(), url)
}

/// [`load_token`] with the fallback file in `dir`.
pub fn load_token_in(dir: &Path, url: &str) -> Option<(String, Store)> {
    if !keyring_disabled()
        && let Ok(t) = keyring::Entry::new(SERVICE, url).and_then(|e| e.get_password())
    {
        return Some((t, Store::Keyring));
    }
    if !keyring_disabled()
        && let Ok(old) = keyring::Entry::new(SERVICE_SQAIL2, url)
        && let Ok(t) = old.get_password()
    {
        // Move it under the new name; keep the old entry if that fails.
        if keyring::Entry::new(SERVICE, url)
            .and_then(|e| e.set_password(&t))
            .is_ok()
        {
            let _ = old.delete_credential();
        }
        return Some((t, Store::Keyring));
    }
    read_file(dir).remove(url).map(|t| (t, Store::File))
}

pub fn delete_token(url: &str) {
    delete_token_in(&crate::settings::config_dir(), url)
}

/// [`delete_token`] with the fallback file in `dir`.
pub fn delete_token_in(dir: &Path, url: &str) {
    if !keyring_disabled() {
        let _ = keyring::Entry::new(SERVICE, url).and_then(|e| e.delete_credential());
        let _ = keyring::Entry::new(SERVICE_SQAIL2, url).and_then(|e| e.delete_credential());
    }
    let mut map = read_file(dir);
    if map.remove(url).is_some() {
        let _ = write_file(dir, &map);
    }
}
