//! Commit hashes, and the rule that says which agent made one.
//!
//! The join exists because every commit in this fleet has the same git author,
//! so the repository itself can say nothing about who wrote what.

use memview::commits::hash_candidates;

#[test]
fn a_short_hash_is_recognised_because_that_is_what_git_prints() {
    // `git commit` and `git log --oneline` both print seven characters.
    // Requiring nine attributed 1 commit of 17, and the failure was silent —
    // the miner reported almost no commits rather than an error.
    assert_eq!(
        hash_candidates(b"[main f9d10a7] Read the files the shell touched"),
        ["f9d10a7"]
    );
}

#[test]
fn a_line_number_is_not_a_hash() {
    // `1234567` is valid hex and is almost always a count, an offset or a
    // timestamp. Requiring a letter costs nothing: a real hash without one of
    // `a`-`f` in seven characters is a one-in-thirty-five event.
    assert!(hash_candidates(b"wrote 1234567 bytes in 3600000 ms").is_empty());
}

#[test]
fn a_sha256_is_not_a_git_hash() {
    // The corpus is full of them — lockfiles, nix output, checksums — and each
    // one offers a seven-character window that could collide with a real short
    // hash. A run longer than forty characters is not a git hash at all.
    let long = b"sha256-0000000000000000000000000000000000000000000000000000000000abcdef";
    assert!(hash_candidates(long).is_empty());
}

#[test]
fn a_hash_inside_a_longer_token_is_not_a_mention() {
    // Base64 and file names embed hex runs that mean nothing. The run must be
    // bounded by non-alphanumerics on both sides to count.
    assert!(hash_candidates(b"zzf9d10a7zz").is_empty());
    assert_eq!(hash_candidates(b"see commit f9d10a7."), ["f9d10a7"]);
}

#[test]
fn each_segment_of_a_uuid_is_offered_separately() {
    // Session ids are everywhere in the transcripts and are hex in parts. They
    // are not rejected here — the lookup against the known hashes is what
    // rejects them — but the scan must not swallow a real hash next to one.
    let ids = hash_candidates(b"7c0202eb-080b-40a5-a654-8758b4ca723e a4f4b7c");
    assert!(ids.contains(&"a4f4b7c"), "the real hash survives: {ids:?}");
}

/// The `--numstat` rename forms, which are the only place in the fleet's
/// evidence where two names are known to be one file.
#[test]
fn a_renamed_path_gives_both_of_its_names() {
    use memview::commits::renamed;
    let owned = |(was, path): (Option<String>, String)| (was.unwrap_or_default(), path);

    // Nothing in common: git drops the braces.
    assert_eq!(
        owned(renamed("src/geo/osm.ts => src/geo/overpass.ts")),
        (
            "src/geo/osm.ts".to_string(),
            "src/geo/overpass.ts".to_string()
        )
    );
    // A shared prefix and suffix, written once.
    assert_eq!(
        owned(renamed("code/kubes/ircd/{inspircd => k8s}/ircd.yaml")),
        (
            "code/kubes/ircd/inspircd/ircd.yaml".to_string(),
            "code/kubes/ircd/k8s/ircd.yaml".to_string()
        )
    );
    // Moved INTO a directory: the old side is empty, and the doubled separator
    // it leaves behind is a different path from the one git meant.
    assert_eq!(
        owned(renamed("code/kubes/ircd/{ => k8s}/net.yaml")),
        (
            "code/kubes/ircd/net.yaml".to_string(),
            "code/kubes/ircd/k8s/net.yaml".to_string()
        )
    );
    // …and out of one.
    assert_eq!(
        owned(renamed("plan/{old => }/types.dhall")),
        (
            "plan/old/types.dhall".to_string(),
            "plan/types.dhall".to_string()
        )
    );
}

#[test]
fn an_ordinary_path_is_left_alone() {
    use memview::commits::renamed;
    // Including one with a brace in it, which is a filename and not a rename.
    for path in ["src/geo/osm.ts", "frontend/src/app/{weird}.ts"] {
        let (was, out) = renamed(path);
        assert_eq!((was, out), (None, path.to_string()));
    }
}
