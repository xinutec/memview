//! A memory whose content changed while its stamp did not (#1199's sibling —
//! the verification I was doing by hand).
//!
//! ⚠ **Every case here is a diff the corpus actually produces.** The stamp is
//! what recall's age banner reads, so a memory that keeps an old stamp over new
//! content claims to be fresher than it is — and understating age is the
//! dangerous direction, because it buys LESS scrutiny.

use memview::stamped::unstamped;

const HEAD: &str = "diff --git a/memory/project_x.md b/memory/project_x.md\n\
                    index 111..222 100644\n\
                    --- a/memory/project_x.md\n\
                    +++ b/memory/project_x.md\n\
                    @@ -1,4 +1,4 @@\n";

#[test]
fn a_body_edit_without_a_stamp_edit_is_reported() {
    let diff = format!("{HEAD}-old prose\n+new prose\n");
    assert_eq!(unstamped(&diff), vec!["memory/project_x.md".to_string()]);
}

#[test]
fn a_body_edit_with_the_stamp_moved_is_clean() {
    let diff = format!(
        "{HEAD}-  modified: 2026-08-01T00:00:00.000Z\n\
         +  modified: 2026-08-29T00:00:00.000Z\n\
         -old prose\n+new prose\n"
    );
    assert!(unstamped(&diff).is_empty());
}

/// Stamping and nothing else is legitimate — `memory-stamp` repairing a missing
/// stamp does exactly this, and reporting it would make the repair tool fail the
/// gate that asked for it.
#[test]
fn moving_only_the_stamp_is_not_a_finding() {
    let diff = format!(
        "{HEAD}-  modified: 2026-08-01T00:00:00.000Z\n\
         +  modified: 2026-08-29T00:00:00.000Z\n"
    );
    assert!(unstamped(&diff).is_empty());
}

/// ⚠ A NEW memory has no previous stamp to advance. Reporting it would fail
/// every commit that writes a memory, which is the ordinary case.
#[test]
fn a_newly_added_memory_is_not_reported() {
    let diff = "diff --git a/memory/project_new.md b/memory/project_new.md\n\
                new file mode 100644\n\
                --- /dev/null\n\
                +++ b/memory/project_new.md\n\
                @@ -0,0 +1,3 @@\n\
                +---\n+name: project_new\n+---\n";
    assert!(unstamped(diff).is_empty());
}

/// ⚠ **The `+++`/`---` headers start with the same characters as content.**
/// Counting them as edits would mark every file in every diff as changed, and
/// the check would fire on a commit that only moved stamps.
#[test]
fn the_file_headers_are_not_read_as_content() {
    // A diff with headers and nothing else cannot be a body change.
    assert!(unstamped(HEAD).is_empty());
}

#[test]
fn several_files_are_judged_independently() {
    let clean = "diff --git a/memory/a.md b/memory/a.md\n\
                 --- a/memory/a.md\n+++ b/memory/a.md\n\
                 -  modified: 2026-08-01T00:00:00.000Z\n\
                 +  modified: 2026-08-29T00:00:00.000Z\n\
                 -x\n+y\n";
    let dirty = "diff --git a/memory/b.md b/memory/b.md\n\
                 --- a/memory/b.md\n+++ b/memory/b.md\n\
                 -x\n+y\n";
    assert_eq!(
        unstamped(&format!("{clean}{dirty}")),
        vec!["memory/b.md".to_string()]
    );
}

/// Anything that is not a memory is none of this check's business — the same
/// commit carries code, and a `.rs` file has no stamp to move.
#[test]
fn a_file_that_is_not_a_memory_is_ignored() {
    let diff = "diff --git a/src/lib.rs b/src/lib.rs\n\
                --- a/src/lib.rs\n+++ b/src/lib.rs\n-x\n+y\n";
    assert!(unstamped(diff).is_empty());
}

/// The last file in a diff has no `diff --git` after it to close it, so a
/// version that only reported on the next header would miss it — which is the
/// commonest shape, one file changed.
#[test]
fn the_last_file_in_the_diff_is_still_judged() {
    let diff = format!("{HEAD}-old\n+new\n");
    assert_eq!(unstamped(&diff).len(), 1);
}
