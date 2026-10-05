//! rustls client configs for connecting to database servers.
//!
//! `verify-full` checks the chain and the host name. `verify-ca` checks the
//! chain only (libpq's mode of the same name): rustls has no switch for that,
//! so it uses a verifier that stops after the trust anchor. `require` and
//! `prefer` encrypt without authenticating the server. A CA certificate
//! replaces the OS trust store. A client certificate is presented whenever
//! TLS is negotiated.

use std::sync::{Arc, OnceLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use sqail_proto::PgSslMode;

/// Largest accepted PEM (certificate bundle or private key).
pub const PEM_LIMIT: usize = 64 * 1024;

/// Material for one PostgreSQL connection. Empty strings count as absent.
pub struct PgTls<'a> {
    pub mode: PgSslMode,
    pub root_pem: Option<&'a str>,
    pub client_cert_pem: Option<&'a str>,
    pub client_key_pem: Option<&'a str>,
}

/// The rustls config `ssl_mode` and the certificates ask for.
pub fn for_postgres(m: PgTls<'_>) -> Result<Arc<ClientConfig>, String> {
    crate::tls::install_crypto_provider();
    let root = blank(m.root_pem);
    let cert = blank(m.client_cert_pem);
    let key = blank(m.client_key_pem);
    if cert.is_some() != key.is_some() {
        return Err("client certificate and private key are both required".into());
    }
    if root.is_none() && cert.is_none() {
        return Ok(match m.mode {
            PgSslMode::VerifyFull => verifying(),
            PgSslMode::VerifyCa => verifying_ca(),
            PgSslMode::Disable | PgSslMode::Prefer | PgSslMode::Require => unverified(),
        });
    }
    if let Some(key) = key
        && key.contains("ENCRYPTED")
    {
        return Err(
            "client key is encrypted; provide an unencrypted PEM private key (PKCS#8, PKCS#1 or SEC1)"
                .into(),
        );
    }
    for (pem, what) in [
        (root, "CA certificate"),
        (cert, "client certificate"),
        (key, "client key"),
    ] {
        if let Some(pem) = pem
            && pem.len() > PEM_LIMIT
        {
            return Err(format!("{what} is larger than 64 KiB"));
        }
    }

    let verify = matches!(m.mode, PgSslMode::VerifyCa | PgSslMode::VerifyFull);
    let provider = CryptoProvider::get_default()
        .cloned()
        .expect("crypto provider installed");
    let builder = ClientConfig::builder();
    let builder = if verify {
        let roots = match root {
            Some(pem) => roots_from_pem(pem)?,
            None => system_roots(),
        };
        if m.mode == PgSslMode::VerifyFull {
            builder.with_root_certificates(roots)
        } else {
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(ChainOnly {
                    roots: Arc::new(roots),
                    provider: provider.clone(),
                }))
        }
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
    };
    finish(builder, cert, key).map(Arc::new)
}

fn blank(pem: Option<&str>) -> Option<&str> {
    pem.map(str::trim).filter(|s| !s.is_empty())
}

fn finish(
    builder: rustls::ConfigBuilder<ClientConfig, rustls::client::WantsClientCert>,
    cert_pem: Option<&str>,
    key_pem: Option<&str>,
) -> Result<ClientConfig, String> {
    match (cert_pem, key_pem) {
        (None, None) => Ok(builder.with_no_client_auth()),
        (Some(cert), Some(key)) => {
            let certs = CertificateDer::pem_slice_iter(cert.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| pem_err("client certificate", e))?;
            if certs.is_empty() {
                return Err("client certificate: no PEM certificate found".into());
            }
            let key = PrivateKeyDer::from_pem_slice(key.as_bytes())
                .map_err(|e| pem_err("client key", e))?;
            builder
                .with_client_auth_cert(certs, key)
                .map_err(|e| pem_err("client certificate", e))
        }
        _ => Err("client certificate and private key are both required".into()),
    }
}

/// Parser messages sometimes quote the PEM. Keep the reason, drop the body.
fn pem_err(what: &str, err: impl std::fmt::Display) -> String {
    let detail = err.to_string();
    let detail = detail.lines().next().unwrap_or("invalid PEM");
    if detail.len() > 160 || detail.contains("BEGIN ") || detail.contains("-----") {
        format!("{what} is not valid PEM")
    } else {
        format!("{what}: {detail}")
    }
}

fn roots_from_pem(pem: &str) -> Result<RootCertStore, String> {
    let mut roots = RootCertStore::empty();
    let mut any = false;
    for cert in CertificateDer::pem_slice_iter(pem.as_bytes()) {
        let cert = cert.map_err(|e| pem_err("CA certificate", e))?;
        roots.add(cert).map_err(|e| pem_err("CA certificate", e))?;
        any = true;
    }
    if !any {
        return Err("CA certificate: no PEM certificate found".into());
    }
    Ok(roots)
}

fn system_roots() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    for err in &native.errors {
        tracing::warn!(error = %err, "could not load a native root certificate");
    }
    let (added, _) = roots.add_parsable_certificates(native.certs);
    tracing::debug!(added, "loaded native root certificates");
    roots
}

/// Verifies against the OS trust store (plus corporate CAs installed there).
pub fn verifying() -> Arc<ClientConfig> {
    static CFG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CFG.get_or_init(|| {
        crate::tls::install_crypto_provider();
        Arc::new(
            ClientConfig::builder()
                .with_root_certificates(system_roots())
                .with_no_client_auth(),
        )
    })
    .clone()
}

/// Chain checked against the OS trust store; host name not checked.
fn verifying_ca() -> Arc<ClientConfig> {
    static CFG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CFG.get_or_init(|| {
        crate::tls::install_crypto_provider();
        let provider = CryptoProvider::get_default()
            .cloned()
            .expect("crypto provider installed");
        Arc::new(
            ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(ChainOnly {
                    roots: Arc::new(system_roots()),
                    provider,
                }))
                .with_no_client_auth(),
        )
    })
    .clone()
}

/// Encrypts but does not authenticate the server (libpq `sslmode=require`).
/// Signatures are still checked, so the handshake itself is sound.
pub fn unverified() -> Arc<ClientConfig> {
    static CFG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CFG.get_or_init(|| {
        crate::tls::install_crypto_provider();
        let provider = CryptoProvider::get_default()
            .cloned()
            .expect("crypto provider installed");
        Arc::new(
            ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
                .with_no_client_auth(),
        )
    })
    .clone()
}

#[derive(Debug)]
struct NoVerify(Arc<CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
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
            &self.0.signature_verification_algorithms,
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
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Trust-anchor check without the host-name check (`sslmode=verify-ca`).
#[derive(Debug)]
struct ChainOnly {
    roots: Arc<RootCertStore>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for ChainOnly {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let cert = rustls::server::ParsedCertificate::try_from(end_entity)?;
        rustls::client::verify_server_cert_signed_by_trust_anchor(
            &cert,
            &self.roots,
            intermediates,
            now,
            self.provider.signature_verification_algorithms.all,
        )?;
        Ok(ServerCertVerified::assertion())
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
    use std::sync::Arc;

    use rustls::pki_types::{PrivateKeyDer, ServerName};
    use rustls::{ClientConfig, RootCertStore, ServerConfig, ServerConnection};

    use super::*;

    struct Material {
        ca_pem: String,
        other_ca_pem: String,
        server_cert: rustls::pki_types::CertificateDer<'static>,
        server_key_pem: String,
        client_cert_pem: String,
        client_key_pem: String,
    }

    fn material() -> Material {
        use rcgen::{
            BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
        };
        let ca_key = KeyPair::generate().unwrap();
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();
        let issuer = Issuer::from_params(&ca_params, &ca_key);

        let server_key = KeyPair::generate().unwrap();
        let mut server_params = CertificateParams::new(vec!["db.example".into()]).unwrap();
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let server_cert = server_params.signed_by(&server_key, &issuer).unwrap();

        let client_key = KeyPair::generate().unwrap();
        let mut client_params = CertificateParams::new(vec!["sqail-client".into()]).unwrap();
        client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let client_cert = client_params.signed_by(&client_key, &issuer).unwrap();

        let other_key = KeyPair::generate().unwrap();
        let mut other_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        other_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let other = other_params.self_signed(&other_key).unwrap();

        Material {
            ca_pem: ca_cert.pem(),
            other_ca_pem: other.pem(),
            server_cert: server_cert.der().clone(),
            server_key_pem: server_key.serialize_pem(),
            client_cert_pem: client_cert.pem(),
            client_key_pem: client_key.serialize_pem(),
        }
    }

    fn server(m: &Material, require_client: bool) -> Arc<ServerConfig> {
        crate::tls::install_crypto_provider();
        let key = PrivateKeyDer::from_pem_slice(m.server_key_pem.as_bytes()).unwrap();
        let chain = vec![m.server_cert.clone()];
        let cfg = if require_client {
            let mut roots = RootCertStore::empty();
            for cert in CertificateDer::pem_slice_iter(m.ca_pem.as_bytes()) {
                roots.add(cert.unwrap()).unwrap();
            }
            let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
                .build()
                .unwrap();
            ServerConfig::builder()
                .with_client_cert_verifier(verifier)
                .with_single_cert(chain, key)
                .unwrap()
        } else {
            ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(chain, key)
                .unwrap()
        };
        Arc::new(cfg)
    }

    fn handshake(
        client_cfg: Arc<ClientConfig>,
        server_cfg: Arc<ServerConfig>,
        name: &str,
    ) -> Result<ServerConnection, String> {
        let mut client = rustls::ClientConnection::new(
            client_cfg,
            ServerName::try_from(name.to_owned()).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let mut server = ServerConnection::new(server_cfg).map_err(|e| e.to_string())?;
        for _ in 0..16 {
            if !client.is_handshaking() && !server.is_handshaking() {
                return Ok(server);
            }
            let mut moved = false;
            if client.wants_write() {
                let mut buf = Vec::new();
                client.write_tls(&mut buf).map_err(|e| e.to_string())?;
                server
                    .read_tls(&mut buf.as_slice())
                    .map_err(|e| e.to_string())?;
                server.process_new_packets().map_err(|e| e.to_string())?;
                moved = true;
            }
            if server.wants_write() {
                let mut buf = Vec::new();
                server.write_tls(&mut buf).map_err(|e| e.to_string())?;
                client
                    .read_tls(&mut buf.as_slice())
                    .map_err(|e| e.to_string())?;
                client.process_new_packets().map_err(|e| e.to_string())?;
                moved = true;
            }
            if !moved {
                return Err("handshake stalled".into());
            }
        }
        Err("handshake did not finish".into())
    }

    fn cfg<'a>(
        mode: PgSslMode,
        m: &'a Material,
        root: Option<&'a str>,
        client: bool,
    ) -> Arc<ClientConfig> {
        for_postgres(PgTls {
            mode,
            root_pem: root,
            client_cert_pem: client.then_some(m.client_cert_pem.as_str()),
            client_key_pem: client.then_some(m.client_key_pem.as_str()),
        })
        .unwrap()
    }

    #[test]
    fn verify_full_trusts_the_given_ca_and_checks_the_name() {
        let m = material();
        let ok = handshake(
            cfg(PgSslMode::VerifyFull, &m, Some(&m.ca_pem), false),
            server(&m, false),
            "db.example",
        );
        assert!(ok.is_ok(), "{ok:?}");
        let wrong_name = handshake(
            cfg(PgSslMode::VerifyFull, &m, Some(&m.ca_pem), false),
            server(&m, false),
            "other.example",
        );
        assert!(
            wrong_name.is_err(),
            "verify-full must reject a different host name"
        );
        let wrong_ca = handshake(
            cfg(PgSslMode::VerifyFull, &m, Some(&m.other_ca_pem), false),
            server(&m, false),
            "db.example",
        );
        assert!(wrong_ca.is_err(), "verify-full must reject a different CA");
    }

    #[test]
    fn verify_ca_skips_the_host_name_and_still_checks_the_chain() {
        let m = material();
        let ok = handshake(
            cfg(PgSslMode::VerifyCa, &m, Some(&m.ca_pem), false),
            server(&m, false),
            "other.example",
        );
        assert!(ok.is_ok(), "{ok:?}");
        let wrong_ca = handshake(
            cfg(PgSslMode::VerifyCa, &m, Some(&m.other_ca_pem), false),
            server(&m, false),
            "db.example",
        );
        assert!(wrong_ca.is_err(), "verify-ca must reject a different CA");
    }

    #[test]
    fn require_presents_the_client_certificate_without_checking_the_server() {
        let m = material();
        let done = handshake(
            cfg(PgSslMode::Require, &m, None, true),
            server(&m, true),
            "db.example",
        )
        .expect("handshake");
        assert!(
            done.peer_certificates().is_some_and(|c| !c.is_empty()),
            "server must see the client certificate"
        );
        let missing = handshake(
            cfg(PgSslMode::Require, &m, None, false),
            server(&m, true),
            "db.example",
        );
        assert!(missing.is_err(), "server requires a client certificate");
    }

    #[test]
    fn bad_pem_is_rejected_without_echoing_the_body() {
        let err = for_postgres(PgTls {
            mode: PgSslMode::Require,
            root_pem: None,
            client_cert_pem: Some("-----BEGIN CERTIFICATE-----\nSENTINEL_CERT_zz\n-----END CERTIFICATE-----\n"),
            client_key_pem: Some("-----BEGIN PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nSENTINEL_KEY_zz\n-----END PRIVATE KEY-----\n"),
        })
        .unwrap_err();
        assert!(!err.contains("SENTINEL"), "{err}");
        assert!(err.contains("encrypted"), "{err}");

        let err = for_postgres(PgTls {
            mode: PgSslMode::VerifyFull,
            root_pem: Some(
                "-----BEGIN CERTIFICATE-----\nSENTINEL_CA_zz\n-----END CERTIFICATE-----\n",
            ),
            client_cert_pem: None,
            client_key_pem: None,
        })
        .unwrap_err();
        assert!(!err.contains("SENTINEL"), "{err}");
        assert!(err.contains("CA certificate"), "{err}");
    }
}
