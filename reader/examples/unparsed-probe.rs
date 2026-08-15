//! What is actually in the commands the shell reader cannot read?
//!
//! A probe, not a check — kept beside `roundtrip-probe.rs` and for the same
//! reason: it answers a design question with a number.
//!
//! **The question** is memview#820. The reader should be *stricter* than the
//! interpreters it models, refusing constructs that only ever appear by accident
//! — and a refusal on purpose is a different fact from a hole not yet filled, so
//! the two should not share one bucket. That policy is sound on its own. What it
//! lacked was a construct worth refusing: backticks were the motivating example
//! and measurement killed them, 4,175 calls carrying one and exactly **one** use
//! of a backtick substitution in an operand position across the whole corpus.
//!
//! So before building a taxonomy, this asks whether it would have more than one
//! member — because "358 unparsed" is a number nobody can act on and "N of them
//! are X" is a decision.
//!
//!     cargo run --release -p reader --example unparsed-probe -- <corpus.jsonl>
//!
//! **The answer, over 121,898 distinct commands: no.** Of 358 unreadable:
//!
//!     124  (34.6%)  case … esac
//!      22  ( 6.1%)  process substitution
//!      19  ( 5.3%)  a backtick anywhere, data included
//!      17  ( 4.7%)  array literal
//!      15  ( 4.2%)  function definition
//!      13  ( 3.6%)  until
//!     153  (42.7%)  none of those — overwhelmingly subshell grouping,
//!                   `(cd $a && git add -A …)`, `(nc -z $ip $port)`
//!
//! Every one of those is something a person writes ON PURPOSE. None is a
//! construct that only ever appears by accident, which is the thing the policy
//! wanted to refuse. So the refused bucket would hold backticks and nothing else,
//! and a taxonomy with one member is not worth the two mechanisms it costs.
//! #820 is closed on that; the gaps it exposed are ordinary gaps.
//!
//! **And then the top two buckets were closed, which is what the probe was for**
//! (memview#901, 2026-08-15). Over 127,342 distinct commands, unparsed went
//! **366 → 113** and the corpus went from 99.7% readable to 99.9%:
//!
//!     case … esac        126 → 7
//!     unbalanced )       158 → 38
//!     simple commands    535,634 → 538,983
//!
//! ⚠ **It was one shared cause, and the `case` rule alone did a third of it.**
//! Both compounds were reachable only at the START of a command, and `do` is an
//! ordinary word to this grammar — so `do case "$f" in …` and `do (cd $d && git
//! commit …)` both put the keyword and the compound in one command, and neither
//! was ever tried. Letting the word run hold them took a further 137.
//!
//! ⚠ **The seven `case`s that remain are not `case` failures.** Six choke on
//! something else in the same command — `done < <(…)`, quoting inside a `--jq`
//! argument — and the seventh is a `case` inside `$( … )`, where `subst_body`
//! scans to the first unbalanced `)` and so ends at the first arm's pattern. One
//! command in the corpus; the fix is a substitution body that knows about arms,
//! which costs more than it buys.
//!
//! ⚠ **A parse rate is not a coverage figure, and this is where that bites.**
//! 99.9% of commands parse, and a command that parses can still hide the ones
//! inside it: a `$( … )` in double quotes is matched whole by the atomic
//! `dquoted` rule and never walked, which is 8,300 distinct commands — 6.5%,
//! seventy times this whole table — reporting nothing while looking clean.
//! memview#918.
//!
//! ⚠ **The count moves when the parser changes**, so it is not a ratchet and is
//! not asserted anywhere. memview#835 took it from 416 to 358 by itself.

use std::collections::{BTreeMap, BTreeSet};

use reader::shell::parse;

/// The shell constructs a command uses, named.
///
/// ⚠ **Bucketing by the parser's own reason does not work**, which is what this
/// probe did first: the reason quotes the offending source, so 358 failures gave
/// 358 buckets and the biggest held eleven. The question is about CONSTRUCTS —
/// which ones the reader cannot read, and whether any is the sort that only ever
/// appears by accident — so the text is asked directly.
///
/// Deliberately crude. A command is counted under every construct it contains,
/// so the columns do not sum to the total; a polling loop is a `for` and a `case`
/// at once, and pretending otherwise would hide whichever came second.
fn constructs(cmd: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    let has = |needle: &str| cmd.contains(needle);
    if has("esac") || has("case ") {
        found.push("case … esac");
    }
    if has("function ") || cmd.contains("() {") || cmd.contains("()\n{") {
        found.push("function definition");
    }
    if has("<(") || has(">(") {
        found.push("process substitution");
    }
    if has("[[") {
        found.push("[[ … ]]");
    }
    if has("until ") {
        found.push("until");
    }
    if has("select ") {
        found.push("select");
    }
    if has("=(") || has("declare -a") || has("declare -A") {
        found.push("array");
    }
    if has("trap ") {
        found.push("trap");
    }
    if has("`") {
        found.push("backtick (data or otherwise)");
    }
    if has("coproc") {
        found.push("coproc");
    }
    if found.is_empty() {
        found.push("(none of the above)");
    }
    found
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: unparsed-probe <corpus.jsonl>"))?;
    let text = std::fs::read_to_string(&path)?;

    let mut seen = BTreeSet::new();
    let mut distinct = 0usize;
    let mut unparsed = 0usize;
    let mut reasons: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    let mut residue: Vec<String> = Vec::new();

    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        if !seen.insert(cmd.to_string()) {
            continue;
        }
        distinct += 1;
        let Err(reason) = parse(cmd) else {
            continue;
        };
        unparsed += 1;
        let named = constructs(cmd);
        // The residue is the interesting part, and naming it needs the parser's
        // own complaint — which is useless in bulk (it quotes the source, so
        // every failure is its own bucket) and exactly right for a handful.
        if named == ["(none of the above)"] && residue.len() < 12 {
            residue.push(
                reason
                    .chars()
                    .take(90)
                    .collect::<String>()
                    .replace('\n', " ⏎ "),
            );
        }
        for name in named {
            let entry = reasons.entry(name.to_string()).or_default();
            entry.0 += 1;
            if entry.1.len() < 2 {
                entry.1.push(
                    cmd.chars()
                        .take(100)
                        .collect::<String>()
                        .replace('\n', " ⏎ "),
                );
            }
        }
    }

    println!("distinct commands   {distinct}");
    println!("unparsed            {unparsed}");
    println!();

    let mut ranked: Vec<_> = reasons.into_iter().collect();
    ranked.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));
    for (reason, (count, examples)) in ranked {
        let share = 100.0 * count as f64 / unparsed.max(1) as f64;
        println!("{count:6}  ({share:4.1}%)  {reason}");
        for example in examples {
            println!("          {example}");
        }
    }
    if !residue.is_empty() {
        println!();
        println!("what the residue actually chokes on:");
        for reason in &residue {
            println!("          {reason}");
        }
    }
    Ok(())
}
