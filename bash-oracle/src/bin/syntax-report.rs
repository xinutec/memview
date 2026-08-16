//! What the syntax tree reads, what it refuses, and whether both gates hold.
//!
//!     cargo run --release -p bash-oracle --bin syntax-report -- <corpus.jsonl> [--oracle] [--why SUBSTRING]
//!
//! ⚠ **Pass `~/.claude/corpus/union.jsonl`.** The live transcripts shrink, so a
//! coverage figure measured against them rises when commands leave and two runs
//! either side of a change are not comparable. The union is the fixed
//! denominator. See `docs/execution-model.md`.
//!
//! **Two rates, reported apart — per command and per byte.** They diverge, and
//! the direction says which work is left: a parser reading most commands but few
//! bytes is failing on the long ones, where the payloads are. The design's other
//! two numbers, per node and depth, need embedding to mean anything and are not
//! reported until there is a second layer to be at.
//!
//! The refusal ranking is the point of the whole report. It is the work queue,
//! and it is ordered by the corpus rather than by what seems interesting.

use std::collections::{BTreeMap, BTreeSet};

use bash_oracle as oracle;
use reader::syntax::{self, Outcome, Reason};

#[derive(Default)]
struct Tally {
    commands: usize,
    bytes: usize,
}

impl Tally {
    fn add(&mut self, text: &str) {
        self.commands += 1;
        self.bytes += text.len();
    }
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: syntax-report <corpus.jsonl> [--oracle]"))?;
    // `--why <substring>` prints every command behind a matching refusal, the
    // same affordance `shell-files` has and for the same reason: a ranking says
    // what to build next, and the commands say whether the refusal is even
    // right. Fourteen "unterminated quote" refusals are either fourteen odd
    // commands or one parser bug, and only this tells them apart.
    let mut run_oracle = false;
    let mut why: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--oracle" => run_oracle = true,
            "--why" => why = args.next(),
            _ => {}
        }
    }
    let text = std::fs::read_to_string(&path)?;

    let mut seen = BTreeSet::new();
    let (mut total, mut read, mut refused) = (Tally::default(), Tally::default(), Tally::default());
    let mut reasons: BTreeMap<Reason, (Tally, Vec<String>)> = BTreeMap::new();
    let mut broken: BTreeMap<&'static str, (usize, Vec<String>)> = BTreeMap::new();
    // Kept for the oracle, which is worth one process per 500 rather than one
    // per command.
    let mut accepted: Vec<(String, syntax::Script)> = Vec::new();
    // Refusals that assert the TEXT is invalid, which bash can adjudicate.
    let mut claimed_invalid: Vec<String> = Vec::new();
    // The FULL set of constructs each refused command needs, from the survey.
    // The ranking above cannot answer "what would building X unlock", because a
    // command is counted under whichever construct the scan met first.
    let mut blockers: Vec<BTreeSet<Reason>> = Vec::new();
    // The survey is a second scanner and could drift from the parser. Pinned
    // rather than trusted: whatever the parser refused must be in the set.
    let mut survey_disagrees = 0usize;

    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(command) = row["cmd"].as_str() else {
            continue;
        };
        if !seen.insert(command.to_string()) {
            continue;
        }
        total.add(command);

        match syntax::check(command) {
            Outcome::Refused(refusal) => {
                refused.add(command);
                // ⚠ Every reason that asserts the TEXT is broken, not merely
                // unmodelled. `EmptyOperand` belongs here too: bash refuses
                // `a &&` and `cat >` exactly as we do, so the claim is
                // falsifiable and has to be falsified.
                if matches!(
                    refusal.reason,
                    Reason::UnterminatedQuote | Reason::DanglingEscape | Reason::EmptyOperand
                ) {
                    claimed_invalid.push(command.to_string());
                }
                if let Some(wanted) = &why
                    && refusal.reason.label().contains(wanted.as_str())
                {
                    println!("--- refused: {}\n{command}", refusal.reason.label());
                }
                let entry = reasons.entry(refusal.reason).or_default();
                entry.0.add(command);
                keep_example(&mut entry.1, command);

                let needs = syntax::survey(command);
                if !needs.contains(&refusal.reason) {
                    survey_disagrees += 1;
                    if survey_disagrees <= 6 {
                        println!(
                            "  ⚠ survey drift: parser says {:?}, survey says {:?}\n      {}",
                            refusal.reason,
                            needs,
                            command
                                .chars()
                                .take(110)
                                .collect::<String>()
                                .replace('\n', "⏎")
                        );
                    }
                }
                blockers.push(needs);
            }
            outcome => {
                read.add(command);
                let needs = syntax::survey(command);
                if !needs.is_empty() {
                    survey_disagrees += 1;
                    if survey_disagrees <= 6 {
                        println!(
                            "  ⚠ survey drift: parser ACCEPTED, survey says {:?}\n      {}",
                            needs,
                            command
                                .chars()
                                .take(110)
                                .collect::<String>()
                                .replace('\n', "⏎")
                        );
                    }
                }
                if outcome.holds() {
                    if run_oracle && let Ok(tree) = syntax::parse(command) {
                        accepted.push((command.to_string(), tree));
                    }
                } else {
                    let entry = broken.entry(outcome.label()).or_default();
                    entry.0 += 1;
                    keep_example(&mut entry.1, command);
                }
            }
        }
    }

    println!("distinct commands   {}", total.commands);
    println!("corpus bytes        {}", total.bytes);
    println!();
    println!(
        "read                {:>7}  ({:.2}% of commands, {:.2}% of bytes)",
        read.commands,
        percent(read.commands, total.commands),
        percent(read.bytes, total.bytes),
    );
    println!(
        "refused             {:>7}  ({:.2}% of commands, {:.2}% of bytes)",
        refused.commands,
        percent(refused.commands, total.commands),
        percent(refused.bytes, total.bytes),
    );

    println!("\nthe work queue — refusals, biggest first:");
    let mut ranked: Vec<_> = reasons.iter().collect();
    ranked.sort_by_key(|(_, (tally, _))| std::cmp::Reverse(tally.commands));
    for (reason, (tally, examples)) in ranked {
        println!(
            "  {:>7}  {:>5.2}%  {}",
            tally.commands,
            percent(tally.commands, total.commands),
            reason.label(),
        );
        for example in examples {
            println!("           {example}");
        }
    }

    // ---- what would actually unlock a command ----
    //
    // ⚠ The ranking above answers "what stopped us first", which is not the same
    // question and cannot be added up. A command needs EVERY construct in it
    // before it can be read, so the unlock figures below are the ones to plan
    // from.
    println!("\nsurvey — the full set of constructs each refused command needs");
    if survey_disagrees > 0 {
        println!(
            "  ⚠ {survey_disagrees} commands where the survey and the parser disagree — \
             the survey is a second scanner and has drifted; the figures below are unsound"
        );
    } else {
        println!(
            "  the survey agrees with the parser on all {} commands",
            total.commands
        );
    }

    let mut alone: BTreeMap<Reason, usize> = BTreeMap::new();
    let mut widths: BTreeMap<usize, usize> = BTreeMap::new();
    for needs in &blockers {
        *widths.entry(needs.len()).or_default() += 1;
        if let [only] = needs.iter().copied().collect::<Vec<_>>()[..] {
            *alone.entry(only).or_default() += 1;
        }
    }

    println!("\n  how many constructs a refused command is missing:");
    for (count, commands) in &widths {
        println!(
            "    {commands:>7}  need {count} construct{}",
            if *count == 1 { "" } else { "s" }
        );
    }

    println!("\n  build ONE construct, and this many commands become readable:");
    let mut ranked_alone: Vec<_> = alone.iter().collect();
    ranked_alone.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
    for (reason, count) in ranked_alone {
        println!(
            "    {count:>7}  {:>5.2}%  {}",
            percent(*count, total.commands),
            reason.label()
        );
    }

    // Greedy: repeatedly take the construct that unlocks the most commands
    // given everything chosen so far. Greedy is not optimal in general, but the
    // question here is what to build NEXT, which is exactly one greedy step.
    println!("\n  greedy build order — cumulative commands readable:");
    let mut chosen: BTreeSet<Reason> = BTreeSet::new();
    let mut cumulative = read.commands;
    for _ in 0..6 {
        let mut best: Option<(Reason, usize)> = None;
        for candidate in reasons.keys().copied() {
            if chosen.contains(&candidate) {
                continue;
            }
            let mut with = chosen.clone();
            with.insert(candidate);
            let unlocked = blockers
                .iter()
                .filter(|needs| needs.is_subset(&with))
                .count();
            if best.is_none_or(|(_, most)| unlocked > most) {
                best = Some((candidate, unlocked));
            }
        }
        let Some((next, unlocked)) = best else { break };
        chosen.insert(next);
        let total_readable = read.commands + unlocked;
        println!(
            "    + {:<28} {:>7}  ({:.2}% of commands, +{} over the step before)",
            next.label(),
            total_readable,
            percent(total_readable, total.commands),
            total_readable - cumulative,
        );
        cumulative = total_readable;
    }

    println!("\ngate 1 — the round-trip law, over what was read:");
    let failures: usize = broken.values().map(|(count, _)| count).sum();
    println!(
        "  {:>7}  holds  ({:.2}% of what was read)",
        read.commands - failures,
        percent(read.commands - failures, read.commands),
    );
    for (label, (count, examples)) in &broken {
        println!("  {count:>7}  {label}");
        for example in examples {
            println!("           {example}");
        }
    }

    if run_oracle {
        // ⚠ Checked before the tree comparison, because it asks a different and
        // more basic question: are the refusals that call the input broken
        // actually right? A wrong one here is a command being dropped from the
        // denominator for a defect of ours.
        println!("\nrefusals that claim the input is not shell, put to `bash -n`:");
        let mut wrong = 0usize;
        for command in &claimed_invalid {
            if !oracle::bash_also_refuses(command)? {
                wrong += 1;
                if wrong <= 3 {
                    println!("  ⚠ bash ACCEPTS this — our refusal is a bug:");
                    println!(
                        "      {}",
                        command
                            .chars()
                            .take(120)
                            .collect::<String>()
                            .replace('\n', "⏎")
                    );
                }
            }
        }
        println!("  {:>7}  checked, {wrong} wrong", claimed_invalid.len());

        println!("\ngate 2 — bash's own printer, over what the law held for:");
        // ⚠ The ORIGINAL command text, not our print of it. Feeding bash our
        // own output can only confirm it agrees with our canonical form; the
        // misparse to catch is of the text the corpus actually holds.
        let texts: Vec<String> = accepted.iter().map(|(text, _)| text.clone()).collect();
        let verdicts = oracle::compare(&texts)?;
        let mut grouped: BTreeMap<&'static str, (usize, Vec<String>)> = BTreeMap::new();
        for ((command, _), verdict) in accepted.iter().zip(&verdicts) {
            let entry = grouped.entry(verdict.label()).or_default();
            entry.0 += 1;
            if !verdict.agrees() && entry.1.len() < 3 {
                entry.1.push(command.to_string());
            }
        }
        for (label, (count, examples)) in &grouped {
            println!("  {count:>7}  {label}");
            for example in examples {
                // ⚠ **Whole, not truncated.** A gate-2 disagreement is rare and
                // is the one finding worth acting on immediately; a 90-column
                // sample of it is not enough to find the command again, and
                // twice cost a wrong one being investigated instead.
                println!("           ---\n{example}\n           ---");
            }
        }
    }
    Ok(())
}

fn keep_example(examples: &mut Vec<String>, command: &str) {
    if examples.len() < 3 {
        examples.push(
            command
                .chars()
                .take(90)
                .collect::<String>()
                .replace('\n', "⏎"),
        );
    }
}

fn percent(part: usize, whole: usize) -> f64 {
    100.0 * part as f64 / whole.max(1) as f64
}
