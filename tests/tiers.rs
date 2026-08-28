//! What the tiering must get right about the root's two populations (#1210).

use memview::tiers::{
    Entry, Held, HeldEntry, Role, Thresholds, Tier, census, expired, median_entry_cost, propose,
};

/// Day 100 is "now" in every test here, so an age is `100 - created`.
const TODAY: i64 = 100;

fn entry(name: &str, created: Option<i64>, breadth: usize) -> Entry {
    Entry {
        name: name.to_string(),
        indexed: true,
        created,
        breadth,
        entry_cost: 40,
        ..Entry::default()
    }
}

/// The ordinary demotable fixture: thin, housed, and judged a POINTER — which
/// is the only role a demotion may be proposed for. Tests that care about the
/// other roles override it.
fn housed(name: &str, created: Option<i64>, breadth: usize) -> Entry {
    Entry {
        homes: vec!["some_hub".to_string()],
        role: Some(Role::Pointer),
        ..entry(name, created, breadth)
    }
}

fn no_strands(_: &[Entry]) -> Vec<String> {
    Vec::new()
}

fn reasons(held: &[HeldEntry]) -> Vec<(&str, Held)> {
    held.iter()
        .map(|h| (h.entry.name.as_str(), h.why))
        .collect()
}

#[test]
fn a_new_entry_holds_a_lease_however_little_it_is_read() {
    let at = Thresholds::default();
    // Written yesterday, opened by nobody. It is live, not proven, and that is
    // the whole point of the tier — a session writing entries constantly is not
    // a defect to suppress.
    let fresh = entry("project_started_today", Some(99), 0);
    assert_eq!(fresh.tier(TODAY, &at), Tier::Lease);
}

#[test]
fn breadth_and_not_age_decides_what_the_lease_becomes() {
    let at = Thresholds::default();
    let old = 100 - 60;
    assert_eq!(entry("wide", Some(old), 9).tier(TODAY, &at), Tier::Tenure);
    assert_eq!(entry("some", Some(old), 4).tier(TODAY, &at), Tier::Middle);
    assert_eq!(entry("none", Some(old), 1).tier(TODAY, &at), Tier::Thin);
}

/// ⚠ The failure the model names: a lease that becomes tenure because nobody
/// looked. Age alone must never promote.
#[test]
fn sitting_in_the_root_for_a_year_earns_nothing() {
    let at = Thresholds::default();
    let ancient = entry("indexed_since_forever", Some(100 - 365), 1);
    assert_eq!(ancient.tier(TODAY, &at), Tier::Thin);
}

#[test]
fn an_undated_entry_is_its_own_tier_and_not_defaulted_either_way() {
    let at = Thresholds::default();
    let unknown = entry("no_transcript_names_it", None, 0);
    assert_eq!(unknown.tier(TODAY, &at), Tier::Undated);
    // Defaulting it to old would make a missing field into a demotion argument.
    assert_ne!(unknown.tier(TODAY, &at), Tier::Thin);
}

#[test]
fn the_census_weighs_only_what_the_index_carries() {
    let at = Thresholds::default();
    let mut outside = entry("read_widely_but_not_listed", Some(0), 9);
    outside.indexed = false;
    let rows = census(&[entry("listed", Some(0), 9), outside], TODAY, &at);
    assert_eq!(rows["TENURE"].entries, 1);
    assert_eq!(rows["TENURE"].bytes, 40);
}

#[test]
fn a_lease_expiry_is_the_crossing_and_not_the_whole_backlog() {
    let at = Thresholds::default();
    let entries = vec![
        entry("crossed_yesterday", Some(100 - 15), 1),
        entry("crossed_last_month", Some(100 - 60), 1),
        entry("still_leased", Some(100 - 3), 1),
    ];
    let due = expired(&entries, TODAY, &at, 7);
    assert_eq!(
        due.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["crossed_yesterday"]
    );
}

#[test]
fn the_admission_budget_is_the_median_line_the_root_already_carries() {
    let entries = vec![
        Entry {
            entry_cost: 10,
            ..entry("a", Some(0), 0)
        },
        Entry {
            entry_cost: 30,
            ..entry("b", Some(0), 0)
        },
        Entry {
            entry_cost: 50,
            ..entry("c", Some(0), 0)
        },
    ];
    assert_eq!(median_entry_cost(&entries), 30);
}

/// The ratchet the model names: the root grows by judgement and shrinks by
/// measurement. One operation, both halves, or the ceiling is breached.
#[test]
fn a_trade_admits_and_demotes_in_one_operation() {
    let at = Thresholds::default();
    let mut widely_read = entry("project_reached_without_the_index", Some(0), 10);
    widely_read.indexed = false;
    widely_read.entry_cost = 0;
    let entries = vec![housed("thin_and_housed", Some(0), 1), widely_read];

    let trade = propose(&entries, TODAY, &at, 0, &no_strands);
    assert_eq!(
        trade
            .demote
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        ["thin_and_housed"]
    );
    assert_eq!(
        trade
            .admit
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        ["project_reached_without_the_index"]
    );
    assert_eq!(trade.recovered, 40);
    assert_eq!(trade.affordable, 1);
    assert_eq!(trade.net(), 0, "one line out paid for one line in");
}

/// ⚠ With no headroom and nothing to demote, an admission is SHOWN and not
/// affordable. Hiding it would report "nothing has earned a slot" when the
/// truth is "something has and there is no room" — the finding that argues for
/// a demotion pass. Spending the bytes anyway breaches the ceiling in the diff.
#[test]
fn an_admission_with_nothing_to_pay_for_it_is_shown_but_not_bought() {
    let at = Thresholds::default();
    let mut widely_read = entry("wanted", Some(0), 10);
    widely_read.indexed = false;
    let trade = propose(&[widely_read], TODAY, &at, 0, &no_strands);
    assert_eq!(trade.admit.len(), 1, "the evidence is the point");
    assert_eq!(trade.affordable, 0, "but nothing pays for it");
    assert_eq!(trade.net(), 0);
}

/// A demotion needs somewhere to land. This is the step the 2026-08-07 pass
/// skipped, and skipping it stranded memories that still existed.
#[test]
fn an_unhoused_entry_is_never_offered_for_demotion() {
    let at = Thresholds::default();
    let orphan = entry("thin_but_nothing_links_it", Some(0), 0);
    assert!(orphan.homes.is_empty());
    let trade = propose(&[orphan], TODAY, &at, 0, &no_strands);
    assert!(trade.demote.is_empty());
}

/// ⚠ Homes are found against the index as it stands, so a pair that links only
/// each other reads as housed until both lines go together.
#[test]
fn a_pair_that_houses_only_each_other_is_dropped_from_the_set() {
    let at = Thresholds::default();
    let entries = vec![
        Entry {
            homes: vec!["b".to_string()],
            ..housed("a", Some(0), 0)
        },
        Entry {
            homes: vec!["a".to_string()],
            ..housed("b", Some(0), 0)
        },
    ];
    let both_strand = |set: &[Entry]| set.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
    let trade = propose(&entries, TODAY, &at, 0, &both_strand);
    assert!(trade.demote.is_empty());
    assert_eq!(trade.recovered, 0);
}

/// ⚠ #884's freeze is on the SPLIT, and it runs until 2026-09-11. A frozen
/// entry still qualifies on the evidence; acting on it perturbs the series.
#[test]
fn a_frozen_entry_is_held_apart_rather_than_dropped_or_demoted() {
    let at = Thresholds::default();
    let frozen = Entry {
        frozen: true,
        ..housed("in_the_control_arm", Some(0), 1)
    };
    let trade = propose(&[frozen], TODAY, &at, 0, &no_strands);
    assert!(trade.demote.is_empty(), "the freeze forbids acting on it");
    assert_eq!(
        reasons(&trade.held),
        [("in_the_control_arm", Held::Frozen)],
        "but it must still be visible, or the proposal is silently short"
    );
    assert_eq!(trade.recovered, 0, "held bytes are not recovered bytes");
}

/// ⚠ #1214: unprovable opens are shown and never scored. A memory whose only
/// evidence is a shell read after `&&` must not tier as if it were proven.
#[test]
fn unprovable_opens_do_not_buy_tenure() {
    let at = Thresholds::default();
    let unproven = Entry {
        maybe_breadth: 20,
        ..entry("read_only_after_an_and", Some(0), 0)
    };
    assert_eq!(unproven.tier(TODAY, &at), Tier::Thin);
}

/// ⚠ **A tripwire's low open count is what SUCCESS looks like** — it fires from
/// the index line and the file is never opened. `Tier::Thin` is breadth-derived,
/// so a demote filter that reads only the tier selects exactly the entries doing
/// their job best. `memory-rank` held these back by name prefix; #884 showed the
/// prefix is the wrong classifier, and dropping it without a replacement left
/// this half unguarded (#1234).
#[test]
fn a_tripwire_is_never_offered_for_demotion() {
    let at = Thresholds::default();
    let trip = Entry {
        role: Some(Role::Tripwire),
        ..housed("feedback_fires_from_the_line", Some(0), 0)
    };
    let trade = propose(&[trip], TODAY, &at, 0, &no_strands);
    assert!(trade.demote.is_empty());
    assert_eq!(
        reasons(&trade.held),
        [("feedback_fires_from_the_line", Held::Tripwire)]
    );
}

/// ⚠ **Unjudged is not "pointer".** #884 has classified some of the corpus, not
/// all of it. Treating an absent judgement as a demotable pointer is a check
/// that passes for the wrong reason, and it fails toward deleting the only place
/// a rule fires.
#[test]
fn an_entry_with_no_role_judgement_is_held_rather_than_assumed_a_pointer() {
    let at = Thresholds::default();
    let unknown = Entry {
        role: None,
        ..housed("nobody_has_judged_this", Some(0), 0)
    };
    let trade = propose(&[unknown], TODAY, &at, 0, &no_strands);
    assert!(trade.demote.is_empty());
    assert_eq!(
        reasons(&trade.held),
        [("nobody_has_judged_this", Held::Unjudged)]
    );
}

/// The demotable case, so the guard above is not simply refusing everything.
#[test]
fn a_thin_housed_pointer_is_demotable() {
    let at = Thresholds::default();
    let ptr = Entry {
        role: Some(Role::Pointer),
        ..housed("project_findable_when_wanted", Some(0), 0)
    };
    let trade = propose(&[ptr], TODAY, &at, 0, &no_strands);
    assert_eq!(
        trade
            .demote
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        ["project_findable_when_wanted"]
    );
    assert_eq!(trade.recovered, 40);
}

/// ⚠ **Role before freeze, and the order is the point.** The freeze lifts on
/// 2026-09-11; being a tripwire does not. Reporting the freeze as the reason
/// would make this read as demotable the day after the harvest — which is the
/// failure #1234 describes, moved by a fortnight rather than fixed.
#[test]
fn a_frozen_tripwire_is_held_for_its_role_not_for_the_freeze() {
    let at = Thresholds::default();
    let both = Entry {
        role: Some(Role::Tripwire),
        frozen: true,
        ..housed("feedback_in_the_control_arm_and_a_tripwire", Some(0), 0)
    };
    let trade = propose(&[both], TODAY, &at, 0, &no_strands);
    assert_eq!(
        reasons(&trade.held),
        [("feedback_in_the_control_arm_and_a_tripwire", Held::Tripwire)]
    );
}

/// ⚠ **A thin verdict that DEPENDS on discarded evidence is not a verdict.**
/// Unprovable opens — a shell read after `&&`, or inside a script with one exit
/// status — are collected and never scored (#1214), which is right: counting
/// them overstates the record. But when counting them would lift a memory out of
/// THIN, the tier is an artefact of what was thrown away, and proposing a
/// demotion on it is acting on evidence known to be incomplete.
#[test]
fn a_thin_verdict_that_turns_on_unprovable_opens_is_held() {
    let at = Thresholds::default();
    // Two proven opens is thin; five more that cannot be proved would not be.
    let shaky = Entry {
        maybe_breadth: 5,
        role: Some(Role::Pointer),
        ..housed("reference_read_mostly_by_shell", Some(0), 2)
    };
    assert_eq!(shaky.tier(TODAY, &at), Tier::Thin);
    let trade = propose(&[shaky], TODAY, &at, 0, &no_strands);
    assert!(trade.demote.is_empty());
    assert_eq!(
        reasons(&trade.held),
        [("reference_read_mostly_by_shell", Held::Unproven)]
    );
}

/// And a memory that is thin whichever way the unprovable opens fall is simply
/// thin — otherwise the guard refuses everything and says nothing.
#[test]
fn unprovable_opens_that_could_not_change_the_tier_do_not_hold_it() {
    let at = Thresholds::default();
    let clear = Entry {
        maybe_breadth: 0,
        role: Some(Role::Pointer),
        ..housed("project_quiet_either_way", Some(0), 1)
    };
    let trade = propose(&[clear], TODAY, &at, 0, &no_strands);
    assert_eq!(
        trade
            .demote
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        ["project_quiet_either_way"]
    );
}

/// ⚠ **The undercount runs the other way too.** A memory just under the tenure
/// bar whose unprovable opens would carry it over is excluded from ADMIT in
/// silence. Counting them would invent a discount factor `docs/memory.md` warns
/// against; saying nothing hides that the bar was decided by discarded evidence.
/// So they are counted apart, and named.
#[test]
fn admission_near_misses_that_turn_on_unprovable_opens_are_counted_apart() {
    let at = Thresholds::default();
    let mut nearly = entry("project_reached_by_shell_heavy_sessions", Some(0), 5);
    nearly.indexed = false;
    nearly.maybe_breadth = 4;
    let trade = propose(&[nearly], TODAY, &at, 0, &no_strands);
    assert!(trade.admit.is_empty(), "5 proven is under the bar of 6");
    assert_eq!(trade.unproven_admissions, 1);
}
