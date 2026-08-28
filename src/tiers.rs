//! The root's two populations, and the one operation that trades between them.
//!
//! `MEMORY.md` holds two kinds of entry that earn their slot differently
//! (`docs/memory.md`): **recent** work, which belongs there because it is live
//! and is supposed to turn over, and **consolidated** rules, which belong there
//! because by the time they matter nobody knows to go looking. Conflating them
//! is why cuts keep going wrong — asked to make room, a session sees one topic
//! and evicts precisely the entries whose value is that they fire elsewhere.
//!
//! ⚠ **Breadth is the factor a session cannot observe about itself.** How often
//! a memory is opened cannot separate "forty reads by one session on one
//! afternoon" (a topic being worked) from "a few reads each by many sessions" (a
//! rule that has consolidated). How MANY distinct agents opened it can.
//!
//! ⚠ **Set cardinalities, never raw counts.** Breadth and days-live survive the
//! duplication trap in the transcripts — the CLI rewrites earlier stretches into
//! the same file, so a fifth of the corpus is second copies
//! (`reference_claude_transcript_rewrites_history`). A count doubles; the number
//! of distinct agents that opened something does not.

use std::collections::BTreeMap;

/// What an index line is for, from `memory-roles.json` — re-exported from the
/// study so both read the same labels.
pub use crate::study::Role;

/// Which population an entry belongs to, and therefore what holds it in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// New, live, and exempt — but only for a while. The failure this tier
    /// exists to make visible is a lease quietly becoming tenure.
    Lease,
    /// Past its lease and consulted widely: it fires in situations other than
    /// the one that wrote it, which is the qualification for a root slot.
    Tenure,
    /// Past its lease, consulted by some but not many. Neither earned nor
    /// expired — reported so the middle is not silently read as either.
    Middle,
    /// Past its lease and consulted by almost nobody.
    Thin,
    /// No creation date, so its lease cannot be judged.
    ///
    /// ⚠ **Kept as its own tier rather than defaulted into one.** Defaulting to
    /// old makes an undated entry a demotion candidate on the strength of a
    /// missing field; defaulting to new exempts it forever. Both read as an
    /// answer. This reads as the gap it is — a DETECTION gap, since the oldest
    /// surviving transcript predates the earliest recovered creation date, so
    /// nothing has been lost to pruning.
    Undated,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Tier::Lease => "LEASE",
            Tier::Tenure => "TENURE",
            Tier::Middle => "MIDDLE",
            Tier::Thin => "THIN",
            Tier::Undated => "UNDATED",
        }
    }
}

/// Where the two cuts fall. Held together so a report states them once and
/// every figure in it is anchored to the same pair.
#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    /// Days an entry is exempt from expiry, counted from when it was written.
    pub lease_days: i64,
    /// Distinct agents that must have opened it for tenure.
    pub tenure_breadth: usize,
    /// At or below this, the entry is thin.
    pub thin_breadth: usize,
}

impl Default for Thresholds {
    /// The values the corpus was measured at on 2026-08-27 (#1210). A fortnight
    /// matches the ranking half-life; six agents is where the outside-the-index
    /// population thins out.
    fn default() -> Self {
        Thresholds {
            lease_days: 14,
            tenure_breadth: 6,
            thin_breadth: 2,
        }
    }
}

/// One memory, as the tiering sees it.
#[derive(Debug, Clone, Default)]
pub struct Entry {
    pub name: String,
    /// Whether `MEMORY.md` carries a line for it.
    pub indexed: bool,
    /// Day it was first written, as [`crate::agents::day_number`] counts them.
    pub created: Option<i64>,
    /// Distinct agents with a PROVEN open.
    pub breadth: usize,
    /// Distinct agents whose only evidence is an open that cannot be proved —
    /// a shell read after `&&`, or inside a script with one exit status.
    ///
    /// ⚠ **Shown, never scored** (#1214). Counting these as opens overstates the
    /// record and scoring them at a discount invents a factor.
    pub maybe_breadth: usize,
    /// Days since it was last opened, or `None` if never.
    pub last_open: Option<i64>,
    /// Bytes its index line spends, which is what demoting it recovers. Zero
    /// for an entry the index does not carry.
    pub entry_cost: usize,
    /// What the line is for, where #884 has judged it.
    pub role: Option<Role>,
    /// Reachable memories that already link it — the homes a demotion could
    /// land in without stranding it.
    pub homes: Vec<String>,
    /// Whether #884's freeze covers it. See [`Trade::held`].
    pub frozen: bool,
}

impl Entry {
    /// Days since it was written, or `None` when nothing dates it.
    pub fn age(&self, today: i64) -> Option<i64> {
        self.created.map(|day| today - day)
    }

    pub fn tier(&self, today: i64, at: &Thresholds) -> Tier {
        match self.age(today) {
            None => Tier::Undated,
            Some(age) if age <= at.lease_days => Tier::Lease,
            Some(_) if self.breadth >= at.tenure_breadth => Tier::Tenure,
            Some(_) if self.breadth <= at.thin_breadth => Tier::Thin,
            Some(_) => Tier::Middle,
        }
    }
}

/// What one tier costs the root.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Weight {
    pub entries: usize,
    pub bytes: usize,
}

/// Tally the indexed entries by tier.
pub fn census(entries: &[Entry], today: i64, at: &Thresholds) -> BTreeMap<&'static str, Weight> {
    let mut out: BTreeMap<&'static str, Weight> = BTreeMap::new();
    for entry in entries.iter().filter(|e| e.indexed) {
        let slot = out.entry(entry.tier(today, at).label()).or_default();
        slot.entries += 1;
        slot.bytes += entry.entry_cost;
    }
    out
}

/// The bytes an entry's line is likely to cost before one has been written.
///
/// ⚠ **An admission's cost cannot be measured, only budgeted.** A demotion
/// recovers a line that exists and can be counted; an admission spends a line
/// nobody has written yet, whose length depends on the teaser somebody chooses.
/// The median of what the root already carries is the honest stand-in, and
/// naming it as a budget is what stops the trade below reading as exact.
pub fn median_entry_cost(entries: &[Entry]) -> usize {
    let mut costs: Vec<usize> = entries
        .iter()
        .filter(|e| e.indexed && e.entry_cost > 0)
        .map(|e| e.entry_cost)
        .collect();
    if costs.is_empty() {
        return 0;
    }
    costs.sort_unstable();
    costs[costs.len() / 2]
}

/// A proposed exchange: what would join the root, what would leave it, and
/// whether the root is smaller afterwards.
#[derive(Debug, Clone, Default)]
pub struct Trade {
    /// Not indexed, and consulted widely enough for tenure. Widest first.
    ///
    /// ⚠ **Every qualifier, not only the ones there is room for.** Truncating
    /// this at the budget hides the evidence that the root is the wrong size:
    /// a report saying "nothing has earned a slot" and one saying "eleven have
    /// and none fit" are different findings, and the second is the one that
    /// argues for a demotion pass.
    pub admit: Vec<Entry>,
    /// How many of `admit`, from the front, the budget actually covers.
    pub affordable: usize,
    /// Indexed, thin, and already linked from somewhere live.
    pub demote: Vec<Entry>,
    /// Would qualify on the evidence, but #884's freeze covers them until the
    /// harvest.
    ///
    /// ⚠ **Held, not dropped.** The freeze is on the SPLIT — do not re-promote
    /// a treated memory, do not demote a control one — so acting on these
    /// perturbs a series that has been running since 2026-08-14. Showing them
    /// separately is what lets the proposal be read now and applied after.
    pub held: Vec<Entry>,
    /// Bytes the demotions recover.
    pub recovered: usize,
    /// Bytes the admissions are budgeted at, at [`median_entry_cost`].
    pub budgeted: usize,
}

impl Trade {
    /// What the root's size becomes if this is carried out. Signed, because an
    /// exchange that grows the root is the answer that matters.
    pub fn net(&self) -> i64 {
        self.budgeted as i64 - self.recovered as i64
    }
}

/// Propose an exchange, both halves in one operation.
///
/// ⚠ **Admission and demotion have to be decided together or the ceiling is
/// breached.** The root grows by judgement and shrinks by measurement — a
/// ratchet pointing the wrong way, and the reason it drifts up rather than
/// settling. Two tools, one proposing joins and one proposing cuts, reproduce
/// that ratchet with more steps.
///
/// ⚠ **A demotion whose only home is another demotion is not offered.** Homes
/// were found against the index as it stands, so a pair that links only each
/// other reads as housed until both lines go together. `strands` names those,
/// and the caller asks the reachability question once of the whole set.
pub fn propose(
    entries: &[Entry],
    today: i64,
    at: &Thresholds,
    budget: usize,
    strands: &dyn Fn(&[Entry]) -> Vec<String>,
) -> Trade {
    let mut trade = Trade::default();

    let mut demote: Vec<Entry> = entries
        .iter()
        .filter(|e| e.indexed && e.tier(today, at) == Tier::Thin && !e.homes.is_empty())
        .cloned()
        .collect();
    demote.sort_by(|a, b| a.breadth.cmp(&b.breadth).then(a.name.cmp(&b.name)));

    let (held, free): (Vec<Entry>, Vec<Entry>) = demote.into_iter().partition(|e| e.frozen);
    let stranded = strands(&free);
    trade.demote = free
        .into_iter()
        .filter(|e| !stranded.contains(&e.name))
        .collect();
    trade.held = held;
    trade.recovered = trade.demote.iter().map(|e| e.entry_cost).sum();

    let mut admit: Vec<Entry> = entries
        .iter()
        .filter(|e| !e.indexed && e.breadth >= at.tenure_breadth)
        .cloned()
        .collect();
    admit.sort_by(|a, b| b.breadth.cmp(&a.breadth).then(a.name.cmp(&b.name)));

    // Spend what the demotions recovered plus whatever headroom the root has,
    // and stop — an admission list longer than the space for it is a wish.
    let room = trade.recovered + budget;
    let median = median_entry_cost(entries);
    let mut spent = 0usize;
    let mut fitting = true;
    for entry in admit {
        let cost = if entry.entry_cost > 0 {
            entry.entry_cost
        } else {
            median
        };
        if fitting && spent + cost <= room {
            spent += cost;
            trade.affordable += 1;
        } else {
            fitting = false;
        }
        trade.admit.push(entry);
    }
    trade.budgeted = spent;
    trade
}

/// Entries that crossed out of the lease within the last `window` days.
///
/// ⚠ **A lease expiring is an EVENT, not a state.** "Past its lease" describes
/// most of the root and is not a list anybody can act on; "crossed since you
/// last looked" is a handful and is. The backlog of everything that crossed
/// earlier is the census above, which is where it belongs — as a shape, once,
/// rather than as a to-do that regenerates every run.
///
/// What each one landed in is [`Entry::tier`]: tenure means the lease earned
/// out, thin means it did not, and middle means the evidence has not decided.
pub fn expired(entries: &[Entry], today: i64, at: &Thresholds, window: i64) -> Vec<Entry> {
    let mut out: Vec<Entry> = entries
        .iter()
        .filter(|e| e.indexed)
        .filter(|e| {
            e.age(today)
                .is_some_and(|age| age > at.lease_days && age <= at.lease_days + window)
        })
        .cloned()
        .collect();
    out.sort_by(|a, b| b.breadth.cmp(&a.breadth).then(a.name.cmp(&b.name)));
    out
}
