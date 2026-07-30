//! Static analysis for the memory corpus.
//!
//!     cargo run --bin memory-lint [-- <corpus dir>]
//!
//! Defaults to the live corpus. Exits non-zero only on ERROR findings, so a
//! rule can be introduced as a warning, worked down to zero, and then promoted
//! in `lint.rs` — after which the corpus can never regress on it.
use anyhow::Result;
use memview::lint;
use memview::store::Corpus;

fn main() -> Result<()> {
    let dir = std::env::args().nth(1).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.claude/projects/-Users-pippijn-Code/memory")
    });
    let corpus = Corpus::load(&dir)?;
    let findings = lint::check(&corpus);
    let reasons = lint::rule_reasons();

    println!("{} memories in {dir}\n", corpus.docs.len());

    let mut current = "";
    for finding in &findings {
        if finding.rule != current {
            current = finding.rule;
            let (sev, why) = reasons
                .get(finding.rule)
                .copied()
                .unwrap_or((lint::Severity::Warning, ""));
            println!("\n{}  {}\n    {why}", sev.label(), finding.rule);
        }
        println!("    {:<58} {}", finding.memory, finding.detail);
    }

    println!("\nrelations in use:");
    for (relation, count) in lint::relation_usage(&corpus) {
        println!("    {relation:<14} {count}");
    }

    let tally = lint::tally(&findings);
    println!("\n{} findings", findings.len());
    for (rule, count) in &tally {
        let sev = reasons
            .get(rule)
            .map(|(s, _)| s.label())
            .unwrap_or("warning");
        println!("    {sev:<8} {rule:<22} {count}");
    }

    if lint::passed(&findings) {
        println!("\nno errors");
        Ok(())
    } else {
        std::process::exit(1);
    }
}
