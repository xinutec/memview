//! The gate: who may talk to this console at all.
//!
//! **The pinned thing is a public key, not a certificate and not a name.** There
//! is no CA and no PKI here on purpose — one client, one server, both known to
//! each other in advance, and a certificate authority would add a third party
//! able to mint a fourth. What the runner holds is a set of SHA-256 hashes of
//! `SubjectPublicKeyInfo`: the key survives the certificate being reissued, which
//! matters because the client's key is generated once inside a phone's secure
//! element and can never be replaced, while its certificate is just a wrapper
//! that may want a longer expiry one day.
//!
//! **Why this and not a firewall rule.** The WireGuard hub decrypts and
//! re-encrypts every peer-to-peer packet, so it can forge a source address. An
//! allow-list of addresses is a doorman; this is the lock. A compromised hub can
//! still deny service — unavoidable for a router — but it cannot produce a
//! `CertificateVerify` signature over a key it does not hold, so it cannot
//! impersonate the phone and it cannot read the traffic.
//!
//! **What this deliberately does NOT check.** Not the certificate's expiry, not
//! its subject, not its chain, not its issuer. Those are questions about who
//! vouched for a name, and nobody vouches for anything here: the key is either
//! the pinned one or it is not. Checking expiry would add a way for the console
//! to lock the phone out on a date nobody chose.

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use sha2::{Digest, Sha256};

/// A key's fingerprint, as it is written in configuration: 64 hex characters.
pub type Pin = String;

/// The fingerprint of the public key inside a DER certificate.
///
/// This is the whole enrolment mechanism: run it over the certificate the phone
/// hands out once, put the answer in the config, and the console will talk to
/// that key and nothing else.
pub fn pin_of(der: &[u8]) -> Result<Pin> {
    let (_, parsed) =
        x509_parser::parse_x509_certificate(der).context("that is not a DER certificate")?;
    let spki = parsed.tbs_certificate.subject_pki.raw;
    Ok(hex(&Sha256::digest(spki)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Accepts exactly the keys it was given, and refuses everything else.
#[derive(Debug)]
struct Pinned {
    allowed: BTreeSet<Pin>,
    /// The signature algorithms the provider actually supports. Verification of
    /// the handshake signature itself is left to rustls — the pin decides *whose*
    /// key it must be, not whether the maths is right.
    supported: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl ClientCertVerifier for Pinned {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        // No CA, so no hints. A client that needs to be told which authority to
        // present for is not a client this console knows.
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        let pin = pin_of(end_entity).map_err(|err| {
            rustls::Error::General(format!("client certificate is unreadable: {err}"))
        })?;
        if self.allowed.contains(&pin) {
            return Ok(ClientCertVerified::assertion());
        }
        // Logged as well as returned, because the returned error goes into a TLS
        // alert and the client sees "handshake failure" and nothing else. The
        // rejected fingerprint is the one thing that makes enrolment possible:
        // plug the device in, watch the log, paste the line into the config.
        tracing::warn!(key = %pin, "refused a client key that is not pinned");
        Err(rustls::Error::General(format!(
            "client key {pin} is not pinned"
        )))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.supported)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported.supported_schemes()
    }
}

/// The server's own identity and the keys it will accept.
#[derive(Debug)]
pub struct Gate {
    pub cert: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
    pub allowed: BTreeSet<Pin>,
}

impl Gate {
    /// Read the server's certificate and key from PEM, and the pins from config.
    pub fn new(cert_pem: &str, key_pem: &str, pins: &[String]) -> Result<Self> {
        let cert: Vec<CertificateDer<'static>> =
            rustls_pemfile::certs(&mut cert_pem.as_bytes()).collect::<Result<_, _>>()?;
        if cert.is_empty() {
            bail!("no certificate in the PEM");
        }
        let key = rustls_pemfile::private_key(&mut key_pem.as_bytes())?
            .context("no private key in the PEM")?;
        // Checked here so the failure is legible. rustls refuses a version-1
        // certificate with `UnsupportedCertVersion` and no hint about which
        // certificate or why — and the openssl shipped with macOS (LibreSSL)
        // produces exactly that from a plain `req -x509`, silently, because a
        // certificate with no extensions has no need of version 3.
        for one in &cert {
            let (_, parsed) = x509_parser::parse_x509_certificate(one)
                .context("the server certificate is not readable DER")?;
            if parsed.tbs_certificate.version.0 != 2 {
                bail!(
                    "the server certificate is version {}, and TLS needs version 3. \
                     macOS's openssl makes a v1 certificate unless it is given an \
                     extension — add `-addext \"subjectAltName=DNS:<name>\"`.",
                    parsed.tbs_certificate.version.0 + 1
                );
            }
        }
        let allowed: BTreeSet<Pin> = pins
            .iter()
            .map(|pin| pin.trim().to_lowercase())
            .filter(|pin| !pin.is_empty())
            .collect();
        if allowed.is_empty() {
            // Serving TLS while accepting every client would be worse than
            // serving none: it looks locked from the outside.
            bail!("TLS is configured but no client key is pinned — nobody could connect");
        }
        Ok(Self { cert, key, allowed })
    }

    /// The rustls configuration this gate describes.
    pub fn server_config(self) -> Result<rustls::ServerConfig> {
        let provider = rustls::crypto::ring::default_provider();
        let supported = provider.signature_verification_algorithms;
        let verifier = Arc::new(Pinned {
            allowed: self.allowed,
            supported,
        });
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()?
            .with_client_cert_verifier(verifier)
            .with_single_cert(self.cert, self.key)
            .context("the certificate and key do not go together")?;
        Ok(config)
    }
}
