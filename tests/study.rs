//! The demotion study's estimator (#884), tested on synthetic data only.
//!
//! Synthetic because the real corpus's post-period does not exist yet and must
//! not be consulted while these choices are being made — see `memview::study`.

use memview::study::{Role, Subject, by_arm, difference_in_differences, match_on_pre_opens};

fn subject(name: &str, treated: bool, pre: u32, post: u32) -> Subject {
    Subject {
        name: name.to_string(),
        treated,
        pre,
        post,
        role: None,
    }
}

/// A treated memory is paired with a control that had the SAME pre-period opens.
#[test]
fn matching_is_exact_on_pre_period_opens() {
    let matching = match_on_pre_opens(&[
        subject("t_a", true, 3, 1),
        subject("c_wrong", false, 9, 9),
        subject("c_right", false, 3, 2),
    ]);
    assert_eq!(matching.pairs.len(), 1);
    assert_eq!(matching.pairs[0].control.name, "c_right");
    assert!(matching.unmatched.is_empty());
}

/// ⚠ A treated memory with no control at its level is REPORTED, not dropped. A
/// silent drop would shrink the sample toward whichever memories happened to
/// match, which is a second selection on top of the one being controlled for.
#[test]
fn a_treated_memory_with_no_control_at_its_level_is_reported() {
    let matching = match_on_pre_opens(&[
        subject("t_lonely", true, 7, 0),
        subject("c_elsewhere", false, 2, 2),
    ]);
    assert!(matching.pairs.is_empty());
    assert_eq!(matching.unmatched.len(), 1);
    assert_eq!(matching.unmatched[0].name, "t_lonely");
}

/// Same input, same pairs — the estimate can be re-derived rather than trusted.
#[test]
fn matching_is_deterministic() {
    let subjects = [
        subject("t_b", true, 2, 1),
        subject("t_a", true, 2, 0),
        subject("c_z", false, 2, 2),
        subject("c_y", false, 2, 1),
    ];
    let once = match_on_pre_opens(&subjects);
    let twice = match_on_pre_opens(&subjects);
    let names = |m: &memview::study::Matching| -> Vec<(String, String)> {
        m.pairs
            .iter()
            .map(|p| (p.treated.name.clone(), p.control.name.clone()))
            .collect()
    };
    assert_eq!(names(&once), names(&twice));
    assert_eq!(names(&once)[0], ("t_a".to_string(), "c_y".to_string()));
}

/// ⚠ **The trap the control group exists for.** Demotion was assigned because
/// opens were low, so treated memories regress upward or fall on their own. Here
/// both arms fall by exactly 2: a naive before/after on the treated alone would
/// report a fall of 2 and read as proof the index line works. The DiD is 0.
#[test]
fn a_fall_that_the_control_also_shows_is_not_an_effect() {
    let matching = match_on_pre_opens(&[subject("t_a", true, 5, 3), subject("c_a", false, 5, 3)]);
    let pairs: Vec<_> = matching.pairs.iter().collect();
    let estimate = difference_in_differences(&pairs);
    assert_eq!(estimate.treated_change, -2.0, "the treated arm did fall");
    assert_eq!(estimate.control_change, -2.0, "and so did its control");
    assert_eq!(
        estimate.did, 0.0,
        "so nothing is attributable to the demotion"
    );
}

/// A fall beyond the control's is what the decision rule is about.
#[test]
fn a_fall_beyond_the_control_is_the_effect() {
    let matching = match_on_pre_opens(&[subject("t_a", true, 6, 1), subject("c_a", false, 6, 5)]);
    let pairs: Vec<_> = matching.pairs.iter().collect();
    assert_eq!(difference_in_differences(&pairs).did, -4.0);
}

/// A memory at zero pre-period opens cannot fall, so it is counted but not
/// called informative — quoting the pair count alone overstates the base.
#[test]
fn a_memory_that_cannot_fall_is_not_counted_as_informative() {
    let matching = match_on_pre_opens(&[
        subject("t_floor", true, 0, 0),
        subject("c_floor", false, 0, 0),
        subject("t_real", true, 4, 2),
        subject("c_real", false, 4, 4),
    ]);
    let pairs: Vec<_> = matching.pairs.iter().collect();
    let estimate = difference_in_differences(&pairs);
    assert_eq!(estimate.pairs, 2);
    assert_eq!(estimate.informative, 1, "the zero-open pair cannot move");
}

/// An arm with no pairs reports zeros — a fact — rather than a NaN that reads
/// downstream as a broken run.
#[test]
fn an_empty_arm_is_zero_not_nan() {
    let estimate = difference_in_differences(&[]);
    assert_eq!(estimate.pairs, 0);
    assert!(estimate.did.is_finite());
}

/// The arms are estimated separately, because a tripwire and a pointer succeed
/// in opposite ways and one average answers neither.
#[test]
fn each_arm_is_estimated_on_its_own() {
    let mut trip = subject("reference_a", true, 4, 0);
    trip.role = Some(Role::Tripwire);
    let mut trip_c = subject("reference_c", false, 4, 4);
    trip_c.role = Some(Role::Tripwire);
    let mut point = subject("project_a", true, 4, 4);
    point.role = Some(Role::Pointer);
    let mut point_c = subject("project_c", false, 4, 4);
    point_c.role = Some(Role::Pointer);

    let matching = match_on_pre_opens(&[trip, trip_c, point, point_c]);
    let arms: std::collections::BTreeMap<_, _> = by_arm(&matching)
        .into_iter()
        .map(|(arm, e)| (arm.label(), e))
        .collect();
    assert_eq!(arms["reference_"].did, -4.0);
    assert_eq!(arms["project_"].did, 0.0);
    assert_eq!(arms["role: tripwire"].did, -4.0);
    assert_eq!(arms["role: pointer"].did, 0.0);
}

// ── The pre-harvest diagnostics: a null band, and the placebo that says whether
// ── the estimate means anything at all.

use memview::study::{Null, pair_differences, placebo, sign_flip_null, subjects_at};

#[test]
fn the_null_band_is_the_same_on_every_machine_and_every_run() {
    let diffs = vec![-3.0, 1.0, 0.0, 2.0, -1.0, 4.0, -2.0];
    let a = sign_flip_null(&diffs, 4000, 20260831);
    let b = sign_flip_null(&diffs, 4000, 20260831);
    assert_eq!(a, b, "a published band must be re-derivable, not trusted");
    assert!(
        a.hi > 0.0 && a.lo < 0.0,
        "a varied sample must give a real band, got {a:?}"
    );
}

#[test]
fn the_sampled_band_is_the_exact_permutation_distribution() {
    // 7 pairs is 2^7 = 128 possible sign patterns, so the true null can be
    // enumerated and the sampler checked against it rather than against itself.
    // ⚠ This is also why two seeds return the SAME band at this size: 4000 draws
    // saturate 128 outcomes. Seed-independence here is the sampler working, and
    // a test asserting the seeds disagree would have been asserting a defect.
    let diffs = vec![-3.0, 1.0, 0.0, 2.0, -1.0, 4.0, -2.0];
    let n = diffs.len();
    let mut exact: Vec<f64> = (0..1u32 << n)
        .map(|mask| {
            diffs
                .iter()
                .enumerate()
                .map(|(i, d)| if mask >> i & 1 == 1 { *d } else { -*d })
                .sum::<f64>()
                / n as f64
        })
        .collect();
    exact.sort_by(f64::total_cmp);
    let lo = exact[exact.len() * 25 / 1000];
    let hi = exact[exact.len() * 975 / 1000];

    let sampled = sign_flip_null(&diffs, 4000, 1);
    assert!(
        (sampled.lo - lo).abs() < 0.15 && (sampled.hi - hi).abs() < 0.15,
        "sampled {sampled:?} should recover the exact band [{lo}, {hi}]"
    );
    assert_eq!(
        sign_flip_null(&diffs, 4000, 2),
        sampled,
        "saturated at this size"
    );
}

#[test]
fn an_effect_larger_than_the_noise_falls_outside_the_band() {
    // Every pair moved the same way by 5 — no amount of sign-flipping reproduces
    // a mean that large, so this must be called distinguishable.
    let diffs = vec![5.0; 40];
    let null = sign_flip_null(&diffs, 4000, 7);
    assert!(
        !null.covers(5.0),
        "a unanimous shift must not read as noise: {null:?}"
    );
    assert!(
        null.covers(0.0),
        "zero must always be inside a symmetric null band"
    );
}

#[test]
fn noise_alone_falls_inside_the_band() {
    let diffs: Vec<f64> = (0..40)
        .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
        .collect();
    let null = sign_flip_null(&diffs, 4000, 7);
    assert!(null.covers(0.0), "{null:?}");
}

#[test]
fn an_empty_pairing_gives_a_degenerate_band_rather_than_a_nan() {
    let null = sign_flip_null(&[], 4000, 1);
    assert_eq!(
        null,
        Null {
            lo: 0.0,
            hi: 0.0,
            draws: 0
        }
    );
}

#[test]
fn subjects_are_measured_in_the_window_they_are_asked_for() {
    let reads = |name: &str, lo: i64, hi: i64| -> u32 {
        // one open on day 100 for `a`, one on day 200 for `b`
        let day = if name == "a" { 100 } else { 200 };
        u32::from(lo <= day && day < hi)
    };
    let role = |_: &str| None;
    let s = subjects_at(&["a".into()], &["b".into()], 150, 60, &reads, &role);
    assert_eq!(s.len(), 2);
    let a = s.iter().find(|s| s.name == "a").expect("a");
    let b = s.iter().find(|s| s.name == "b").expect("b");
    assert!(a.treated && !b.treated);
    assert_eq!((a.pre, a.post), (1, 0), "a's open is before the fake t");
    assert_eq!((b.pre, b.post), (0, 1), "b's open is after it");
}

#[test]
fn a_placebo_on_arms_that_trend_together_finds_nothing() {
    // Both arms open on exactly the same days, so no fake t can separate them.
    let opens =
        |_: &str, lo: i64, hi: i64| -> u32 { (lo..hi).filter(|d| d % 3 == 0).count() as u32 };
    let role = |_: &str| None;
    let treated: Vec<String> = (0..30).map(|i| format!("t{i}")).collect();
    let control: Vec<String> = (0..30).map(|i| format!("c{i}")).collect();
    let out = placebo(&treated, &control, &[100, 120], 28, &opens, &role, 5);
    assert_eq!(out.len(), 2);
    for p in &out {
        assert!(!p.flags_an_effect(), "parallel arms must not flag: {p:?}");
    }
}

#[test]
fn a_placebo_catches_arms_that_were_already_diverging() {
    // The treated arm's opens decay before any treatment exists. This is the
    // real corpus's shape and the reason the diagnostic was written: the
    // procedure must SAY so rather than report the decay as an effect.
    let opens = |name: &str, lo: i64, hi: i64| -> u32 {
        if name.starts_with('c') {
            return (lo..hi).filter(|d| d % 3 == 0).count() as u32;
        }
        (lo..hi).filter(|d| d % 3 == 0 && *d < 110).count() as u32
    };
    let role = |_: &str| None;
    let treated: Vec<String> = (0..30).map(|i| format!("t{i}")).collect();
    let control: Vec<String> = (0..30).map(|i| format!("c{i}")).collect();
    let out = placebo(&treated, &control, &[100], 28, &opens, &role, 5);
    assert!(
        out[0].flags_an_effect(),
        "a pre-existing divergence must flag: {:?}",
        out[0]
    );
    assert!(
        out[0].estimate.did < 0.0,
        "and it must be the falling direction"
    );
}

#[test]
fn the_band_describes_the_same_numbers_the_estimate_averages() {
    let subjects = vec![
        subject("t1", true, 2, 0),
        subject("t2", true, 2, 1),
        subject("c1", false, 2, 2),
        subject("c2", false, 2, 3),
    ];
    let m = memview::study::match_on_pre_opens(&subjects);
    let pairs: Vec<&memview::study::Pair> = m.pairs.iter().collect();
    let diffs = pair_differences(&pairs);
    let e = memview::study::difference_in_differences(&pairs);
    let mean = diffs.iter().sum::<f64>() / diffs.len() as f64;
    assert!((mean - e.did).abs() < 1e-9, "{mean} vs {}", e.did);
}
