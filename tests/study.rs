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
    let arms: std::collections::BTreeMap<_, _> = by_arm(&matching).into_iter().collect();
    assert_eq!(arms["reference_"].did, -4.0);
    assert_eq!(arms["project_"].did, 0.0);
    assert_eq!(arms["role: tripwire"].did, -4.0);
    assert_eq!(arms["role: pointer"].did, 0.0);
}
