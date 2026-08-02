//! The document-graph rules: whether the corpus is navigable at all.

use memview::lint::{Severity, check};
use memview::store::Corpus;

/// A corpus of an index plus `(name, body)` memories.
fn corpus(dir: &std::path::Path, index: &str, docs: &[(&str, &str)]) -> Corpus {
    std::fs::write(dir.join("MEMORY.md"), index).expect("write index");
    for (name, body) in docs {
        std::fs::write(
            dir.join(format!("{name}.md")),
            format!(
                "---\nname: {name}\ndescription: d\nmetadata:\n  type: project\n---\n\n{body}\n"
            ),
        )
        .expect("write memory");
    }
    Corpus::load(dir).expect("loads")
}

fn findings(corpus: &Corpus, rule: &str) -> Vec<String> {
    check(corpus, None)
        .into_iter()
        .filter(|f| f.rule == rule && f.severity == Severity::Error)
        .map(|f| f.memory)
        .collect()
}

#[test]
fn a_memory_several_hops_from_the_index_is_reachable() {
    // Pippijn, 2026-08-02: "MEMORY.md doesn't need to index everything. things
    // have to be reachable, but don't need to all be in MEMORY.md". The rule
    // used to demand an index line per memory and failed the gate on a corpus
    // that was perfectly navigable — three memories had just been consolidated
    // under one entry that links them all.
    let dir = tempfile::tempdir().expect("tempdir");
    let found = corpus(
        dir.path(),
        "# Memory index\n- [hub](project_hub.md)\n",
        &[
            ("project_hub", "see [[project_middle]]"),
            ("project_middle", "and [[project_leaf]]"),
            ("project_leaf", "the end, back to [[project_hub]]"),
        ],
    );
    assert!(findings(&found, "unreachable").is_empty());
}

#[test]
fn a_memory_nothing_links_is_an_error() {
    // The failure the rule is actually for: a memory that exists and that no
    // reader can arrive at.
    let dir = tempfile::tempdir().expect("tempdir");
    let found = corpus(
        dir.path(),
        "# Memory index\n- [hub](project_hub.md)\n",
        &[
            ("project_hub", "see [[project_middle]]"),
            ("project_middle", "nothing further"),
            (
                "project_island",
                "linked from nowhere, links [[project_hub]]",
            ),
        ],
    );
    // It links out, so it is not `stranded` — and it still cannot be found.
    assert_eq!(findings(&found, "unreachable"), ["project_island"]);
}
