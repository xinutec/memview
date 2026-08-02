//! Check a phone's attestation chain, and print the pin if it holds up.
//!
//!     cargo run -p console --bin attest -- <chain.pem> <challenge-hex> [status.json]
//!
//! Run by `scripts/enrol.sh`, which is where the challenge comes from and where
//! the status list is fetched. It exists separately because the answer it gives is
//! the one thing in this system that cannot be undone by editing a config: a key
//! pinned on a bad chain is a key that looks exactly like a good one forever
//! after.
//!
//! Exits non-zero if anything failed, and prints the pin only when nothing did —
//! so a pipeline that reads its output cannot accidentally enrol a phone the check
//! rejected.

use anyhow::{Context, Result, bail};
use console::attest::examine;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(chain), Some(challenge)) = (args.next(), args.next()) else {
        bail!("usage: attest <chain.pem> <challenge-hex> [status.json]");
    };
    let status = args.next();

    let pem = std::fs::read_to_string(&chain).with_context(|| format!("reading {chain}"))?;
    let challenge = unhex(&challenge).context("the challenge must be hex")?;
    let status = match &status {
        Some(path) => {
            Some(std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?)
        }
        None => None,
    };

    let examination = examine(&pem, &challenge, status.as_deref())?;
    for finding in &examination.findings {
        println!(
            "{} {}",
            if finding.ok { "  ok" } else { "FAIL" },
            finding.what
        );
        println!("       {}", finding.detail);
    }
    println!();
    if !examination.ok() {
        bail!("this chain does not support enrolling the key — do not pin it");
    }
    // The pin last and alone, so it is what a script reads off the end.
    println!("{}", examination.pin);
    Ok(())
}

fn unhex(text: &str) -> Result<Vec<u8>> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        bail!("odd number of hex digits");
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair)?, 16).context("not a hex digit pair")
        })
        .collect()
}
