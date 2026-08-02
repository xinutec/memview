//! Print the fingerprint of a certificate's public key — the enrolment tool.
//!
//!     cargo run -p console --bin pin -- phone.pem
//!
//! This is the whole of enrolling a device: read the certificate it hands out
//! once, put this line in `CONSOLE_CLIENT_KEYS`, restart. Revoking is deleting
//! the line. There is no CA, no revocation list and no expiry to chase, because
//! there is exactly one relationship being described and both ends of it are
//! known in advance.
//!
//! The fingerprint is of the *key*, not the certificate, so re-issuing the
//! certificate around the same key does not change it — which matters for a key
//! generated inside a phone's secure element, where the key is the thing that
//! cannot be replaced.

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
