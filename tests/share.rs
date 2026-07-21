//! The public share token: one active link at a time, rotation kills the
//! previous one immediately, and the state survives a restart.

use memview::share::{ShareStore, build_share_url};

#[test]
fn rotate_replaces_the_previous_link_and_survives_a_reload() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("share.json");
    let store = ShareStore::load(&path).expect("loads empty");
    assert!(store.get().is_none());

    let first = store.rotate().expect("rotate");
    assert!(store.is_valid(&first.token));

    let second = store.rotate().expect("rotate again");
    assert!(!store.is_valid(&first.token), "old link must stop working");
    assert!(store.is_valid(&second.token));

    let reloaded = ShareStore::load(&path).expect("reloads");
    assert!(reloaded.is_valid(&second.token));
}

#[test]
fn revoke_removes_access_and_persists() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("share.json");
    let store = ShareStore::load(&path).expect("loads");
    let active = store.rotate().expect("rotate");

    store.revoke().expect("revoke");
    assert!(!store.is_valid(&active.token));
    assert!(ShareStore::load(&path).expect("reloads").get().is_none());
}

#[test]
fn tokens_are_unguessable_and_unique() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = ShareStore::load(dir.path().join("share.json")).expect("loads");
    let a = store.rotate().expect("rotate").token;
    let b = store.rotate().expect("rotate").token;
    assert_ne!(a, b);
    // 32 random bytes → 43 base64url chars.
    assert_eq!(a.len(), 43);
    assert!(
        a.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    );
}

#[test]
fn touch_records_access_without_changing_the_token() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("share.json");
    let store = ShareStore::load(&path).expect("loads");
    let active = store.rotate().expect("rotate");
    assert!(active.last_accessed_at.is_none());

    store.touch();
    let after = store.get().expect("still active");
    assert_eq!(after.token, active.token);
    assert!(after.last_accessed_at.is_some());
    // Persisted, so the owner sees it after a restart.
    let reloaded = ShareStore::load(&path)
        .expect("reloads")
        .get()
        .expect("active");
    assert!(reloaded.last_accessed_at.is_some());
}

#[test]
fn share_urls_join_cleanly_regardless_of_trailing_slash() {
    assert_eq!(
        build_share_url("https://x.example", "tok"),
        "https://x.example/share/tok"
    );
    assert_eq!(
        build_share_url("https://x.example/", "tok"),
        "https://x.example/share/tok"
    );
}
