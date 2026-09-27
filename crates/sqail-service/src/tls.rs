//! Server TLS: load the configured certificate, or create a self-signed
//! development certificate on first start.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};

use crate::config::Config;

pub struct ServerTls {
    pub config: Arc<ServerConfig>,
    /// SHA-256 of the leaf certificate, `AB:CD:…` — clients pin this.
    pub fingerprint: String,
    pub self_signed: bool,
}

/// Install aws-lc-rs as the process-wide rustls provider. Idempotent.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

pub fn load(cfg: &Config) -> Result<ServerTls> {
    install_crypto_provider();
    let (cert_path, key_path, self_signed) = match (&cfg.tls.cert, &cfg.tls.key) {
        (Some(c), Some(k)) => (c.clone(), k.clone(), false),
        (None, None) => {
            let dir = cfg.data_dir.join("tls");
            let (c, k) = (dir.join("dev-cert.pem"), dir.join("dev-key.pem"));
            if !c.exists() || !k.exists() {
                generate_dev_cert(&c, &k)?;
            }
            (c, k, true)
        }
        _ => bail!("tls.cert and tls.key must be set together"),
    };

    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(&cert_path)
        .with_context(|| format!("reading {}", cert_path.display()))?
        .collect::<Result<_, _>>()
        .with_context(|| format!("parsing {}", cert_path.display()))?;
    let leaf = certs
        .first()
        .context("certificate file contains no certificate")?;
    let fingerprint = fingerprint(leaf);
    let key = PrivateKeyDer::from_pem_file(&key_path)
        .with_context(|| format!("reading {}", key_path.display()))?;

    let versions: &[&rustls::SupportedProtocolVersion] = if cfg.tls.allow_tls12 {
        &[&rustls::version::TLS13, &rustls::version::TLS12]
    } else {
        &[&rustls::version::TLS13]
    };
    let builder = ServerConfig::builder_with_protocol_versions(versions);
    let builder = match &cfg.tls.client_ca {
        None => builder.with_no_client_auth(),
        Some(ca_path) => {
            let mut roots = rustls::RootCertStore::empty();
            for ca in CertificateDer::pem_file_iter(ca_path)
                .with_context(|| format!("reading {}", ca_path.display()))?
            {
                roots
                    .add(ca.with_context(|| format!("parsing {}", ca_path.display()))?)
                    .context("adding client CA")?;
            }
            let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
                .build()
                .context("building client certificate verifier")?;
            tracing::info!(ca = %ca_path.display(), "mutual TLS: client certificates required");
            builder.with_client_cert_verifier(verifier)
        }
    };
    let mut config = builder
        .with_single_cert(certs, key)
        .context("building TLS config")?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(ServerTls {
        config: Arc::new(config),
        fingerprint,
        self_signed,
    })
}

/// Check that a PEM certificate chain and private key belong together and
/// can serve TLS. Returns the leaf certificate's fingerprint.
pub fn check_pair(cert_pem: &[u8], key_pem: &[u8]) -> Result<String> {
    install_crypto_provider();
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<_, _>>()
        .context("the certificate is not valid PEM")?;
    let leaf = certs.first().context("no certificate found in the PEM")?;
    let fingerprint = fingerprint(leaf);
    let key = PrivateKeyDer::from_pem_slice(key_pem).context("the private key is not valid PEM")?;
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("the certificate and key do not match")?;
    Ok(fingerprint)
}

pub fn fingerprint(cert: &CertificateDer<'_>) -> String {
    Sha256::digest(cert.as_ref())
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn generate_dev_cert(cert_path: &Path, key_path: &Path) -> Result<()> {
    let mut names = vec!["localhost".to_string(), "127.0.0.1".into(), "::1".into()];
    if let Ok(host) = std::env::var("HOSTNAME").or_else(|_| std::env::var("COMPUTERNAME")) {
        names.push(host.to_lowercase());
    }
    let ck = rcgen::generate_simple_self_signed(names).context("generating dev certificate")?;
    if let Some(dir) = cert_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(cert_path, ck.cert.pem())?;
    crate::crypto::write_private_file(key_path, ck.signing_key.serialize_pem().as_bytes())?;
    tracing::warn!(cert = %cert_path.display(), "generated a self-signed development certificate");
    Ok(())
}
