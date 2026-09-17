//! Print the fingerprint of a certificate's public key — the enrolment tool.
//!
//!     cargo run -p console --bin pin -- phone.pem
//!
//! Enrolling a device is this line in `CONSOLE_CLIENT_KEYS` and a restart;
//! revoking is deleting it. No CA, no revocation list, no expiry: one
//! relationship, both ends known in advance. The fingerprint is of the KEY, so
//! re-issuing the certificate around it changes nothing — the key in a phone's
//! secure element is the thing that cannot be replaced.

use anyhow::{Context, Result, bail};
use console::tls::pin_of;

fn main() -> Result<()> {
    let Some(path) = std::env::args().nth(1) else {
        bail!("usage: pin <certificate.pem>");
    };
    let pem = std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?;
    let mut found = 0;
    for cert in rustls_pemfile::certs(&mut pem.as_bytes()) {
        let cert = cert.context("reading a certificate out of the PEM")?;
        println!("{}", pin_of(&cert)?);
        found += 1;
    }
    if found == 0 {
        bail!("no certificate in {path}");
    }
    Ok(())
}
