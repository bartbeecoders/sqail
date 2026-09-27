//! AES-256-GCM encryption for `.sqail` file payloads.
//!
//! Two modes:
//! - **Machine key** (default): a 32-byte key stored at `<app_data>/sqail.key`.
//!   Portable only between sessions on the same machine / user.
//! - **Passphrase**: Argon2id-derived key. Portable across machines given the passphrase.
//!
//! An envelope is a small JSON object carrying the algorithm id, nonce, the
//! salt (passphrase mode only) and the ciphertext — all base64.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};

const KEY_FILE: &str = "sqail.key";
const ALG_MACHINE: &str = "AES-256-GCM/machine";
const ALG_PASSPHRASE: &str = "AES-256-GCM/argon2id";

static MACHINE_KEY_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Initialise the module with the directory that holds the machine key.
/// Safe to call multiple times — only the first call takes effect.
pub fn init(app_data_dir: &Path) {
    let _ = MACHINE_KEY_PATH.set(app_data_dir.join(KEY_FILE));
}

fn machine_key_path() -> Result<&'static PathBuf, String> {
    MACHINE_KEY_PATH
        .get()
        .ok_or_else(|| "crypto module not initialised".to_string())
}

fn load_or_create_machine_key() -> Result<[u8; 32], String> {
    let path = machine_key_path()?;
    if let Ok(bytes) = std::fs::read(path) {
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
    }
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create key dir: {e}"))?;
    }
    std::fs::write(path, key).map_err(|e| format!("write key: {e}"))?;
    Ok(key)
}

fn derive_passphrase_key(passphrase: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    // Argon2id at interactive parameters — good balance for per-file save.
    let params = Params::new(19_456, 2, 1, Some(32))
        .map_err(|e| format!("argon2 params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|e| format!("argon2 derive: {e}"))?;
    Ok(out)
}

/// On-disk envelope. `salt` is only populated in passphrase mode.
#[derive(Debug, Serialize, Deserialize)]
pub struct CryptoEnvelope {
    #[serde(rename = "$enc")]
    pub enc: bool,
    pub alg: String,
    pub nonce: String,
    pub ct: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salt: Option<String>,
}

pub fn encrypt(plaintext: &str, passphrase: Option<&str>) -> Result<CryptoEnvelope, String> {
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);

    let (key_bytes, alg, salt_b64) = match passphrase {
        Some(pw) if !pw.is_empty() => {
            let mut salt = [0u8; 16];
            rand::thread_rng().fill_bytes(&mut salt);
            let k = derive_passphrase_key(pw, &salt)?;
            (k, ALG_PASSPHRASE, Some(B64.encode(salt)))
        }
        _ => (load_or_create_machine_key()?, ALG_MACHINE, None),
    };

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
        .map_err(|e| format!("encrypt: {e}"))?;

    Ok(CryptoEnvelope {
        enc: true,
        alg: alg.to_string(),
        nonce: B64.encode(nonce_bytes),
        ct: B64.encode(ct),
        salt: salt_b64,
    })
}

pub fn decrypt(env: &CryptoEnvelope, passphrase: Option<&str>) -> Result<String, String> {
    let nonce = B64.decode(&env.nonce).map_err(|e| format!("nonce b64: {e}"))?;
    let ct = B64.decode(&env.ct).map_err(|e| format!("ct b64: {e}"))?;

    let key_bytes: [u8; 32] = match env.alg.as_str() {
        ALG_MACHINE => load_or_create_machine_key()?,
        ALG_PASSPHRASE => {
            let pw = passphrase.ok_or_else(|| "passphrase required".to_string())?;
            let salt = env
                .salt
                .as_ref()
                .ok_or_else(|| "salt missing for passphrase envelope".to_string())?;
            let salt = B64.decode(salt).map_err(|e| format!("salt b64: {e}"))?;
            derive_passphrase_key(pw, &salt)?
        }
        other => return Err(format!("unknown alg: {other}")),
    };

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
    let pt = cipher
        .decrypt(Nonce::from_slice(&nonce), ct.as_ref())
        .map_err(|_| "decrypt failed (wrong passphrase or corrupted file)".to_string())?;
    String::from_utf8(pt).map_err(|e| format!("utf8: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_tmp_key<F: FnOnce()>(f: F) {
        let dir = std::env::temp_dir().join(format!("sqail-crypto-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // OnceLock can only be set once per process; if a prior test set it,
        // reuse that path — the key file is still under a temp dir.
        let _ = MACHINE_KEY_PATH.set(dir.join(KEY_FILE));
        f();
    }

    #[test]
    fn machine_round_trip() {
        with_tmp_key(|| {
            let env = encrypt("hello, secret", None).unwrap();
            assert_eq!(env.alg, ALG_MACHINE);
            assert!(env.salt.is_none());
            let pt = decrypt(&env, None).unwrap();
            assert_eq!(pt, "hello, secret");
        });
    }

    #[test]
    fn passphrase_round_trip() {
        with_tmp_key(|| {
            let env = encrypt("top secret", Some("correct horse")).unwrap();
            assert_eq!(env.alg, ALG_PASSPHRASE);
            assert!(env.salt.is_some());
            let pt = decrypt(&env, Some("correct horse")).unwrap();
            assert_eq!(pt, "top secret");
            assert!(decrypt(&env, Some("wrong")).is_err());
        });
    }
}
