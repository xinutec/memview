//! What a phone claims, and what enrolment does about it.
//!
//! The records here are built by hand rather than captured from a device. Two
//! reasons, and the second is the one that matters: a real chain identifies a real
//! phone and this repository is public — and a captured chain can only ever show
//! the case that happened to be true when it was taken. Hand-built records are the
//! only way to test the answers that matter, which are the refusals: an imported
//! key, a key with no authentication, a TEE key claiming to be StrongBox, a chain
//! answering last month's challenge.
//!
//! The certificate carrying each record is self-signed by `rcgen`, so every test
//! also asserts the thing that makes the check worth running at all: a chain that
//! does not reach a Google root is refused no matter how good the record inside it
//! looks.

use console::attest::{SecurityLevel, examine, parse_record};
use rcgen::{CertificateParams, CustomExtension, KeyPair};

/// Google's KeyDescription extension.
const ATTESTATION_OID: &[u64] = &[1, 3, 6, 1, 4, 1, 11129, 2, 1, 17];

const STRONGBOX: u64 = 2;
const TEE: u64 = 1;
const GENERATED: u64 = 0;
const IMPORTED: u64 = 2;

/// The shape of a record, so each test can say only what it is about.
struct Claim {
    attestation_level: u64,
    keymint_level: u64,
    challenge: Vec<u8>,
    no_auth_required: bool,
    user_auth_type: Option<u64>,
    auth_timeout: Option<u64>,
    origin: Option<u64>,
}

impl Claim {
    /// What a correctly enrolled Pixel says.
    fn good(challenge: &[u8]) -> Self {
        Self {
            attestation_level: STRONGBOX,
            keymint_level: STRONGBOX,
            challenge: challenge.to_vec(),
            no_auth_required: false,
            user_auth_type: Some(3),
            auth_timeout: Some(300),
            origin: Some(GENERATED),
        }
    }

    fn der(&self) -> Vec<u8> {
        let mut hardware = Vec::new();
        if self.no_auth_required {
            hardware.push(tagged(503, der(TAG_NULL, &[])));
        }
        if let Some(kind) = self.user_auth_type {
            hardware.push(tagged(504, integer(kind)));
        }
        if let Some(seconds) = self.auth_timeout {
            hardware.push(tagged(505, integer(seconds)));
        }
        if let Some(origin) = self.origin {
            hardware.push(tagged(702, integer(origin)));
        }
        sequence(&[
            integer(400),                       // attestationVersion
            enumerated(self.attestation_level), // attestationSecurityLevel
            integer(400),                       // keyMintVersion
            enumerated(self.keymint_level),     // keyMintSecurityLevel
            octets(&self.challenge),            // attestationChallenge
            octets(&[]),                        // uniqueId
            sequence(&[]),                      // softwareEnforced
            sequence(&hardware),                // hardwareEnforced
        ])
    }

    /// The record inside a certificate, as `enrol.sh` would hand it over.
    fn pem(&self) -> String {
        let key = KeyPair::generate().expect("keypair");
        let mut params = CertificateParams::new(vec!["phone".to_string()]).expect("params");
        params
            .custom_extensions
            .push(CustomExtension::from_oid_content(
                ATTESTATION_OID,
                self.der(),
            ));
        params.self_signed(&key).expect("certificate").pem()
    }
}

/// Whether the named check passed. Panics if it did not run, because a check that
/// silently stopped running is the failure mode this whole file is about.
fn verdict(pem: &str, challenge: &[u8], what: &str) -> bool {
    let examination = examine(pem, challenge, Some(EMPTY_STATUS)).expect("examined");
    examination
        .findings
        .iter()
        .find(|finding| finding.what == what)
        .unwrap_or_else(|| panic!("no finding called {what:?}"))
        .ok
}

const EMPTY_STATUS: &str = r#"{"entries": {}}"#;

#[test]
fn a_record_is_read_back_as_it_was_written() {
    let claim = Claim::good(b"challenge");
    let record = parse_record(&claim.der()).expect("parsed");
    assert_eq!(record.attestation_security_level, SecurityLevel::StrongBox);
    assert_eq!(record.keymint_security_level, SecurityLevel::StrongBox);
    assert_eq!(record.challenge, b"challenge");
    assert_eq!(record.hardware.origin, Some(GENERATED));
    assert_eq!(record.hardware.auth_timeout, Some(300));
    assert!(!record.hardware.no_auth_required);
}

#[test]
fn a_chain_that_does_not_reach_google_is_refused_however_good_it_looks() {
    // The claim in this certificate is perfect. It is also self-signed, which is
    // what a fabricated attestation is: a phone saying what it knows you want.
    let challenge = b"fresh-challenge";
    let claim = Claim::good(challenge);
    let pem = claim.pem();
    assert!(
        !verdict(
            &pem,
            challenge,
            "the chain reaches a Google attestation root"
        ),
        "a self-signed chain must not pass the root check"
    );
    assert!(
        !examine(&pem, challenge, Some(EMPTY_STATUS))
            .expect("examined")
            .ok(),
        "and so the examination as a whole must fail"
    );
}

#[test]
fn a_record_answering_another_challenge_is_refused() {
    // The replay case: a chain captured from an honest enrolment months ago says
    // everything the checker wants to hear, about a key that may now be elsewhere.
    let claim = Claim::good(b"last-months-challenge");
    assert!(!verdict(
        &claim.pem(),
        b"todays-challenge",
        "the record answers this enrolment's challenge"
    ));
    assert!(verdict(
        &claim.pem(),
        b"last-months-challenge",
        "the record answers this enrolment's challenge"
    ));
}

#[test]
fn an_imported_key_is_refused() {
    // A key that was imported existed outside the secure element at some point,
    // which is the single guarantee the whole arrangement is bought for.
    let challenge = b"fresh-challenge";
    let mut claim = Claim::good(challenge);
    claim.origin = Some(IMPORTED);
    let what = "the key was generated in the element, not imported";
    assert!(!verdict(&claim.pem(), challenge, what));

    claim.origin = None;
    assert!(
        !verdict(&claim.pem(), challenge, what),
        "and saying nothing is not a pass"
    );
}

#[test]
fn a_key_nobody_has_to_unlock_is_refused() {
    let challenge = b"fresh-challenge";
    let mut claim = Claim::good(challenge);
    claim.no_auth_required = true;
    let what = "the key needs a person";
    assert!(!verdict(&claim.pem(), challenge, what));

    // And so is a key whose authentication is only the OS's promise: with no
    // authenticator type in the hardware-enforced list there is nothing the secure
    // element is holding anybody to.
    claim.no_auth_required = false;
    claim.user_auth_type = None;
    assert!(!verdict(&claim.pem(), challenge, what));
}

#[test]
fn a_tee_key_is_refused_even_when_the_record_claims_strongbox() {
    // The mixed case is the interesting one. A StrongBox claim signed at TEE level
    // is a claim made by software that does not hold the key it is describing.
    let challenge = b"fresh-challenge";
    let mut claim = Claim::good(challenge);
    claim.attestation_level = TEE;
    let what = "the key lives in StrongBox";
    assert!(!verdict(&claim.pem(), challenge, what));

    claim.attestation_level = STRONGBOX;
    claim.keymint_level = TEE;
    assert!(!verdict(&claim.pem(), challenge, what));
}

#[test]
fn skipping_the_revocation_list_is_a_failure_and_not_a_pass() {
    // The check that would otherwise quietly do nothing on a bad network, and read
    // afterwards as having been done.
    let challenge = b"fresh-challenge";
    let claim = Claim::good(challenge);
    let examination = examine(&claim.pem(), challenge, None).expect("examined");
    let finding = examination
        .findings
        .iter()
        .find(|finding| finding.what == "no certificate in the chain is revoked")
        .expect("the revocation finding");
    assert!(!finding.ok);
    assert!(finding.detail.contains("not checked"));
}

#[test]
fn a_revoked_serial_is_caught() {
    let challenge = b"fresh-challenge";
    let pem = Claim::good(challenge).pem();
    // rcgen picks the serial, so read it back rather than guessing it.
    let der = rustls_pemfile::certs(&mut pem.as_bytes())
        .next()
        .expect("a cert")
        .expect("valid");
    let (_, cert) = x509_parser::prelude::FromDer::from_der(&der[..]).expect("parsed");
    let cert: x509_parser::certificate::X509Certificate = cert;
    let serial: String = cert
        .raw_serial()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();

    let status = format!(r#"{{"entries": {{"{serial}": {{"status": "REVOKED"}}}}}}"#);
    let examination = examine(&pem, challenge, Some(&status)).expect("examined");
    let finding = examination
        .findings
        .iter()
        .find(|finding| finding.what == "no certificate in the chain is revoked")
        .expect("the revocation finding");
    assert!(!finding.ok, "a listed serial must be caught");
    assert!(finding.detail.contains("REVOKED"));
}

// ---- DER, by hand ----
//
// Small enough to write, and writing it is the point: a builder that shared code
// with the parser would agree with it about a mistake.

const TAG_INTEGER: u8 = 0x02;
const TAG_OCTET_STRING: u8 = 0x04;
const TAG_NULL: u8 = 0x05;
const TAG_ENUMERATED: u8 = 0x0a;
const TAG_SEQUENCE: u8 = 0x30;

fn der(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend(length(body.len()));
    out.extend_from_slice(body);
    out
}

fn length(n: usize) -> Vec<u8> {
    if n < 0x80 {
        return vec![n as u8];
    }
    let bytes: Vec<u8> = n
        .to_be_bytes()
        .into_iter()
        .skip_while(|byte| *byte == 0)
        .collect();
    let mut out = vec![0x80 | bytes.len() as u8];
    out.extend(bytes);
    out
}

fn integer(value: u64) -> Vec<u8> {
    der(TAG_INTEGER, &integer_body(value))
}

fn enumerated(value: u64) -> Vec<u8> {
    der(TAG_ENUMERATED, &integer_body(value))
}

fn integer_body(value: u64) -> Vec<u8> {
    let mut bytes: Vec<u8> = value
        .to_be_bytes()
        .into_iter()
        .skip_while(|byte| *byte == 0)
        .collect();
    if bytes.is_empty() {
        bytes.push(0);
    } else if bytes[0] & 0x80 != 0 {
        // Otherwise it would decode as negative.
        bytes.insert(0, 0);
    }
    bytes
}

fn octets(body: &[u8]) -> Vec<u8> {
    der(TAG_OCTET_STRING, body)
}

fn sequence(items: &[Vec<u8>]) -> Vec<u8> {
    der(TAG_SEQUENCE, &items.concat())
}

/// `[n] EXPLICIT`, with the high-tag-number form every AuthorizationList tag needs.
fn tagged(number: u32, inner: Vec<u8>) -> Vec<u8> {
    let mut tag = vec![0xa0 | 0x1f]; // context-specific, constructed, "see below"
    let mut base128 = Vec::new();
    let mut rest = number;
    loop {
        base128.insert(0, (rest & 0x7f) as u8);
        rest >>= 7;
        if rest == 0 {
            break;
        }
    }
    let last = base128.len() - 1;
    for byte in &mut base128[..last] {
        *byte |= 0x80;
    }
    tag.extend(base128);
    tag.extend(length(inner.len()));
    tag.extend(inner);
    tag
}
