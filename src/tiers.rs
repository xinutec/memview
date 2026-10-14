//! The index's two populations — recent work, which turns over, and consolidated
//! rules, which must stay (`docs/memory.md`) — and the exchange between them.
//! Breadth counts distinct agents, never raw opens.

use std::collections::BTreeMap;

/// What an index line is for, from `memory-roles.json` — re-exported from the study.
pub use crate::study::Role;

/// Which population an entry belongs to, and therefore what holds it in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// New, live, and exempt for a while. The failure this tier makes visible is a
    /// lease quietly becoming tenure.
    Lease,
    /// Past its lease and consulted widely: it fires in situations other than
    /// the one that wrote it, which is the qualification for a root slot.
    Tenure,
    /// Past its lease, consulted by some but not many. Neither earned nor
    /// expired — reported so the middle is not silently read as either.
    Middle,
    /// Past its lease and consulted by almost nobody.
    Thin,
    /// No creation date, so its lease cannot be judged — its own tier, not a default.
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
    /// Days of the last [`crate::agents::RETURN_WINDOW`] on which one agent came
    /// back to a memory for it to be that agent's working set, and held.
    pub working_days: usize,
}

impl Default for Thresholds {
    /// Measured against the corpus (#1210): a fortnight matches the ranking
    /// half-life, and the agent threshold is where the unindexed population thins out.
    fn default() -> Self {
        Thresholds {
            lease_days: 14,
            tenure_breadth: 6,
            thin_breadth: 2,
            // Measured 2026-09-26: the median indexed memory's most loyal agent
            // came back on 1 day of the last 30, the 90th percentile on 4.
            working_days: 5,
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
    /// Distinct agents whose only evidence is an open that cannot be proved. Shown,
    /// never scored (#1214).
    pub maybe_breadth: usize,
    /// Of [`Self::breadth`], the agents that opened it outside a sweep — see
    /// [`Breadth::unswept`]. What ADMIT counts.
    pub unswept_breadth: usize,
    /// The most days any one agent came back to it lately — see [`returns`].
    pub returns: usize,
    /// Days since it was last opened, or `None` if never.
    pub last_open: Option<i64>,
    /// Bytes its index line spends, which is what demoting it recovers. Zero
    /// for an entry the index does not carry.
    pub entry_cost: usize,
    /// What the line is for, where #884 has judged it.
    pub role: Option<Role>,
    /// Whether the index line states its claim — [`crate::study::states_a_claim`].
    pub claims: bool,
    /// Reachable memories that already link it — the homes a demotion could
    /// land in without stranding it.
    pub homes: Vec<String>,
    /// Whether #884's freeze covers it. See [`Trade::held`].
    pub frozen: bool,
    /// How many links from the index reach it — 1 for a root line, `None` if none.
    pub depth: Option<usize>,
}

/// Where a demoted entry's target lands; `None` is stranded, never allowed.
pub fn falls(before: Option<usize>, after: Option<usize>) -> String {
    match (before, after) {
        (_, None) => "STRANDS".to_string(),
        (Some(was), Some(now)) => format!("{was}h→{now}h"),
        (None, Some(now)) => format!("→{now}h"),
    }
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

/// The bytes an entry's line is likely to cost before one has been written: an
/// admission's cost can only be budgeted, and the root's median is the stand-in.
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

/// Distinct agents that have opened a memory. Forty opens by one agent count
/// once: breadth is how widely a memory travelled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Breadth {
    /// Agents with a proven open.
    pub proven: usize,
    /// Agents whose only evidence is an open that cannot be proved.
    pub unprovable: usize,
    /// Of `proven`, the agents that opened it outside a sweep
    /// ([`crate::agents::Agent::swept`]). Shown, not yet scored (#1735).
    pub unswept: usize,
}

/// The [`Breadth`] of `memory`, without `excluding` — the session judging it,
/// whose own reading would otherwise count.
pub fn breadth(agents: &[crate::agents::Agent], memory: &str, excluding: Option<&str>) -> Breadth {
    let mut out = Breadth::default();
    for agent in agents
        .iter()
        .filter(|agent| excluding != Some(agent.name.as_str()))
    {
        let Some(use_) = agent.memories.get(memory) else {
            continue;
        };
        if use_.reads > 0 {
            out.proven += 1;
            if !agent.swept.contains(memory) {
                out.unswept += 1;
            }
        } else if use_.maybe_reads > 0 {
            out.unprovable += 1;
        }
    }
    out
}

/// The most days, in the last [`crate::agents::RETURN_WINDOW`], that any one
/// agent other than `excluding` came back to `memory` outside a sweep.
pub fn returns(agents: &[crate::agents::Agent], memory: &str, excluding: Option<&str>) -> usize {
    agents
        .iter()
        .filter(|agent| excluding != Some(agent.name.as_str()))
        .filter_map(|agent| agent.returned.get(memory))
        .max()
        .copied()
        .unwrap_or(0)
}

/// Why a demotion the evidence would offer is not being offered. Checked in this
/// order: the freeze lifts at the harvest, a tripwire's reason never does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// The line IS the memory — a low open count is what SUCCESS looks like.
    Tripwire,
    /// Judged a pointer, but the line states a claim. The line is what a reader
    /// meets, so it is held on that and the judgement reported instead.
    Claims,
    /// One agent keeps coming back to it: its working set, which breadth
    /// cannot see (`docs/memory.md`, "Breadth alone punishes deep focus").
    WorkingSet,
    /// An absent judgement is not a pointer: it fails toward deleting a rule that
    /// fires from its line.
    Unjudged,
    /// Thin only because unscored opens were discarded.
    Unproven,
    /// #884's freeze is on the SPLIT: acting on it perturbs a running series.
    Frozen,
}

/// A demotion the evidence supports and something else forbids.
#[derive(Debug, Clone)]
pub struct HeldEntry {
    pub entry: Entry,
    pub why: Held,
}

/// A proposed exchange: what would join the root, what would leave it, and
/// whether the root is smaller afterwards.
#[derive(Debug, Clone, Default)]
pub struct Trade {
    /// Not indexed, yet found widely enough outside sweeps for tenure — all of them.
    pub admit: Vec<Entry>,
    /// How many of `admit`, from the front, the budget actually covers.
    pub affordable: usize,
    /// Indexed, thin, housed, and judged a POINTER — the only role a demotion may
    /// be proposed for.
    pub demote: Vec<Entry>,
    /// Would qualify on opens, and must not be demoted anyway. Held, not dropped:
    /// the reason is the finding.
    pub held: Vec<HeldEntry>,
    /// Memories kept from tenure only by uncountable shell opens — counted,
    /// never admitted.
    pub unproven_admissions: usize,
    /// Memories that clear the tenure bar only through agents that reached them
    /// in a sweep: audited, not found (#1735). Counted, never admitted.
    pub swept_admissions: usize,
    /// Bytes the demotions recover.
    pub recovered: usize,
    /// Bytes the admissions are budgeted at, at [`median_entry_cost`].
    pub budgeted: usize,
}

impl Trade {
    /// What the root's size becomes. Signed: an exchange that grows the root is the
    /// answer that matters.
    pub fn net(&self) -> i64 {
        self.budgeted as i64 - self.recovered as i64
    }
}

/// Propose an exchange, both halves at once. A demotion whose only home is another
/// demotion is not offered.
pub fn propose(
    entries: &[Entry],
    today: i64,
    at: &Thresholds,
    budget: usize,
    strands: &dyn Fn(&[Entry]) -> Vec<String>,
) -> Trade {
    let mut trade = Trade::default();

    let mut candidates: Vec<Entry> = entries
        .iter()
        .filter(|e| e.indexed && e.tier(today, at) == Tier::Thin && !e.homes.is_empty())
        .cloned()
        .collect();
    candidates.sort_by(|a, b| a.breadth.cmp(&b.breadth).then(a.name.cmp(&b.name)));

    // The tier alone never selects a demotion: a tripwire's low count is success.
    let mut free: Vec<Entry> = Vec::new();
    for entry in candidates {
        // Would counting the unprovable opens lift it out of THIN?
        let turns_on_discarded = entry.breadth + entry.maybe_breadth > at.thin_breadth;
        let why = match entry.role {
            Some(Role::Tripwire) => Some(Held::Tripwire),
            None => Some(Held::Unjudged),
            // First: a stated claim does not expire; the other holds do.
            Some(Role::Pointer) if entry.claims => Some(Held::Claims),
            Some(Role::Pointer) if entry.returns >= at.working_days => Some(Held::WorkingSet),
            Some(Role::Pointer) if turns_on_discarded => Some(Held::Unproven),
            Some(Role::Pointer) if entry.frozen => Some(Held::Frozen),
            Some(Role::Pointer) => None,
        };
        match why {
            Some(why) => trade.held.push(HeldEntry { entry, why }),
            None => free.push(entry),
        }
    }

    let stranded = strands(&free);
    trade.demote = free
        .into_iter()
        .filter(|e| !stranded.contains(&e.name))
        .collect();
    trade.recovered = trade.demote.iter().map(|e| e.entry_cost).sum();

    // Admission counts only agents that found a memory, not ones that swept past
    // it; demotion above keeps raw breadth, where an undercount loses a rule.
    trade.unproven_admissions = entries
        .iter()
        .filter(|e| !e.indexed && e.unswept_breadth < at.tenure_breadth)
        .filter(|e| e.unswept_breadth + e.maybe_breadth >= at.tenure_breadth)
        .count();
    trade.swept_admissions = entries
        .iter()
        .filter(|e| !e.indexed && e.unswept_breadth < at.tenure_breadth)
        .filter(|e| e.breadth >= at.tenure_breadth)
        .count();

    let mut admit: Vec<Entry> = entries
        .iter()
        .filter(|e| !e.indexed && e.unswept_breadth >= at.tenure_breadth)
        .cloned()
        .collect();
    admit.sort_by(|a, b| {
        b.unswept_breadth
            .cmp(&a.unswept_breadth)
            .then(b.breadth.cmp(&a.breadth))
            .then(a.name.cmp(&b.name))
    });

    // Spend what the demotions recovered plus the root's headroom, and stop.
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

/// Entries whose lease ran out within the last `window` days.
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
