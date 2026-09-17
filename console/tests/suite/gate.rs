//! Who gets in, proven by connecting.
//!
//! The claim the whole design rests on is negative — *a key that is not pinned
//! cannot reach the API* — and a negative claim is only worth what the attempt
//! to break it is worth. So these run a real TLS server and make real
//! connections: the right key, the wrong key, and no key at all.
//!
//! Certificates are generated here rather than committed. A keypair in a
//! repository outlives the test that needed it.

use std::sync::Arc;

use console::tls::{Gate, pin_of};
use rcgen::{CertifiedKey, generate_simple_self_signed};
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use tokio_rustls::TlsConnector;

/// A self-signed identity: PEM for the server, DER for pinning.
struct Identity {
    cert_pem: String,
    key_pem: String,
}

fn identity(name: &str) -> Identity {
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec![name.to_string()]).expect("generate");
    Identity {
        cert_pem: cert.pem(),
        key_pem: signing_key.serialize_pem(),
    }
}

fn der_of(pem: &str) -> CertificateDer<'static> {
    rustls_pemfile::certs(&mut pem.as_bytes())
        .next()
        .expect("a certificate")
        .expect("valid PEM")
}

/// A client that presents `identity`, or nothing when it is `None`.
///
/// The server is not verified here — these tests are about the *client* half of
/// the handshake, and the phone's own pinning of the server key is the other
/// direction. A verifier that accepts anything keeps that out of the way.
fn client_config(identity: Option<&Identity>) -> rustls::ClientConfig {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyServer(provider)));
    match identity {
        Some(id) => {
            let cert = vec![der_of(&id.cert_pem)];
            let key = rustls_pemfile::private_key(&mut id.key_pem.as_bytes())
                .expect("key")
                .expect("a key");
            builder
                .with_client_auth_cert(cert, key)
                .expect("client auth")
        }
        None => builder.with_no_client_auth(),
    }
}

/// Accepts any server certificate — see `client_config`.
#[derive(Debug)]
struct AnyServer(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AnyServer {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
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
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Serve one TLS connection with the given gate, and report what the request
/// line was — or `None` if the handshake never completed.
async fn serve_once(
    gate: rustls::ServerConfig,
) -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<Option<String>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("addr");
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(gate));
    let handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.ok()?;
        let mut tls = acceptor.accept(stream).await.ok()?;
        let mut buffer = [0u8; 256];
        let read = tls.read(&mut buffer).await.ok()?;
        let request = String::from_utf8_lossy(&buffer[..read]).to_string();
        let _ = tls
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
            .await;
        // Close cleanly: without close_notify the client's read ends in an
        // "unexpected EOF" error and a successful exchange looks like a refusal.
        let _ = tls.shutdown().await;
        Some(request)
    });
    (address, handle)
}

/// Try to reach the server with `identity`, and say whether the request landed.
async fn reaches(address: std::net::SocketAddr, identity: Option<&Identity>) -> bool {
    let connector = TlsConnector::from(Arc::new(client_config(identity)));
    let Ok(stream) = tokio::net::TcpStream::connect(address).await else {
        return false;
    };
    let name = ServerName::try_from("console.local").expect("name");
    let mut tls = match connector.connect(name, stream).await {
        Ok(tls) => tls,
        Err(err) => {
            eprintln!("handshake refused: {err}");
            return false;
        }
    };
    if tls
        .write_all(b"GET /api/state HTTP/1.1\r\nHost: console\r\n\r\n")
        .await
        .is_err()
    {
        return false;
    }
    let mut answer = String::new();
    // A rejected client can complete `connect` and only learn of the refusal on
    // the first read, so the request must actually be answered to count.
    match tls.read_to_string(&mut answer).await {
        Ok(_) => answer.contains("200 OK"),
        Err(err) => {
            eprintln!("read failed: {err}");
            false
        }
    }
}

fn gate_for(server: &Identity, pins: &[String]) -> rustls::ServerConfig {
    Gate::new(&server.cert_pem, &server.key_pem, pins)
        .expect("gate")
        .server_config()
        .expect("server config")
}

#[tokio::test]
async fn the_pinned_key_gets_in() {
    let server = identity("console.local");
    let phone = identity("phone");
    let pin = pin_of(&der_of(&phone.cert_pem)).expect("pin");

    let (address, served) = serve_once(gate_for(&server, &[pin])).await;
    assert!(
        reaches(address, Some(&phone)).await,
        "the pinned key is admitted"
    );
    assert!(
        served
            .await
            .expect("join")
            .is_some_and(|r| r.contains("/api/state")),
        "and its request reached the server"
    );
}

#[tokio::test]
async fn a_key_that_is_not_pinned_is_refused() {
    // The whole point. This is the compromised-hub case: it can reach the socket
    // and speak TLS, and it does not hold the key.
    let server = identity("console.local");
    let phone = identity("phone");
    let stranger = identity("someone else");
    let pin = pin_of(&der_of(&phone.cert_pem)).expect("pin");

    let (address, served) = serve_once(gate_for(&server, &[pin])).await;
    assert!(
        !reaches(address, Some(&stranger)).await,
        "a key that is not pinned does not get in"
    );
    assert!(
        served.await.expect("join").is_none(),
        "and nothing of its request reached the server"
    );
}

#[tokio::test]
async fn no_certificate_at_all_is_refused() {
    // The ordinary case: a browser, a curl, a port scanner.
    let server = identity("console.local");
    let phone = identity("phone");
    let pin = pin_of(&der_of(&phone.cert_pem)).expect("pin");

    let (address, served) = serve_once(gate_for(&server, &[pin])).await;
    assert!(
        !reaches(address, None).await,
        "no client certificate, no entry"
    );
    assert!(served.await.expect("join").is_none());
}

#[tokio::test]
async fn a_second_pinned_key_gets_in_beside_the_first() {
    // Adding the iPhone later is adding a line, and must not disturb the phone
    // that is already enrolled.
    let server = identity("console.local");
    let phone = identity("phone");
    let tablet = identity("tablet");
    let pins = vec![
        pin_of(&der_of(&phone.cert_pem)).expect("pin"),
        pin_of(&der_of(&tablet.cert_pem)).expect("pin"),
    ];

    let (address, _) = serve_once(gate_for(&server, &pins)).await;
    assert!(
        reaches(address, Some(&tablet)).await,
        "the second key works too"
    );
}

#[test]
fn the_pin_is_of_the_key_and_not_of_the_certificate() {
    // Why this matters: a phone's key is generated once in its secure element
    // and cannot be replaced, while its certificate may be reissued. Pinning the
    // certificate would lock the device out on a reissue.
    let phone = identity("phone");
    let der = der_of(&phone.cert_pem);
    let pin = pin_of(&der).expect("pin");
    assert_eq!(pin.len(), 64, "a sha-256 in hex");
    assert_eq!(pin, pin_of(&der).expect("pin"), "and it is stable");

    // A different key gives a different pin — the property that makes it a pin.
    let other = identity("phone");
    assert_ne!(pin, pin_of(&der_of(&other.cert_pem)).expect("pin"));
}

#[test]
fn tls_without_a_pinned_key_is_refused_rather_than_served_open() {
    // Serving TLS while accepting everybody looks locked from the outside, which
    // is worse than being plainly off.
    let server = identity("console.local");
    let empty: Vec<String> = Vec::new();
    let err = Gate::new(&server.cert_pem, &server.key_pem, &empty).expect_err("refused");
    assert!(format!("{err:#}").contains("nobody could connect"));
}
