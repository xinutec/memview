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
/// ⚠ **A closed set with ONE definition of each filter.** The estimate and its
/// null band have to be computed over the same pairs, and the band lives in the
/// caller — so a second copy of "what is in this arm", written as a `match` on
/// the label, would let a band describe a different subset than the number
/// beside it with nothing to show for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    All,
    /// The arm #884 turns on: mostly tripwires, whose success mode is being read
    /// in the index and never opened, so ranking them by opens asks the wrong
    /// question of them.
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

/// A deterministic 64-bit generator, written out rather than taken from a crate.
///
/// ⚠ **A dependency's generator is not guaranteed stable across versions**, and
/// this one decides whether a result is called distinguishable from zero. A
/// `rand` bump that silently changed the stream would move the null band under a
/// published estimate with nothing in the diff to show it. SplitMix64 is six
/// lines, is fixed by its constants, and gives the same band on any machine and
/// any year — the same reason [`match_on_pre_opens`] is deterministic.
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

/// The per-pair differences the estimate is the mean of.
///
/// Public because the null band below is a statement about THESE numbers, and a
/// caller that recomputed them another way could band a different quantity than
/// the one it reports.
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
    /// Whether an estimate is inside the band, i.e. indistinguishable from zero.
    ///
    /// ⚠ **This is the decision rule's missing half.** #884 says to act on
    /// whether the estimate is "indistinguishable from zero" and nothing in the
    /// code could answer that — the estimator returned a point and no spread, so
    /// any number at all read as an effect.
    pub fn covers(&self, did: f64) -> bool {
        self.lo <= did && did <= self.hi
    }
}

/// A 95% null band for the DiD, by flipping the sign of each pair's difference.
///
/// ⚠ **Sign-flipping is the test the design licenses.** Under the null that
/// demotion did nothing, a matched pair's two members are exchangeable, so
/// negating a pair's difference gives an equally likely dataset. That needs no
/// distributional assumption — which matters here, where the outcome is a small
/// integer count with a floor at zero and is nothing like normal.
///
/// ⚠ **It says nothing about BIAS.** The band is symmetric around zero by
/// construction, so it can only ask whether an estimate is larger than noise. An
/// estimator that returns the same number when no treatment happened will
/// produce an estimate outside this band and still be measuring nothing —
/// [`placebo`] is what answers that, and the two are not substitutes.
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

/// One memory's exposure and outcome, measured around an arbitrary day.
///
/// ⚠ **Factored out so the placebo cannot drift from the estimate.** A placebo
/// built by a second copy of this loop would answer a slightly different
/// question than the one it is supposed to be checking, and the divergence would
/// be invisible — the same "second implementation of one invariant" that let
/// `memory-rank` strand a pair (#869).
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
    /// Whether the procedure claimed an effect where there was none to find.
    ///
    /// ⚠ **True here invalidates the real estimate, it does not qualify it.** A
    /// difference-in-differences rests on the two arms trending together in the
    /// absence of treatment; this is that assumption measured. When it fails,
    /// the harvest number is the sum of an effect and a divergence that was
    /// already running, and nothing in the output can separate them.
    pub fn flags_an_effect(&self) -> bool {
        !self.null.covers(self.estimate.did)
    }
}

/// Run the whole procedure at fake treatment days before the real one.
///
/// ⚠ **Uses only days before `t`, so it can be run while the study is live.**
/// That is the point: a pre-trend found after the harvest is an excuse, and the
/// same finding eleven days before it is still a design decision.
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

/// The treated-minus-control gap in one window, and what it was measured on.
///
/// ⚠ **A GAP, not an effect.** It is the level of the difference between the
/// arms over one stretch of time. Under parallel trends a series of these is
/// flat before treatment and steps at it; the whole point of measuring them
/// separately is that here it is NOT flat, and a single before/after cannot
/// tell a step from a slope.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gap {
    /// Which window, counted in bin widths from `t`. `-1` is the bin ending at
    /// `t`, `0` is the first bin after it.
    pub at: i64,
    pub pairs: usize,
    pub gap: f64,
}

/// The treated-minus-control gap in each equal-length bin around `t`.
///
/// `leads` bins before `t` and one bin after. Pairs are matched ONCE, on opens
/// in the bin immediately before `t`, and the same pairing is used for every
/// bin — re-matching per bin would let the composition move under the series
/// and turn a change of membership into an apparent trend.
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

/// A straight line through the pre-treatment gaps, as `gap ≈ slope * at + at_zero`.
///
/// ⚠ **Least squares over the LEADS ONLY.** The post bin is what the line is
/// used to predict; fitting it in would let the effect pull the counterfactual
/// toward itself and shrink the very thing being measured.
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

/// Fit the counterfactual through the pre-treatment gaps.
///
/// Returns `None` for fewer than two bins: one point has no slope, and pretending
/// it does — by assuming flat — is the parallel-trends assumption this exists to
/// stop assuming.
pub fn pre_trend(gaps: &[Gap]) -> Option<Trend> {
    // ⚠ **`at != -1`: the anchor is excluded.** Exact matching forces the gap in
    // the bin before `t` to zero, so it is a definition rather than a
    // measurement — fitting through it drags every slope toward flat and hides
    // the drift this exists to expose.
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
    // Every lead at the same bin index cannot happen (they are distinct by
    // construction), but a zero denominator would be a silent NaN downstream.
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
/// ⚠⚠ **THIS DOES NOT RESCUE #884'S DESIGN, and that was measured rather than
/// feared.** It was written as the standard repair for a failed parallel-trends
/// assumption and put to the same acceptance test as the raw estimator: run at
/// fake treatment days where nothing was demoted, it must return ~0.
///
/// ```text
/// fake t     raw      corrected
/// t-28d    -1.121       +0.288
/// t-21d    -1.620       -0.233
/// t-14d    -1.007       -0.763
/// ```
///
/// It does not. The residue reaches 2.2x the noise floor — the correction's own
/// error is larger than the effect it exists to recover — and adding leads moves
/// it without settling it. The pre-period gap series is not a clean trend plus
/// noise; extrapolating a line through it amplifies the noise.
///
/// ⚠ **So this is a DIAGNOSTIC, not an estimator.** It is kept because the gap
/// series it rests on is what made the problem legible, and because a caller
/// that wants the corrected number should have to see the placebo beside it —
/// `demotion-study` prints both and never one alone.
///
/// ⚠ **The anchor bin is zero BY CONSTRUCTION.** Matching is exact on opens in
/// `[t-bin, t)`, so the gap there cannot be anything but zero and carries no
/// information about the trend. [`pre_trend`] fits through the other leads for
/// that reason; including it was the first version and it flattened every slope
/// toward nothing.
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
