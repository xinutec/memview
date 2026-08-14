//! Corpus loading, link graph, search and rendering — exercised through the
//! public API against fixture memories shaped like the real corpus
//! (frontmatter + `[[wikilinks]]` + a MEMORY.md index).

use memview::store::{Corpus, Graph, GraphNode, has_section, render_markdown};

fn corpus() -> Corpus {
    Corpus::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/memory"
    ))
    .expect("fixture corpus loads")
}

fn node<'a>(graph: &'a Graph, name: &str) -> &'a GraphNode {
    graph
        .nodes
        .iter()
        .find(|n| n.meta.name == name)
        .unwrap_or_else(|| panic!("{name} is a node"))
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
fn the_origin_session_is_read_from_frontmatter_when_the_memory_declares_one() {
    let corpus = corpus();
    let alpha = corpus.get("project_alpha").expect("alpha present");
    assert_eq!(
        alpha.origin_session.as_deref(),
        Some("00000000-0000-0000-0000-000000000000")
    );
    // Frontmatter without the key, and no frontmatter at all, are both simply
    // absent — 10 of the live corpus's 384 memories predate the field, and an
    // empty string would render as an origin nothing can resolve.
    assert_eq!(
        corpus.get("reference_beta").expect("beta").origin_session,
        None
    );
    assert_eq!(
        corpus.get("feedback_gamma").expect("gamma").origin_session,
        None
    );
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
    // The misspelt relation dangles with its prefix intact — that is what makes
    // a typo findable instead of leaving it to pass as an untyped link.
    assert_eq!(
        dangling,
        ["project_not_written_yet", "superseeds:reference_beta"]
    );
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

/// `Corpus::search` with no mined usage — which is what CI has, and what every
/// test here means by "search".
fn search(corpus: &memview::store::Corpus, q: &str) -> Vec<memview::store::SearchHit> {
    corpus.search(q, &std::collections::BTreeMap::new()).hits
}

#[test]
fn search_ranks_name_and_description_hits_above_body_hits() {
    let corpus = corpus();
    let hits = search(&corpus, "beta");
    let names: Vec<&str> = hits.iter().map(|h| h.meta.name.as_str()).collect();
    // reference_beta matches on name+description; project_alpha only in body.
    assert_eq!(names, ["reference_beta", "project_alpha"]);
    // The body hit quotes its surroundings, rendered — the wikilink around the
    // term keeps its text and loses its brackets, because a snippet is read and
    // not navigated.
    let snippet = hits[1].snippet.as_ref().expect("body hit has a snippet");
    assert!(snippet.contains("reference_beta"), "{snippet}");
    assert!(!snippet.contains("[["), "{snippet}");
}

#[test]
fn search_is_case_insensitive_and_empty_for_no_query() {
    let corpus = corpus();
    assert_eq!(search(&corpus, "WIREGUARD").len(), 1);
    assert!(search(&corpus, "").is_empty());
}

#[test]
fn search_snippets_survive_multibyte_bodies() {
    // The snippet window is a byte range around the match; a corpus full of
    // accented prose must not slice a character in half (that would panic).
    let corpus = corpus();
    let hits = search(&corpus, "naïve");
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
    let hits = search(&corpus, "needletoken");
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

#[test]
fn graph_nodes_carry_the_curated_section_size_and_degrees() {
    let graph = corpus().graph();
    assert_eq!(graph.nodes.len(), 3);

    let alpha = node(&graph, "project_alpha");
    assert_eq!(alpha.section.as_deref(), Some("Projects"));
    // Alpha wikilinks three names but only two are written; a dangling link has
    // no node to point at, so it is not out-degree either.
    assert_eq!(alpha.out_degree, 2);
    // reference_beta's body ends "Related: [[project_alpha]]", which resolves.
    assert_eq!(alpha.in_degree, 1);
    assert!(alpha.size > 0, "body length feeds node radius");

    let gamma = node(&graph, "feedback_gamma");
    assert_eq!(gamma.section.as_deref(), Some("Working rules"));
    assert_eq!(gamma.in_degree, 1);
    assert_eq!(gamma.out_degree, 0);
}

#[test]
fn graph_edges_are_the_resolvable_wikilinks_only() {
    let graph = corpus().graph();
    let mut pairs: Vec<(&str, &str)> = graph
        .edges
        .iter()
        .map(|e| (e.source.as_str(), e.target.as_str()))
        .collect();
    pairs.sort_unstable();
    assert_eq!(
        pairs,
        [
            ("project_alpha", "feedback_gamma"),
            ("project_alpha", "reference_beta"),
            ("reference_beta", "project_alpha"),
        ]
    );
}

#[test]
fn graph_sections_keep_the_index_order_not_alphabetical() {
    // A legend follows the order Pippijn curated in MEMORY.md.
    assert_eq!(corpus().graph().sections, ["Projects", "Working rules"]);
}

#[test]
fn graph_ignores_self_links_collapses_repeats_and_reports_unindexed_memories() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("MEMORY.md"),
        "# Memory index\n\npreamble [why](project_b.md)\n\n## Only section\n- [a](project_a.md)\n",
    )
    .expect("write index");
    std::fs::write(
        dir.path().join("project_a.md"),
        "---\nname: project_a\ndescription: a\nmetadata:\n  type: project\n---\n\n\
         Cites [[project_b]] twice: [[project_b]]. And itself, [[project_a]].\n",
    )
    .expect("write a");
    std::fs::write(
        dir.path().join("project_b.md"),
        "---\nname: project_b\ndescription: b\nmetadata:\n  type: project\n---\n\nNothing.\n",
    )
    .expect("write b");

    let graph = Corpus::load(dir.path()).expect("loads").graph();
    let pairs: Vec<(&str, &str)> = graph
        .edges
        .iter()
        .map(|e| (e.source.as_str(), e.target.as_str()))
        .collect();
    // Two mentions of the same target are one relationship; a self-link is none.
    assert_eq!(pairs, [("project_a", "project_b")]);
    assert_eq!(node(&graph, "project_a").out_degree, 1);
    assert_eq!(
        node(&graph, "project_a").section.as_deref(),
        Some("Only section")
    );
    // project_b is linked only from the preamble, above any `##` heading. It has
    // no section — reported as such rather than bucketed into a fake catch-all,
    // because "indexed nowhere" is a real thing to be able to see.
    assert_eq!(node(&graph, "project_b").section, None);
}

#[test]
fn graph_of_a_corpus_without_an_index_still_has_nodes_and_edges() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("project_a.md"),
        "---\nname: project_a\ndescription: a\n---\n\nSee [[project_b]].\n",
    )
    .expect("write a");
    std::fs::write(
        dir.path().join("project_b.md"),
        "---\nname: project_b\ndescription: b\n---\n\nNothing.\n",
    )
    .expect("write b");

    let graph = Corpus::load(dir.path()).expect("loads").graph();
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 1);
    assert!(graph.sections.is_empty());
    assert!(graph.nodes.iter().all(|n| n.section.is_none()));
}

#[test]
fn a_typed_link_records_its_relation_on_the_edge() {
    let graph = corpus().graph();
    let edge = graph
        .edges
        .iter()
        .find(|e| e.source == "project_alpha" && e.target == "feedback_gamma")
        .expect("alpha links gamma");
    // The body mentions gamma twice: once plainly and once as `governs:`. A
    // claim beats a passing mention, whichever the parser reaches first.
    assert_eq!(edge.relation.as_deref(), Some("governs"));
}

#[test]
fn an_untyped_link_claims_nothing() {
    let graph = corpus().graph();
    let edge = graph
        .edges
        .iter()
        .find(|e| e.source == "project_alpha" && e.target == "reference_beta")
        .expect("alpha links beta");
    // beta is linked plainly first, then as `because:`. Same rule as above.
    assert_eq!(edge.relation.as_deref(), Some("because"));
}

#[test]
fn a_misspelt_relation_dangles_instead_of_passing_as_untyped() {
    // The whole point of a closed vocabulary. `superseeds:reference_beta` must
    // NOT resolve to reference_beta — a typo that silently degrades to a plain
    // link is a typo nobody ever finds.
    let corpus = corpus();
    let doc = corpus.get("project_alpha").expect("alpha exists");
    let (_, dangling) = corpus.outlinks(doc);
    assert!(
        dangling.iter().any(|d| d == "superseeds:reference_beta"),
        "expected the typo to dangle, got {dangling:?}"
    );
}

#[test]
fn rendering_puts_the_relation_on_the_link_not_in_the_sentence() {
    let html = render_markdown("A rule that [[governs:project_alpha]] the work.").unwrap();
    // The prose must read "…that project_alpha the work", never
    // "governs:project_alpha" — the relation is structure, not wording.
    assert!(html.contains(r#"href="/m/project_alpha""#), "{html}");
    assert!(html.contains(r#"title="governs""#), "{html}");
    assert!(!html.contains("governs:project_alpha"), "{html}");
}

#[test]
fn an_untyped_wikilink_renders_exactly_as_before() {
    let html = render_markdown("See [[project_alpha]].").unwrap();
    assert!(html.contains(r#"href="/m/project_alpha""#), "{html}");
    assert!(!html.contains("title="), "{html}");
}

#[test]
fn a_wikilink_inside_code_is_not_a_link() {
    // comrak never makes a link inside code, and this must agree with it. The
    // hand-rolled scanner that used to do this job reported three shell and
    // Lean snippets across the live corpus as links to memories nobody had
    // written — `[[-n "$target"]]` among them.
    let html = render_markdown("Run `rm [[-n \"$target\"]]` first.").unwrap();
    assert!(!html.contains("/m/"), "{html}");

    let md = "Text [[project_alpha]] then:\n\n```sh\nfoo [[la,lo,ts]] bar\n```\n";
    let html = render_markdown(md).unwrap();
    assert!(html.contains(r#"href="/m/project_alpha""#), "{html}");
    assert!(!html.contains("/m/la,lo,ts"), "{html}");
}

#[test]
fn a_section_head_is_the_word_however_it_is_punctuated() {
    // The corpus writes all of these, and every one of them is the memory
    // stating its reason. The check this replaced was a literal
    // `contains("**Why:**")`, which flagged nine memories that already said why
    // — asking the corpus to write worse prose so a checker could find it.
    for head in [
        "**Why:**",
        "**Why.**",
        "**Why (the nixos-repo caution):**",
        "**Why I was wrong.**",
        "**Why this matters:**",
        "**why:**",
    ] {
        assert!(
            has_section(&format!("{head} because of the thing."), "Why"),
            "{head}"
        );
    }
    for head in [
        "**How to apply:**",
        "**How to apply.**",
        "**How to apply, generally.**",
        "**How to apply the exception:**",
    ] {
        assert!(
            has_section(&format!("{head} do the thing."), "How to apply"),
            "{head}"
        );
    }
}

#[test]
fn a_section_head_has_to_be_bold_and_has_to_be_the_word() {
    // Plain prose is not a section, whatever it opens with.
    assert!(!has_section("Why this happened is a long story.", "Why"));
    // The word has to end there — `Whyever` is a different word, and a memory
    // that only mentions applying something has not said how.
    assert!(!has_section("**Whyever not:** go ahead.", "Why"));
    assert!(!has_section("**How to applyify:** no.", "How to apply"));
    // A heading is not the section either: the convention is a bold lead-in,
    // and accepting `## Why` would quietly permit two shapes.
    assert!(!has_section("## Why\n\nbecause.", "Why"));
}

#[test]
fn a_section_head_inside_a_fence_is_an_example_not_a_section() {
    // The exact failure the literal `contains()` could not see: a memory ABOUT
    // the convention, quoting it, counted as a memory following it.
    let quoting = "This rule wants:\n\n```md\n**Why:** the reason goes here\n```\n";
    assert!(!has_section(quoting, "Why"));
}

#[test]
fn a_description_reaches_the_page_as_rendered_markdown() {
    // A tenth of the corpus's descriptions hold a code span or a bold run, and
    // shown raw they read as punctuation. The value is serialised as HTML, so
    // the ranking and the linter go on seeing the words.
    let rendered = memview::store::render_inline("`code/kubes/dhall/` models the fleet");
    assert_eq!(rendered, "<code>code/kubes/dhall/</code> models the fleet");
    assert_eq!(
        memview::store::render_inline("**How to apply:** for new automation"),
        "<strong>How to apply:</strong> for new automation"
    );
}

#[test]
fn an_underscored_name_is_not_emphasis() {
    // The corpus is made of names like this, and a pattern-matching renderer
    // turns the middle of one into italics. CommonMark says intraword `_` is
    // not emphasis, which is the reason this is parsed rather than matched.
    assert_eq!(
        memview::store::render_inline("project_kubes_dhall_model and feedback_no_coauthor"),
        "project_kubes_dhall_model and feedback_no_coauthor"
    );
}

#[test]
fn a_marker_the_truncation_left_open_stays_literal() {
    // A snippet is a window cut out of a body, so it can end mid-construct.
    // CommonMark renders an unclosed marker as the text it is — the words are
    // never swallowed, which is the only thing that matters here.
    let rendered = memview::store::render_inline("…so `$files` arrived. `xinutec-infra/plan…");
    assert!(rendered.contains("<code>$files</code>"), "{rendered}");
    assert!(rendered.contains("`xinutec-infra/plan…"), "{rendered}");
}

#[test]
fn block_structure_contributes_words_and_no_markup() {
    // A snippet can start at a heading or inside a list. It must yield the text
    // of those blocks, never a stray `<li>` closing a tag nothing opened.
    let rendered = memview::store::render_inline("# Heading\n\n- one item\n- two");
    assert_eq!(rendered, "Heading one item two");
}

#[test]
fn html_in_a_description_is_shown_and_not_run() {
    // Bodies quote tags in prose. They render escaped — visible as text — which
    // is also what keeps the innerHTML binding on the other side harmless.
    let rendered = memview::store::render_inline("use <script>alert(1)</script> in prose");
    assert!(!rendered.contains("<script>"), "{rendered}");
    assert!(rendered.contains("&lt;script&gt;"), "{rendered}");
}

/// A demotion asks what the index still reaches WITHOUT the lines it drops, and
/// the fixture already has the shape that makes the answer non-obvious:
/// `project_alpha` and `reference_beta` link each other, and `feedback_gamma` is
/// linked only from `project_alpha`.
fn reaches_without(names: &[&str]) -> std::collections::BTreeSet<String> {
    let corpus = corpus();
    let index = corpus.index_md.clone().expect("fixture has an index");
    let dropping = names.iter().map(|n| (*n).to_string()).collect();
    memview::store::reachable_without(&corpus.docs, &index, &dropping)
}

#[test]
fn dropping_one_index_line_leaves_it_reachable_through_its_neighbour() {
    // The ordinary, safe case: beta is still linked from alpha, which is listed.
    let reached = reaches_without(&["reference_beta"]);
    assert!(reached.contains("reference_beta"), "{reached:?}");
    assert!(reached.contains("feedback_gamma"), "{reached:?}");
}

#[test]
fn two_memories_that_house_each_other_are_stranded_when_both_lines_go() {
    // ⚠ THE defect this exists for (#869). Asked one at a time, each of these is
    // housed by the other and looks safe to demote; asked together, nothing
    // reaches either. `memory-rank` summed 25 candidates as if independent and
    // offered exactly such a pair — the 2026-08-07 stranding, with a number on
    // it. A per-candidate check cannot see this, however carefully it is read.
    let alone = reaches_without(&["project_alpha"]);
    assert!(alone.contains("project_alpha"), "beta houses alpha alone");

    let both = reaches_without(&["project_alpha", "reference_beta"]);
    assert!(!both.contains("project_alpha"), "{both:?}");
    assert!(!both.contains("reference_beta"), "{both:?}");
}

#[test]
fn a_memory_with_its_own_index_line_survives_its_only_inbound_link_being_demoted() {
    // gamma's one inbound link is from alpha, and it is listed in its own right.
    // So demoting alpha and beta together does not touch it — the walk starts
    // from every line the index still carries, not from the demoted docs'
    // descendants. Wrong the other way round, this test asserted gamma WAS
    // stranded; the code was right and the expectation was not.
    let reached = reaches_without(&["project_alpha", "reference_beta"]);
    assert!(reached.contains("feedback_gamma"), "{reached:?}");
}
