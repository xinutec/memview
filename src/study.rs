//! The demotion study's estimator (#884), written before its outcome existed.
//!
//! ⚠ **Written 2026-08-21, three weeks before the 2026-09-11 harvest, and
//! deliberately without looking at any post-period data.** Every choice left
//! open by the pre-registration — how ties in the match are broken, what an edit
//! with no read means, what happens to a memory that left the corpus — is a
//! choice that could be nudged toward a result if made while the answer was
//! visible. The pre-period was inspected; the post-period was not.
//!
//! The design, fixed in #884 before any post-period data existed:
//!
//!   treated   the 152 memories demoted from `MEMORY.md` between 08-10 and 08-14
//!   control   the 336 still listed at t = 2026-08-14
//!   outcome   DAYS a memory was opened, in [t-28, t) against [t, t+28)
//!   estimate  (treated_post - treated_pre) - (control_post - control_pre)
//!
//! ⚠ **Matching is mandatory and on the selection variable itself.** Demotion
//! was assigned BECAUSE opens were low — measured, not assumed: 82% of treated
//! had any pre-period open against 96% of control. An unmatched before/after
//! would show a fall from regression to the mean alone and would read as proof
//! that the index line works.

use std::collections::{BTreeMap, VecDeque};

/// What an index line is for, from `memory-roles.json`.
///
/// The two succeed differently — a tripwire works by being read in the index and
/// never opened — so an estimate that averages them answers neither question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Tripwire,
    Pointer,
}

/// One memory's exposure and outcome.
#[derive(Debug, Clone)]
pub struct Subject {
    pub name: String,
    pub treated: bool,
    /// Days with at least one open in `[t-28, t)`.
    pub pre: u32,
    /// Days with at least one open in `[t, t+28)`.
    pub post: u32,
    pub role: Option<Role>,
}

impl Subject {
    /// The within-memory change the estimate differences.
    fn change(&self) -> f64 {
        f64::from(self.post) - f64::from(self.pre)
    }

    /// ⚠ **A memory at zero pre-period opens cannot fall**, so it carries no
    /// information about whether the index line drives opens — a floor, not a
    /// null. 27 of the 152 treated are here. They are matched and reported like
    /// any other, and also counted separately, because quoting n=152 for an
    /// estimate that ~121 memories can actually move would overstate its base.
    pub fn can_fall(&self) -> bool {
        self.pre > 0
    }
}

/// A treated memory and the control it was matched to.
#[derive(Debug, Clone)]
pub struct Pair {
    pub treated: Subject,
    pub control: Subject,
}

/// The result of matching, including what it could not cover.
#[derive(Debug, Clone)]
pub struct Matching {
    pub pairs: Vec<Pair>,
    /// ⚠ Treated memories with no control at their own pre-period level.
    /// **Reported, never silently dropped** — 16 of 152 at the last check, both
    /// failures at the extremes of the distribution.
    pub unmatched: Vec<Subject>,
}

/// Exact 1:1 matching on pre-period opens.
///
/// Exact rather than a caliper: the variable is a small integer count (0..=17
/// observed), so "the same number of days" is available and needs no tolerance
/// to justify. Deterministic by construction — subjects are consumed in name
/// order within each stratum — so the same input gives the same pairs on any
/// machine and the estimate can be re-derived rather than trusted.
pub fn match_on_pre_opens(subjects: &[Subject]) -> Matching {
    let mut by_level: BTreeMap<u32, VecDeque<Subject>> = BTreeMap::new();
    let mut treated: Vec<Subject> = Vec::new();
    let mut controls: Vec<Subject> = subjects.iter().filter(|s| !s.treated).cloned().collect();
    controls.sort_by(|a, b| a.name.cmp(&b.name));
    for control in controls {
        by_level.entry(control.pre).or_default().push_back(control);
    }
    treated.extend(subjects.iter().filter(|s| s.treated).cloned());
    treated.sort_by(|a, b| a.name.cmp(&b.name));

    let mut pairs = Vec::new();
    let mut unmatched = Vec::new();
    for subject in treated {
        match by_level
            .get_mut(&subject.pre)
            .and_then(std::collections::VecDeque::pop_front)
        {
            Some(control) => pairs.push(Pair {
                treated: subject,
                control,
            }),
            None => unmatched.push(subject),
        }
    }
    Matching { pairs, unmatched }
}

/// A difference-in-differences estimate over matched pairs.
#[derive(Debug, Clone, PartialEq)]
pub struct Estimate {
    pub pairs: usize,
    /// Pairs whose treated memory could actually fall (`pre > 0`).
    pub informative: usize,
    pub treated_change: f64,
    pub control_change: f64,
    /// `treated_change - control_change`. Negative means demoted memories fell
    /// further than their matched controls, i.e. the index line was carrying
    /// opens.
    pub did: f64,
}

/// The estimate over a set of pairs.
///
/// Returns zeros for an empty set rather than a NaN: an arm with no pairs is a
/// fact to report, and a NaN downstream reads as a broken run instead.
pub fn difference_in_differences(pairs: &[&Pair]) -> Estimate {
    if pairs.is_empty() {
        return Estimate {
            pairs: 0,
            informative: 0,
            treated_change: 0.0,
            control_change: 0.0,
            did: 0.0,
        };
    }
    let n = pairs.len() as f64;
    let treated_change = pairs.iter().map(|p| p.treated.change()).sum::<f64>() / n;
    let control_change = pairs.iter().map(|p| p.control.change()).sum::<f64>() / n;
    Estimate {
        pairs: pairs.len(),
        informative: pairs.iter().filter(|p| p.treated.can_fall()).count(),
        treated_change,
        control_change,
        did: treated_change - control_change,
    }
}

/// The arms the pre-registration asks for, each estimated on its own.
///
/// `reference_` is the arm this ticket turns on: those memories are mostly
/// tripwires, and a tripwire's success mode is being read in the index and never
/// opened — so ranking them by opens asks the wrong question of them.
pub fn by_arm(matching: &Matching) -> Vec<(String, Estimate)> {
    let all: Vec<&Pair> = matching.pairs.iter().collect();
    let mut out = vec![("all".to_string(), difference_in_differences(&all))];
    for (label, keep) in [
        (
            "reference_",
            &(|p: &Pair| p.treated.name.starts_with("reference_")) as &dyn Fn(&Pair) -> bool,
        ),
        ("project_", &|p: &Pair| {
            p.treated.name.starts_with("project_")
        }),
        ("role: tripwire", &|p: &Pair| {
            p.treated.role == Some(Role::Tripwire)
        }),
        ("role: pointer", &|p: &Pair| {
            p.treated.role == Some(Role::Pointer)
        }),
    ] {
        let arm: Vec<&Pair> = matching.pairs.iter().filter(|p| keep(p)).collect();
        out.push((label.to_string(), difference_in_differences(&arm)));
    }
    out
}

/// Today, as `YYYY-MM-DD`.
///
/// Its own function so the harvest guard has one place to be read from, and so a
/// test never has to reach for the real clock.
pub fn today() -> String {
    time::OffsetDateTime::now_utc()
        .date()
        .format(&time::macros::format_description!("[year]-[month]-[day]"))
        .unwrap_or_default()
}
