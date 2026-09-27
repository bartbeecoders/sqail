//! Service API tokens live in the OS credential store (Secret Service on
//! Linux, Credential Manager on Windows). When none is available they fall
//! back to `tokens.toml` in the config dir, readable only by the user.

use std::collections::BTreeMap;
use std::path::PathBuf;

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

fn file_path() -> PathBuf {
    crate::settings::config_dir().join("tokens.toml")
}

fn read_file() -> BTreeMap<String, String> {
    std::fs::read_to_string(file_path())
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_file(map: &BTreeMap<String, String>) -> anyhow::Result<()> {
    let path = file_path();
    std::fs::create_dir_all(crate::settings::config_dir())?;
    std::fs::write(&path, toml::to_string(map)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
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
    let keyring = if keyring_disabled() {
        Err(keyring::Error::NoEntry)
    } else {
        keyring::Entry::new(SERVICE, url).and_then(|e| e.set_password(token))
    };
    match keyring {
        Ok(()) => {
            // Do not leave an older copy in the fallback file.
            let mut map = read_file();
            if map.remove(url).is_some() {
                write_file(&map)?;
            }
            Ok(Store::Keyring)
        }
        Err(e) => {
            tracing::warn!(error = %e, "no OS credential store; saving token to tokens.toml");
            let mut map = read_file();
            map.insert(url.to_string(), token.to_string());
            write_file(&map)?;
            Ok(Store::File)
        }
    }
}

pub fn load_token(url: &str) -> Option<(String, Store)> {
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
    read_file().remove(url).map(|t| (t, Store::File))
}

pub fn delete_token(url: &str) {
    let _ = keyring::Entry::new(SERVICE, url).and_then(|e| e.delete_credential());
    let _ = keyring::Entry::new(SERVICE_SQAIL2, url).and_then(|e| e.delete_credential());
    let mut map = read_file();
    if map.remove(url).is_some() {
        let _ = write_file(&map);
    }
}
