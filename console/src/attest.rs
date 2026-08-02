//! Checking a phone's claim about the key it just made.
//!
//! Enrolment is the one moment where a mistake is permanent and quiet: whatever
//! key is pinned is thereafter *the* credential for arbitrary code execution on
//! this machine, and a key that is a file on a phone's filesystem looks exactly
//! like a key that lives in a secure element. Android Key Attestation is what
//! tells them apart — the keystore signs a record describing the key it just
//! generated, using a chain that goes back to a root Google published, so the
//! answer does not come from the software being asked about.
//!
//! **This exists because it is a paragraph otherwise.** The check runs once per
//! device, by hand, at the end of a fiddly session — which is exactly the shape of
//! a step that gets skipped, or done by eye, or done against a chain read out of
//! the device that is being checked. A binary that exits non-zero cannot be done
//! by eye.
//!
//! ## What is actually verified
//!
//! - Every signature in the chain, link by link.
//! - That the top of it is a Google attestation root **held here** rather than one
//!   the phone supplied.
//! - That no certificate in the chain appears in Google's revocation list — the
//!   entries that matter are the keys extracted from real devices.
//! - That the record's challenge is the one this enrolment generated, which is
//!   what makes it an answer rather than a recording.
//! - That the security level is StrongBox, that the key was *generated* rather
//!   than imported, and that it requires user authentication with a time limit.
//!
//! ## What it cannot verify
//!
//! That the phone in your hand is the phone that produced the chain. Nothing in a
//! certificate can say that. It is answered by the chain arriving over a USB cable
//! from a device you are holding, which is why enrolment is deliberately not a
//! network operation.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow, bail};
use x509_parser::asn1_rs::{Any, Integer, Tag};
use x509_parser::prelude::{FromDer, X509Certificate};

/// The roots, embedded rather than fetched. See the PEM's own header for why
/// there are two.
const ROOTS: &str = include_str!("attestation-roots.pem");

/// Google's `KeyDescription`, the extension the whole record lives in.
const ATTESTATION_OID: &str = "1.3.6.1.4.1.11129.2.1.17";

/// Where the keystore says a key lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityLevel {
    /// In the OS, which is to say nowhere in particular.
    Software,
    /// In the TEE — a separate world on the same silicon.
    TrustedEnvironment,
    /// In a discrete tamper-resistant chip. What a Pixel has and what is wanted.
    StrongBox,
    /// A level this was not written to know about; treated as not good enough.
    Other(u32),
}

impl SecurityLevel {
    fn of(value: u32) -> Self {
        match value {
            0 => Self::Software,
            1 => Self::TrustedEnvironment,
            2 => Self::StrongBox,
            other => Self::Other(other),
        }
    }
}

impl std::fmt::Display for SecurityLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Software => write!(f, "software"),
            Self::TrustedEnvironment => write!(f, "the TEE"),
            Self::StrongBox => write!(f, "StrongBox"),
            Self::Other(n) => write!(f, "an unknown security level ({n})"),
        }
    }
}

/// The handful of `AuthorizationList` entries this console has an opinion about.
///
/// The list has some seventy tags. Reading only these is deliberate: a check that
/// parsed everything would have to decide what every field meant, and the ones
/// omitted here are ones whose value could not change the answer.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Authorizations {
    /// `[503]` — present when the key may be used with nobody logged in.
    pub no_auth_required: bool,
    /// `[504]` — which authenticators count (1 = password/PIN, 2 = fingerprint).
    pub user_auth_type: Option<u64>,
    /// `[505]` — how many seconds one authentication lasts. Absent means the key
    /// is authenticated per use, which is stricter, not weaker.
    pub auth_timeout: Option<u64>,
    /// `[702]` — 0 when the secure element generated the key itself.
    pub origin: Option<u64>,
}

/// The keystore's description of one key.
#[derive(Debug)]
pub struct Record {
    pub attestation_version: u64,
    /// How trustworthy the *record* is.
    pub attestation_security_level: SecurityLevel,
    /// Where the *key* lives. These can differ, and the weaker one is the answer.
    pub keymint_security_level: SecurityLevel,
    pub challenge: Vec<u8>,
    /// Constraints the secure element enforces. The software-enforced list is
    /// deliberately not read: it is the OS's promise about itself.
    pub hardware: Authorizations,
}

/// One thing that was checked, and how it came out.
#[derive(Debug)]
pub struct Finding {
    pub ok: bool,
    pub what: &'static str,
    pub detail: String,
}

impl Finding {
    fn pass(what: &'static str, detail: impl Into<String>) -> Self {
        Self {
            ok: true,
            what,
            detail: detail.into(),
        }
    }

    fn fail(what: &'static str, detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            what,
            detail: detail.into(),
        }
    }
}

/// Everything enrolment wants to know about a chain.
pub struct Examination {
    pub findings: Vec<Finding>,
    /// SHA-256 of the leaf's public key — the line that goes in the config, and
    /// only worth having if every finding passed.
    pub pin: String,
    pub record: Record,
}

impl Examination {
    pub fn ok(&self) -> bool {
        self.findings.iter().all(|finding| finding.ok)
    }
}

/// Check a chain against a challenge, and against Google's revocation list.
///
/// `status` is the body of <https://android.googleapis.com/attestation/status>,
/// or `None`. None is **not** a pass: it produces a failing finding, because a
/// revocation check that silently does nothing when the network is down is worse
/// than no revocation check at all — it reads as having been done.
pub fn examine(chain_pem: &str, challenge: &[u8], status: Option<&str>) -> Result<Examination> {
    let ders: Vec<Vec<u8>> = rustls_pemfile::certs(&mut chain_pem.as_bytes())
        .map(|one| one.map(|one| one.to_vec()))
        .collect::<Result<_, _>>()
        .context("reading the chain")?;
    if ders.is_empty() {
        bail!("no certificates in the chain");
    }
    let certs: Vec<X509Certificate> = ders
        .iter()
        .map(|der| {
            X509Certificate::from_der(der)
                .map(|(_, cert)| cert)
                .map_err(|err| anyhow!("a certificate in the chain is unreadable: {err}"))
        })
        .collect::<Result<_>>()?;

    let mut findings = vec![
        links(&certs),
        reaches_a_root(&certs),
        revocations(&certs, status),
    ];

    let leaf = &certs[0];
    let record = record_of(leaf).context("the leaf carries no readable attestation record")?;
    findings.push(answers_the_challenge(&record, challenge));
    findings.push(lives_in_strongbox(&record));
    findings.push(was_generated_here(&record));
    findings.push(needs_a_person(&record));

    let pin = crate::tls::pin_of(&ders[0])?;
    Ok(Examination {
        findings,
        pin,
        record,
    })
}

/// The attestation record inside a certificate, if it has one.
pub fn record_of(cert: &X509Certificate) -> Result<Record> {
    let extension = cert
        .tbs_certificate
        .extensions()
        .iter()
        .find(|ext| ext.oid.to_id_string() == ATTESTATION_OID)
        .context("no attestation extension — this key was not attested")?;
    parse_record(extension.value)
}

/// Parse a `KeyDescription`.
///
/// Written against the schema rather than a library because there is no crate for
/// it that is worth a supply-chain entry on the machine this protects, and the
/// structure is eight fields.
pub fn parse_record(der: &[u8]) -> Result<Record> {
    let (_, outer) = Any::from_der(der).map_err(|err| anyhow!("not a DER record: {err}"))?;
    let fields = items(outer.data)?;
    // attestationVersion, attestationSecurityLevel, keyMintVersion,
    // keyMintSecurityLevel, attestationChallenge, uniqueId, softwareEnforced,
    // hardwareEnforced.
    if fields.len() < 8 {
        bail!(
            "a KeyDescription has eight fields; this has {}",
            fields.len()
        );
    }
    Ok(Record {
        attestation_version: integer(&fields[0], Tag::Integer)?,
        attestation_security_level: SecurityLevel::of(
            u32::try_from(integer(&fields[1], Tag::Enumerated)?).unwrap_or(u32::MAX),
        ),
        keymint_security_level: SecurityLevel::of(
            u32::try_from(integer(&fields[3], Tag::Enumerated)?).unwrap_or(u32::MAX),
        ),
        challenge: octets(&fields[4])?,
        hardware: authorizations(&fields[7])?,
    })
}

// ---- the individual checks ----

fn links(certs: &[X509Certificate]) -> Finding {
    let what = "every signature in the chain";
    for (index, pair) in certs.windows(2).enumerate() {
        if let Err(err) = pair[0].verify_signature(Some(pair[1].public_key())) {
            return Finding::fail(
                what,
                format!("certificate {index} is not signed by the one above it: {err}"),
            );
        }
    }
    Finding::pass(
        what,
        format!("{} certificates, each signed by the next", certs.len()),
    )
}

fn reaches_a_root(certs: &[X509Certificate]) -> Finding {
    let what = "the chain reaches a Google attestation root";
    let roots: Vec<Vec<u8>> = rustls_pemfile::certs(&mut ROOTS.as_bytes())
        .filter_map(|one| one.ok())
        .map(|one| one.to_vec())
        .collect();
    let top = certs.last().expect("a non-empty chain");
    for der in &roots {
        // The chain usually ends *at* the root; when it stops one short, the root
        // we hold has to have signed the last certificate instead. Either way the
        // trusted bytes are these, not the device's.
        if top.as_ref() == der.as_slice() {
            return Finding::pass(what, "it ends at a root held in this repository");
        }
        let Ok((_, root)) = X509Certificate::from_der(der) else {
            continue;
        };
        if top.verify_signature(Some(root.public_key())).is_ok() {
            return Finding::pass(what, "its top certificate is signed by a root held here");
        }
    }
    Finding::fail(
        what,
        "its top certificate is neither a known Google root nor signed by one — \
         which is what a fabricated chain looks like",
    )
}

fn revocations(certs: &[X509Certificate], status: Option<&str>) -> Finding {
    let what = "no certificate in the chain is revoked";
    let Some(status) = status else {
        return Finding::fail(
            what,
            "not checked — pass Google's status list, or this proves nothing",
        );
    };
    let entries: BTreeMap<String, serde_json::Value> =
        match serde_json::from_str::<serde_json::Value>(status) {
            Ok(serde_json::Value::Object(map)) => match map.get("entries") {
                Some(serde_json::Value::Object(entries)) => entries
                    .iter()
                    .map(|(k, v)| (k.to_lowercase(), v.clone()))
                    .collect(),
                _ => return Finding::fail(what, "the status list has no `entries`"),
            },
            _ => return Finding::fail(what, "the status list is not a JSON object"),
        };
    for cert in certs {
        for serial in serial_forms(cert.raw_serial()) {
            if let Some(entry) = entries.get(&serial) {
                return Finding::fail(what, format!("serial {serial} is listed: {entry}"));
            }
        }
    }
    Finding::pass(
        what,
        format!(
            "checked {} serials against {} entries",
            certs.len(),
            entries.len()
        ),
    )
}

fn answers_the_challenge(record: &Record, challenge: &[u8]) -> Finding {
    let what = "the record answers this enrolment's challenge";
    if record.challenge == challenge {
        return Finding::pass(what, format!("{} bytes, matching", challenge.len()));
    }
    Finding::fail(
        what,
        format!(
            "the record answers {} instead — a chain from some earlier enrolment, or \
             from another device",
            hex(&record.challenge)
        ),
    )
}

fn lives_in_strongbox(record: &Record) -> Finding {
    let what = "the key lives in StrongBox";
    // Both levels, and the weaker one decides: a StrongBox key described by a
    // TEE-signed record is a record that could have been written by compromised
    // TEE software about a key it does not hold.
    let levels = [
        record.attestation_security_level,
        record.keymint_security_level,
    ];
    if levels
        .iter()
        .all(|level| *level == SecurityLevel::StrongBox)
    {
        return Finding::pass(what, "and the record itself is StrongBox-signed");
    }
    Finding::fail(
        what,
        format!(
            "the record is signed by {} and describes a key in {}",
            record.attestation_security_level, record.keymint_security_level
        ),
    )
}

fn was_generated_here(record: &Record) -> Finding {
    let what = "the key was generated in the element, not imported";
    // An imported key existed outside the secure element at some point, which is
    // the one thing this whole arrangement is buying.
    const GENERATED: u64 = 0;
    match record.hardware.origin {
        Some(GENERATED) => Finding::pass(what, "origin is GENERATED"),
        Some(other) => Finding::fail(what, format!("origin is {other}, not GENERATED")),
        None => Finding::fail(
            what,
            "the hardware-enforced list does not say where it came from",
        ),
    }
}

fn needs_a_person(record: &Record) -> Finding {
    let what = "the key needs a person";
    if record.hardware.no_auth_required {
        return Finding::fail(
            what,
            "NO_AUTH_REQUIRED — a snatched locked phone would still work",
        );
    }
    match (record.hardware.user_auth_type, record.hardware.auth_timeout) {
        (Some(kind), Some(seconds)) => Finding::pass(
            what,
            format!("authenticator type {kind}, one unlock lasting {seconds}s"),
        ),
        (Some(kind), None) => Finding::pass(
            what,
            format!("authenticator type {kind}, authenticated per use"),
        ),
        (None, _) => Finding::fail(
            what,
            "no authenticator type is enforced by the hardware, so the requirement is \
             the OS's word",
        ),
    }
}

// ---- DER ----

/// Every TLV in a sequence's content, in order.
fn items(content: &[u8]) -> Result<Vec<Any<'_>>> {
    let mut rest = content;
    let mut out = Vec::new();
    while !rest.is_empty() {
        let (next, any) = Any::from_der(rest).map_err(|err| anyhow!("malformed DER: {err}"))?;
        out.push(any);
        rest = next;
    }
    Ok(out)
}

fn integer(field: &Any, expected: Tag) -> Result<u64> {
    if field.header.tag() != expected {
        bail!("expected {expected}, found {}", field.header.tag());
    }
    Integer::new(field.data)
        .as_u64()
        .map_err(|err| anyhow!("not an integer we can read: {err}"))
}

fn octets(field: &Any) -> Result<Vec<u8>> {
    if field.header.tag() != Tag::OctetString {
        bail!("expected an octet string, found {}", field.header.tag());
    }
    Ok(field.data.to_vec())
}

fn authorizations(field: &Any) -> Result<Authorizations> {
    // Google's tag numbers, from the KeyMint AuthorizationList. Written as the
    // numbers they are: the names in the schema are the only names they have.
    const NO_AUTH_REQUIRED: u32 = 503;
    const USER_AUTH_TYPE: u32 = 504;
    const AUTH_TIMEOUT: u32 = 505;
    const ORIGIN: u32 = 702;

    let mut out = Authorizations::default();
    for entry in items(field.data)? {
        match entry.header.tag().0 {
            NO_AUTH_REQUIRED => out.no_auth_required = true,
            USER_AUTH_TYPE => out.user_auth_type = Some(tagged_integer(&entry)?),
            AUTH_TIMEOUT => out.auth_timeout = Some(tagged_integer(&entry)?),
            ORIGIN => out.origin = Some(tagged_integer(&entry)?),
            // Some seventy other tags, none of which change the answer.
            _ => {}
        }
    }
    Ok(out)
}

/// The integer inside an `[N] EXPLICIT INTEGER` — one more TLV down.
fn tagged_integer(entry: &Any) -> Result<u64> {
    let (_, inner) =
        Any::from_der(entry.data).map_err(|err| anyhow!("malformed tagged value: {err}"))?;
    integer(&inner, Tag::Integer)
}

/// The forms a serial number can take in Google's status list.
///
/// It publishes lowercase hex with no leading zero, but a DER serial carries one
/// whenever its high bit would otherwise make it negative — so the obvious
/// rendering misses exactly half the entries, silently, and a missed revocation
/// looks the same as no revocation.
fn serial_forms(raw: &[u8]) -> Vec<String> {
    let full = hex(raw);
    let trimmed = full.trim_start_matches('0').to_string();
    if trimmed.is_empty() || trimmed == full {
        vec![full]
    } else {
        vec![full, trimmed]
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
