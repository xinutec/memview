//! The demotion study's estimator (#884), written 2026-08-21, three weeks
//! before the 2026-09-11 harvest and without looking at post-period data: every
//! choice the pre-registration left open was made blind.
//!
//!   treated   the 152 memories demoted from `MEMORY.md` between 08-10 and 08-14
//!   control   the 336 still listed at t = 2026-08-14
//!   outcome   DAYS a memory was opened, in [t-28, t) against [t, t+28)
//!   estimate  (treated_post - treated_pre) - (control_post - control_pre)
//!
//! Matching is mandatory and on the selection variable: demotion was assigned
//! BECAUSE opens were low (82% of treated had any pre-period open against 96%
//! of control), so an unmatched before/after shows regression to the mean.

use std::collections::{BTreeMap, VecDeque};

/// What an index line is for, from `memory-roles.json`. A tripwire works by
/// being read and never opened, so an estimate averaging the two answers neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Tripwire,
    Pointer,
}

/// What `memory-roles.json` judges one name to be. One reader, because three
/// tools decide demotion from this file; two copies of the string match gave
/// opposite answers for 192 entries (memview#884). `None` is a THIRD state and
/// never a pointer: unjudged is unexamined, and a caller must hold it.
pub fn role_of(roles: &serde_json::Value, name: &str) -> Option<Role> {
    named_role(roles["roles"][name].as_str())
}

/// Whether an index line STATES ITS CLAIM, which is what makes a line fire on a
/// reader who did not come looking — `feedback_pointer_or_tripwire`'s test.
///
/// ⚠ **One definition, deliberately.** Two copies of a string match gave opposite
/// answers for 192 entries (memview#884), which is why `role_of` above reads one
/// record rather than re-deriving. The same applies here: `memory-lint` and
/// `memory-tiers` must agree about a line, or one will offer a demotion the other
/// reports as a defect.
pub fn states_a_claim(label: &str) -> bool {
    label.contains("**") || label.split_whitespace().count() >= 4
}

/// The vocabulary itself. Anything unrecognised is `None`, never a third kind.
pub fn named_role(text: Option<&str>) -> Option<Role> {
    match text {
        Some("tripwire") => Some(Role::Tripwire),
        Some("pointer") => Some(Role::Pointer),
        _ => None,
    }
}

/// A memory's role, taking the AUTHOR'S declaration over the record. The record
/// is one model's classification memview#884 was pre-registered on, so it cannot
/// be re-run and cannot keep up; every memory written after a pass stayed
/// exempt forever (memview#1537). The frontmatter fixes that without a backfill.
pub fn role_for(declared: Option<&str>, roles: &serde_json::Value, name: &str) -> Option<Role> {
    named_role(declared).or_else(|| role_of(roles, name))
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

    /// A memory at zero pre-period opens cannot fall — a floor, not a null. 27 of
    /// the 152 treated; matched and reported, and counted separately.
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
    /// Treated memories with no control at their own pre-period level. Reported,
    /// never dropped — 16 of 152 at the last check.
    pub unmatched: Vec<Subject>,
}

/// Exact 1:1 matching on pre-period opens. Exact rather than a caliper: a small
/// integer count. Deterministic — subjects consumed in name order — so the
/// estimate can be re-derived rather than trusted.
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

/// Zeros for an empty set rather than a NaN, which reads as a broken run.
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

/// The arms the pre-registration asks for. A closed set with ONE definition of
/// each filter: the null band lives in the caller and must be computed over the
/// same pairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    All,
    /// The arm #884 turns on: mostly tripwires, which succeed by being read in the
    /// index and never opened.
    Reference,
    Project,
    Tripwire,
    Pointer,
}

impl Arm {
    /// Every arm, in report order.
    pub const EVERY: [Arm; 5] = [
        Arm::All,
        Arm::Reference,
        Arm::Project,
        Arm::Tripwire,
        Arm::Pointer,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Arm::All => "all",
            Arm::Reference => "reference_",
            Arm::Project => "project_",
            Arm::Tripwire => "role: tripwire",
            Arm::Pointer => "role: pointer",
        }
    }

    /// Whether a pair belongs to this arm, by its TREATED member.
    pub fn holds(self, pair: &Pair) -> bool {
        match self {
            Arm::All => true,
            Arm::Reference => pair.treated.name.starts_with("reference_"),
            Arm::Project => pair.treated.name.starts_with("project_"),
            Arm::Tripwire => pair.treated.role == Some(Role::Tripwire),
            Arm::Pointer => pair.treated.role == Some(Role::Pointer),
        }
    }

    /// This arm's pairs out of a matching.
    pub fn pairs(self, matching: &Matching) -> Vec<&Pair> {
        matching.pairs.iter().filter(|p| self.holds(p)).collect()
    }
}

pub fn by_arm(matching: &Matching) -> Vec<(Arm, Estimate)> {
    Arm::EVERY
        .iter()
        .map(|&arm| (arm, difference_in_differences(&arm.pairs(matching))))
        .collect()
}

/// Today, as `YYYY-MM-DD`. Its own function so a test never reaches for the clock.
pub fn today() -> String {
    time::OffsetDateTime::now_utc()
        .date()
        .format(&time::macros::format_description!("[year]-[month]-[day]"))
        .unwrap_or_default()
}

/// A deterministic 64-bit generator, written out rather than taken from a crate:
/// a `rand` bump could silently move the null band under a published estimate.
/// SplitMix64 is six lines and fixed by its constants.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A fair coin.
    fn flip(&mut self) -> bool {
        self.next() >> 63 == 1
    }
}

/// The per-pair differences the estimate is the mean of. Public because the null
/// band is a statement about THESE numbers.
pub fn pair_differences(pairs: &[&Pair]) -> Vec<f64> {
    pairs
        .iter()
        .map(|p| p.treated.change() - p.control.change())
        .collect()
}

/// Where a DiD would fall if the pairing carried no effect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Null {
    pub lo: f64,
    pub hi: f64,
    pub draws: usize,
}

impl Null {
    /// Whether an estimate is inside the band — the decision rule's missing half:
    /// #884 says to act on "indistinguishable from zero", and a point estimate
    /// without a spread made any number read as an effect.
    pub fn covers(&self, did: f64) -> bool {
        self.lo <= did && did <= self.hi
    }
}

/// A 95% null band for the DiD, by flipping the sign of each pair's difference:
/// under the null a matched pair's members are exchangeable, and this needs no
/// distributional assumption for an outcome floored at zero. It says nothing
/// about BIAS — [`placebo`] answers that, and the two are not substitutes.
pub fn sign_flip_null(diffs: &[f64], draws: usize, seed: u64) -> Null {
    if diffs.is_empty() || draws == 0 {
        return Null {
            lo: 0.0,
            hi: 0.0,
            draws: 0,
        };
    }
    let mut rng = SplitMix64(seed);
    let n = diffs.len() as f64;
    let mut means: Vec<f64> = (0..draws)
        .map(|_| {
            diffs
                .iter()
                .map(|d| if rng.flip() { *d } else { -*d })
                .sum::<f64>()
                / n
        })
        .collect();
    means.sort_by(f64::total_cmp);
    Null {
        lo: means[draws * 25 / 1000],
        hi: means[(draws * 975 / 1000).min(draws - 1)],
        draws,
    }
}

/// One memory's exposure and outcome, measured around an arbitrary day. Factored
/// out so the placebo cannot drift from the estimate (#869's lesson).
pub fn subjects_at(
    treated: &[String],
    control: &[String],
    t0: i64,
    window: i64,
    opens: &dyn Fn(&str, i64, i64) -> u32,
    role: &dyn Fn(&str) -> Option<Role>,
) -> Vec<Subject> {
    let mut out = Vec::new();
    for (names, is_treated) in [(treated, true), (control, false)] {
        for name in names {
            out.push(Subject {
                pre: opens(name, t0 - window, t0),
                post: opens(name, t0, t0 + window),
                name: name.clone(),
                treated: is_treated,
                role: role(name),
            });
        }
    }
    out
}

/// What the pre-registered procedure reports at a day when nothing happened.
#[derive(Debug, Clone)]
pub struct Placebo {
    /// The fake treatment day.
    pub at: i64,
    pub estimate: Estimate,
    pub null: Null,
}

impl Placebo {
    /// Whether the procedure claimed an effect where there was none. True
    /// INVALIDATES the real estimate: the parallel-trends assumption, measured.
    pub fn flags_an_effect(&self) -> bool {
        !self.null.covers(self.estimate.did)
    }
}

/// Run the whole procedure at fake treatment days before the real one. Only
/// days before `t`, so it can run while the study is live — a pre-trend found
/// after the harvest is an excuse.
pub fn placebo(
    treated: &[String],
    control: &[String],
    fake_days: &[i64],
    window: i64,
    opens: &dyn Fn(&str, i64, i64) -> u32,
    role: &dyn Fn(&str) -> Option<Role>,
    seed: u64,
) -> Vec<Placebo> {
    fake_days
        .iter()
        .map(|&at| {
            let subjects = subjects_at(treated, control, at, window, opens, role);
            let matching = match_on_pre_opens(&subjects);
            let pairs: Vec<&Pair> = matching.pairs.iter().collect();
            let estimate = difference_in_differences(&pairs);
            let null = sign_flip_null(&pair_differences(&pairs), 4000, seed);
            Placebo { at, estimate, null }
        })
        .collect()
}

// ── The pre-trend correction (#884, written 2026-08-31, eleven days before the
// ── harvest and without looking at any post-period outcome).

/// The treated-minus-control gap in one window. A GAP, not an effect: under
/// parallel trends a series of these is flat before treatment and steps at it;
/// here it is NOT flat, and one before/after cannot tell a step from a slope.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gap {
    /// Which window, counted in bin widths from `t`. `-1` is the bin ending at
    /// `t`, `0` is the first bin after it.
    pub at: i64,
    pub pairs: usize,
    pub gap: f64,
}

/// The gap in each equal-length bin around `t`: `leads` before, one after.
/// Pairs are matched ONCE, on the bin before `t`, and the same pairing serves
/// every bin, or a change of membership reads as a trend.
pub fn event_study(
    treated: &[String],
    control: &[String],
    t: i64,
    bin: i64,
    leads: usize,
    opens: &dyn Fn(&str, i64, i64) -> u32,
    role: &dyn Fn(&str) -> Option<Role>,
) -> Vec<Gap> {
    // One pairing, from the bin that decided selection.
    let base = subjects_at(treated, control, t, bin, opens, role);
    let matching = match_on_pre_opens(&base);

    let mut out = Vec::new();
    for k in -(leads as i64)..=0 {
        let lo = t + k * bin;
        let hi = lo + bin;
        let mut diffs = Vec::new();
        for pair in &matching.pairs {
            let a = f64::from(opens(&pair.treated.name, lo, hi));
            let b = f64::from(opens(&pair.control.name, lo, hi));
            diffs.push(a - b);
        }
        if diffs.is_empty() {
            continue;
        }
        let n = diffs.len();
        out.push(Gap {
            at: k,
            pairs: n,
            gap: diffs.iter().sum::<f64>() / n as f64,
        });
    }
    out
}

/// A straight line through the pre-treatment gaps. Least squares over the LEADS
/// ONLY: fitting the post bin in would let the effect pull the counterfactual.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trend {
    pub slope: f64,
    pub at_zero: f64,
    /// How many bins the line was fitted through.
    pub bins: usize,
}

impl Trend {
    /// What the gap would have been at bin `k` had nothing happened.
    pub fn expected(&self, k: i64) -> f64 {
        self.slope * k as f64 + self.at_zero
    }
}

/// Fit the counterfactual through the pre-treatment gaps. `None` for fewer than
/// two bins: assuming flat is the assumption this exists to stop assuming.
pub fn pre_trend(gaps: &[Gap]) -> Option<Trend> {
    // `at != -1`: exact matching forces the anchor bin's gap to zero, so it is a
    // definition, and fitting through it drags every slope toward flat.
    let leads: Vec<&Gap> = gaps.iter().filter(|g| g.at < 0 && g.at != -1).collect();
    if leads.len() < 2 {
        return None;
    }
    let n = leads.len() as f64;
    let mean_x = leads.iter().map(|g| g.at as f64).sum::<f64>() / n;
    let mean_y = leads.iter().map(|g| g.gap).sum::<f64>() / n;
    let mut num = 0.0;
    let mut den = 0.0;
    for g in &leads {
        let dx = g.at as f64 - mean_x;
        num += dx * (g.gap - mean_y);
        den += dx * dx;
    }
    // Distinct by construction, but a zero denominator would be a silent NaN.
    if den == 0.0 {
        return None;
    }
    let slope = num / den;
    Some(Trend {
        slope,
        at_zero: mean_y - slope * mean_x,
        bins: leads.len(),
    })
}

/// The estimate after the divergence that was already running is subtracted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corrected {
    /// The gap actually observed in the first post-treatment bin.
    pub observed: f64,
    /// What the pre-treatment line predicted for that bin.
    pub expected: f64,
    /// `observed - expected`. Negative means demoted memories fell further than
    /// the trend they were already on.
    pub effect: f64,
    pub trend: Trend,
}

/// Subtract the pre-existing divergence from the post-treatment gap.
///
/// THIS DOES NOT RESCUE #884'S DESIGN, measured rather than feared: put to the
/// same acceptance test as the raw estimator — fake treatment days must return
/// ~0 — it does not.
///
/// ```text
/// fake t     raw      corrected
/// t-28d    -1.121       +0.288
/// t-21d    -1.620       -0.233
/// t-14d    -1.007       -0.763
/// ```
///
/// The residue reaches 2.2x the noise floor. So this is a DIAGNOSTIC, kept
/// because the gap series made the problem legible; `demotion-study` prints it
/// beside the placebo and never alone. The anchor bin is zero by construction,
/// which is why [`pre_trend`] fits through the other leads.
pub fn correct(gaps: &[Gap]) -> Option<Corrected> {
    let post = gaps.iter().find(|g| g.at == 0)?;
    let trend = pre_trend(gaps)?;
    let expected = trend.expected(0);
    Some(Corrected {
        observed: post.gap,
        expected,
        effect: post.gap - expected,
        trend,
    })
}
