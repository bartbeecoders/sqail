//! rustls configs: pin a certificate by fingerprint, or verify normally.

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

/// Appears in the rustls error so callers can recognise a pin mismatch.
pub(crate) const MISMATCH_MARKER: &str = "sqail: certificate fingerprint mismatch";

fn provider() -> Arc<CryptoProvider> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    CryptoProvider::get_default()
        .cloned()
        .expect("provider installed")
}

/// `AB:CD:…` SHA-256 of a DER certificate (same format the service logs).
pub fn fingerprint_of(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn normalize(fp: &str) -> Result<String, String> {
    let hex: String = fp.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 64 {
        return Err(format!(
            "fingerprint must be 32 bytes of hex, got {} digits",
            hex.len()
        ));
    }
    Ok(hex.to_uppercase())
}

/// Pinned or system-verified, optionally presenting a client certificate.
pub(crate) fn config(
    trust: &crate::Trust,
    identity: Option<&crate::Identity>,
) -> Result<ClientConfig, String> {
    let provider = provider();
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;
    let builder = match trust {
        crate::Trust::Pinned(fp) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinVerifier {
                v: Verifier::Pinned(normalize(fp)?),
                provider,
            })),
        crate::Trust::System => {
            let mut roots = rustls::RootCertStore::empty();
            let native = rustls_native_certs::load_native_certs();
            roots.add_parsable_certificates(native.certs);
            builder.with_root_certificates(roots)
        }
    };
    let mut cfg = match identity {
        None => builder.with_no_client_auth(),
        Some(id) => {
            use rustls::pki_types::pem::PemObject;
            let certs = CertificateDer::pem_slice_iter(&id.cert_pem)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("client certificate: {e}"))?;
            let key = rustls::pki_types::PrivateKeyDer::from_pem_slice(&id.key_pem)
                .map_err(|e| format!("client key: {e}"))?;
            builder
                .with_client_auth_cert(certs, key)
                .map_err(|e| format!("client certificate: {e}"))?
        }
    };
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(cfg)
}

pub(crate) fn recording() -> (ClientConfig, Arc<Mutex<Option<String>>>) {
    let seen = Arc::new(Mutex::new(None));
    (custom(Verifier::Record(seen.clone())), seen)
}

fn custom(v: Verifier) -> ClientConfig {
    let provider = provider();
    let mut cfg = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("default versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinVerifier { v, provider }))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    cfg
}

#[derive(Debug)]
enum Verifier {
    Pinned(String),
    Record(Arc<Mutex<Option<String>>>),
}

#[derive(Debug)]
struct PinVerifier {
    v: Verifier,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let got = fingerprint_of(end_entity.as_ref());
        match &self.v {
            Verifier::Pinned(want) if got.replace(':', "") == *want => {
                Ok(ServerCertVerified::assertion())
            }
            Verifier::Pinned(_) => Err(rustls::Error::General(MISMATCH_MARKER.into())),
            Verifier::Record(seen) => {
                *seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(got);
                Ok(ServerCertVerified::assertion())
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_normalize() {
        let fp = "ab:cd:".repeat(16);
        assert_eq!(normalize(&fp).unwrap(), "ABCD".repeat(16));
        assert!(normalize("AB:CD").is_err());
    }
}
