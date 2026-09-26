//! At-rest encryption of connection secrets (AES-256-GCM) and token hashing.

use std::io::Write;
use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use sha2::{Digest, Sha256};

const PREFIX: &str = "v1:";
const NONCE_LEN: usize = 12;

/// The key that encrypts every stored secret. Loaded from `SQAIL_MASTER_KEY`
/// (base64, 32 bytes) or `<data_dir>/master.key`, created on first start with
/// owner-only permissions.
pub struct MasterKey(Aes256Gcm);

impl MasterKey {
    pub fn load_or_create(data_dir: &Path) -> Result<Self> {
        let bytes = if let Ok(b64) = std::env::var("SQAIL_MASTER_KEY") {
            B64.decode(b64.trim())
                .context("SQAIL_MASTER_KEY is not valid base64")?
        } else {
            let path = data_dir.join("master.key");
            if path.exists() {
                B64.decode(std::fs::read_to_string(&path)?.trim())
                    .with_context(|| format!("{} is corrupt", path.display()))?
            } else {
                let key: [u8; 32] = rand::random();
                write_private_file(&path, B64.encode(key).as_bytes())?;
                tracing::info!(path = %path.display(), "created master key");
                key.to_vec()
            }
        };
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 32 {
            bail!("master key must be 32 bytes, got {}", bytes.len());
        }
        Ok(Self(
            Aes256Gcm::new_from_slice(bytes).map_err(|e| anyhow!("{e}"))?,
        ))
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let nonce_bytes: [u8; NONCE_LEN] = rand::random();
        let nonce = Nonce::from(nonce_bytes);
        let ct = self
            .0
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| anyhow!("encryption failed"))?;
        let mut out = nonce_bytes.to_vec();
        out.extend_from_slice(&ct);
        Ok(format!("{PREFIX}{}", B64.encode(out)))
    }

    pub fn decrypt(&self, stored: &str) -> Result<String> {
        let raw = stored
            .strip_prefix(PREFIX)
            .context("unknown secret format")?;
        let raw = B64.decode(raw).context("secret is not valid base64")?;
        if raw.len() <= NONCE_LEN {
            bail!("secret too short");
        }
        let (nonce, ct) = raw.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().expect("split at NONCE_LEN");
        let pt = self.0.decrypt(&Nonce::from(nonce), ct).map_err(|_| {
            anyhow!("secret cannot be decrypted (wrong master key or tampered data)")
        })?;
        String::from_utf8(pt).context("secret is not UTF-8")
    }
}

/// SHA-256 hex digest. Tokens carry 256 bits of entropy, so a fast hash is
/// the right tool (no password stretching needed).
pub fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

/// Create a new file readable only by the current user.
pub fn write_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    // On Windows the per-user app-data directory already restricts access.
    let mut f = opts
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    f.write_all(contents)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> MasterKey {
        MasterKey::from_bytes(&[7u8; 32]).unwrap()
    }

    #[test]
    fn round_trip() {
        let k = key();
        let ct = k.encrypt("s3cret ✓").unwrap();
        assert!(ct.starts_with("v1:"));
        assert_ne!(k.encrypt("s3cret ✓").unwrap(), ct, "nonce must differ");
        assert_eq!(k.decrypt(&ct).unwrap(), "s3cret ✓");
    }

    #[test]
    fn tampering_is_detected() {
        let k = key();
        let ct = k.encrypt("hello").unwrap();
        let mut raw = B64.decode(&ct[3..]).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        assert!(k.decrypt(&format!("v1:{}", B64.encode(raw))).is_err());
    }

    #[test]
    fn wrong_key_fails() {
        let ct = key().encrypt("hello").unwrap();
        let other = MasterKey::from_bytes(&[8u8; 32]).unwrap();
        assert!(other.decrypt(&ct).is_err());
    }
}
