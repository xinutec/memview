//! The dependency list is the boundary, so it is pinned rather than trusted.
//!
//! Both binaries in this workspace link this crate, and they sit at different
//! privilege levels: one is a read-only viewer on an internet-facing host, the
//! other spawns Claude Code processes on the root-of-truth Mac. Sharing is safe
//! only for as long as there is nothing here to misconfigure — no client, no
//! server, no process, no credential.
//!
//! That property is invisible in the code. Nothing fails to compile when a
//! dependency arrives, and the review that would have caught it is a diff in a
//! manifest, which is the easiest kind of line to wave through. So it is
//! asserted: adding one is a decision that has to be made twice, here and in
//! `Cargo.toml`, and the second time with this list in front of you.

/// What may be linked, and why each earns its place.
///
/// Two parser generators for the grammars, a base64 decoder for the heredocs the
/// transcripts carry encoded, an error type, serde for the artefact shapes, and
/// a hash. Every one of them is a pure transformation of bytes already in hand.
///
/// ⚠ **`sha2` is here as a HASH, not as a credential primitive.** It
/// fingerprints the stretch of a transcript a resumed read has already consumed
/// (`watermark.rs`), so drift in a file can be detected without re-reading it —
/// no key, no secret, no verification of anything anybody sent. If a future use
/// of it authenticates, signs, or compares against something a user supplied,
/// that use belongs in whichever crate holds the trust, not in this one.
const ALLOWED: [&str; 7] = [
    "anyhow",
    "base64",
    "pest",
    "pest_derive",
    "serde",
    "serde_json",
    "sha2",
];

/// The names under `[dependencies]`, in order.
///
/// Hand-read rather than parsed with a TOML crate, which would be a seventh
/// dependency to check the other six — and a dev-dependency at that, so the test
/// guarding the boundary would have widened it.
///
/// `[dev-dependencies]` is deliberately not read: those are linked into no
/// binary either crate ships, so a tree-sitter probe measuring the grammar costs
/// the boundary nothing.
fn declared() -> Vec<String> {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("reading this crate's own manifest");
    manifest
        .lines()
        .skip_while(|line| line.trim() != "[dependencies]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .filter_map(|line| line.split_once('='))
        .map(|(name, _)| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

#[test]
fn the_leaf_stays_a_leaf() {
    let mut found = declared();
    found.sort();
    assert_eq!(
        found, ALLOWED,
        "the reader's dependencies changed. It is linked by both a viewer that \
         serves the internet and a console that spawns processes, and it is safe \
         for both only while it can do neither. If the new one is genuinely a \
         pure transformation of bytes, add it to ALLOWED with the reason; if it \
         opens, serves, spawns or authenticates anything, it belongs in whichever \
         crate needs it instead."
    );
}
