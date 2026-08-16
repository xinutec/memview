//! The post-login redirect target, which is attacker-settable by design.
//!
//! `?return_to=` is a query parameter on `/login`, so whatever it holds reaches
//! `Location:` after a successful sign-in. The only thing standing between a
//! crafted link and an off-site redirect is [`validate_return_to`], which makes
//! the vectors below the point of this file rather than a flourish: each is a
//! published way of spelling an absolute URL that still begins with a slash.

use memview::routes::auth::validate_return_to;

#[test]
fn keeps_an_internal_path_whole() {
    for path in [
        "/",
        "/memory/project_thing",
        "/search?q=needle",
        "/search?q=a+b&sort=rank",
        // A backslash past the first character cannot open an authority, so it
        // is an ordinary path character and the return survives it.
        "/search?q=a%5Cb",
        r"/search?q=a\b",
    ] {
        assert_eq!(validate_return_to(Some(path)), path, "{path:?} is internal");
    }
}

#[test]
fn refuses_every_spelling_that_leaves_the_site() {
    for path in [
        // Protocol-relative: the classic.
        "//attacker.example",
        "//attacker.example/path",
        // ⚠ The URL Standard treats `\` as `/` for http(s), so a browser reads
        // this as `//attacker.example` and navigates off-site. It starts with a
        // slash and not with `//`, which is exactly why a `!starts_with("//")`
        // test passed it through.
        r"/\attacker.example",
        r"/\/attacker.example",
        r"\\attacker.example",
        // Absolute, in the forms that do not begin with a slash at all.
        "https://attacker.example",
        "http://attacker.example",
        r"\/attacker.example",
        // No scheme-relative form survives either.
        "attacker.example",
        "javascript:alert(1)",
    ] {
        assert_eq!(
            validate_return_to(Some(path)),
            "/",
            "{path:?} must not be a redirect target"
        );
    }
}

#[test]
fn an_absent_or_empty_target_is_the_root() {
    assert_eq!(validate_return_to(None), "/");
    assert_eq!(validate_return_to(Some("")), "/");
}
