//! Corpus loading, link graph, search and rendering — exercised through the
//! public API against fixture memories shaped like the real corpus
//! (frontmatter + `[[wikilinks]]` + a MEMORY.md index).

use memview::store::{Corpus, render_markdown};

fn corpus() -> Corpus {
    Corpus::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/memory"
    ))
    .expect("fixture corpus loads")
}

#[test]
fn loads_every_memory_but_not_the_index() {
    let corpus = corpus();
    let mut names: Vec<&str> = corpus.docs.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["feedback_gamma", "project_alpha", "reference_beta"]);
    // MEMORY.md is the index, not a memory of its own.
    assert!(
        corpus
            .index_md
            .expect("index present")
            .contains("# Memory index")
    );
}

#[test]
fn reads_description_and_type_from_frontmatter() {
    let corpus = corpus();
    let alpha = corpus.get("project_alpha").expect("alpha present");
    assert_eq!(alpha.meta.mtype, "project");
    assert!(alpha.meta.description.contains("the alpha service"));
    // The frontmatter is stripped from the body, not rendered as content.
    assert!(!alpha.body.contains("originSessionId"));
    assert!(alpha.body.contains("the alpha service"));
}

#[test]
fn type_falls_back_to_the_filename_prefix_without_frontmatter() {
    let corpus = corpus();
    let gamma = corpus.get("feedback_gamma").expect("gamma present");
    assert_eq!(gamma.meta.mtype, "feedback");
    assert_eq!(gamma.meta.description, "");
}

#[test]
fn outlinks_separate_written_memories_from_dangling_ones() {
    let corpus = corpus();
    let alpha = corpus.get("project_alpha").expect("alpha present");
    let (existing, dangling) = corpus.outlinks(alpha);
    let mut names: Vec<&str> = existing.iter().map(|m| m.name.as_str()).collect();
    names.sort_unstable();
    // `[[feedback_gamma|the habit]]` is piped AND wrapped across a source line;
    // comrak still renders it as one link, so the graph must resolve it too.
    assert_eq!(names, ["feedback_gamma", "reference_beta"]);
    assert_eq!(dangling, ["project_not_written_yet"]);
}

#[test]
fn backlinks_find_the_memories_pointing_here() {
    let corpus = corpus();
    let backlinks = corpus.backlinks("reference_beta");
    let names: Vec<&str> = backlinks.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["project_alpha"]);
    // Backlinks and outlinks agree: alpha's wrapped `[[feedback_gamma|…]]`
    // counts from both directions.
    let gamma_backlinks = corpus.backlinks("feedback_gamma");
    let gamma_names: Vec<&str> = gamma_backlinks.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(gamma_names, ["project_alpha"]);
    // A not-yet-written memory still collects backlinks — that's the signal for
    // what's worth writing.
    let pending = corpus.backlinks("project_not_written_yet");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].name, "project_alpha");
    // A name nobody mentions has none.
    assert!(corpus.backlinks("project_unmentioned").is_empty());
}

#[test]
fn search_ranks_name_and_description_hits_above_body_hits() {
    let corpus = corpus();
    let hits = corpus.search("beta");
    let names: Vec<&str> = hits.iter().map(|h| h.meta.name.as_str()).collect();
    // reference_beta matches on name+description; project_alpha only in body.
    assert_eq!(names, ["reference_beta", "project_alpha"]);
    // The body hit quotes its surrounding markdown, not just the term.
    let snippet = hits[1].snippet.as_ref().expect("body hit has a snippet");
    assert!(snippet.contains("[[reference_beta]]"), "{snippet}");
}

#[test]
fn search_is_case_insensitive_and_empty_for_no_query() {
    let corpus = corpus();
    assert_eq!(corpus.search("WIREGUARD").len(), 1);
    assert!(corpus.search("").is_empty());
}

#[test]
fn search_snippets_survive_multibyte_bodies() {
    // The snippet window is a byte range around the match; a corpus full of
    // accented prose must not slice a character in half (that would panic).
    let corpus = corpus();
    let hits = corpus.search("naïve");
    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.as_ref().expect("snippet").contains("naïve"));
}

#[test]
fn search_snippet_offset_survives_cased_multibyte_prefix() {
    // 'İ' is 2 bytes but lowercases to "i̇" (3 bytes), so `body.to_lowercase()`
    // is longer than `body` — and a match offset found in the lowercased copy
    // points too far into the ORIGINAL. With enough such chars before the match,
    // the drift exceeds the snippet's back-window and the naive offset would slice
    // PAST the match entirely, dropping it from the snippet. The correct offset
    // (found against the original) keeps it centred.
    let dir = tempfile::tempdir().expect("tempdir");
    let prefix = "İ ".repeat(120); // ~120 bytes of lowercase drift, > the 80-byte window
    std::fs::write(
        dir.path().join("MEMORY.md"),
        "# Memory index\n- [p](project_drift.md)\n",
    )
    .expect("write index");
    std::fs::write(
        dir.path().join("project_drift.md"),
        format!(
            "---\nname: project_drift\ndescription: d\nmetadata:\n  type: project\n---\n\n{prefix}NEEDLETOKEN tail\n"
        ),
    )
    .expect("write memory");

    let corpus = Corpus::load(dir.path()).expect("loads");
    let hits = corpus.search("needletoken");
    assert_eq!(hits.len(), 1);
    let snippet = hits[0].snippet.as_ref().expect("body snippet");
    assert!(
        snippet.contains("NEEDLETOKEN"),
        "snippet windowed the wrong offset: {snippet:?}"
    );
}

#[test]
fn rendering_rewrites_both_link_forms_but_leaves_external_urls() {
    let html = render_markdown(
        "See [[project_alpha]] and [Title](reference_beta.md) and [ext](https://x.example/).",
    )
    .expect("renders");
    assert!(html.contains("href=\"/m/project_alpha\""), "{html}");
    assert!(html.contains("href=\"/m/reference_beta\""), "{html}");
    assert!(html.contains("href=\"https://x.example/\""), "{html}");
}

#[test]
fn rendering_escapes_raw_html_rather_than_dropping_it() {
    // Memory bodies quote tags in prose (`<system-reminder>`); the reader must
    // still see the text.
    let html = render_markdown("a <system-reminder> in prose").expect("renders");
    assert!(html.contains("&lt;system-reminder&gt;"), "{html}");
}

#[test]
fn missing_corpus_directory_is_an_error_not_an_empty_corpus() {
    assert!(Corpus::load("/nonexistent/memview/corpus").is_err());
}
