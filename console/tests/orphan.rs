//! What a spawn shape leaves behind when its timeout fires.
//!
//! #797 is a zombie under the console that #753's fix does not touch, and the
//! task's rule for it is: **do not fix by guessing which spawn**. Waiting is a
//! poor way to find out — measured over the 50 hours to 2026-08-15, the console
//! ran 17 gist sweeps with 0 failures and 3 deaf captures with 0 probe
//! timeouts, so neither suspect's failing path executed even once. Zero zombies
//! in that window says nothing about either shape.
//!
//! So force the condition instead. Each test spawns a child that outlives a
//! deliberately short timeout, drops the wait the way `tokio::time::timeout`
//! does, and then asks the process table about **that pid**. The child is
//! short-lived on purpose: an unreaped child is only *visible* as a zombie once
//! it exits, so a `sleep 30` would show nothing but a running process either
//! way.
//!
//! The two shapes are the two in the console, and they differ in one flag:
//!
//!   src/gist.rs:307   `.kill_on_drop(true)`, then `wait_with_output()`
//!   src/deaf.rs:104   `.output()` with no `kill_on_drop`
//!
//! ⚠ **[`the_detector_can_see_a_zombie`] is why the other two mean anything.**
//! A test that looks for a zombie and finds none passes just as well when it is
//! looking in the wrong place, so one case here leaks a child deliberately and
//! fails if the process table does not show it.
//!
//! ⚠ **By pid, never by count.** Cargo runs these on threads of one process, so
//! every test here shares a parent — "no zombies under us" is a question about
//! whichever tests happen to be running beside it, including the one below that
//! leaves one on purpose.
//!
//! ⚠ These assert on this machine's tokio and this platform's reaping, not on
//! documented behaviour. That is the point: the question is what happens on the
//! Mac the console runs on, and a doc sentence cannot answer it.

use std::process::Stdio;
use std::time::Duration;

/// Long enough for the child to exit and for a reaper to have had its turn.
const SETTLE: Duration = Duration::from_millis(1500);

/// Shorter than the child's life, so the timeout is what ends the wait.
const PATIENCE: Duration = Duration::from_millis(100);

/// Whether `pid` is a zombie right now.
///
/// `ps -p` and not a scan: the pid is known, and asking about one process
/// cannot accidentally answer about a neighbour's.
fn is_a_zombie(pid: u32) -> bool {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "state="])
        .output()
        .expect("ps did not run");
    String::from_utf8_lossy(&out.stdout).trim().starts_with('Z')
}

/// A child that exits on its own shortly after the timeout will have fired.
fn briefly() -> tokio::process::Command {
    let mut command = tokio::process::Command::new("sleep");
    command
        .arg("0.4")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

/// The instrument, proved: a child nobody waits for IS visible as a zombie.
///
/// Deliberately `std::process`, which has no reaper behind it — the tokio cases
/// are asking whether tokio's orphan queue runs, so the control must not use
/// it. The zombie this leaves is reaped by the kernel when the test binary
/// exits, and no other case here counts zombies, so it disturbs nothing.
#[test]
fn the_detector_can_see_a_zombie() {
    let child = std::process::Command::new("sleep")
        .arg("0.2")
        .stdout(Stdio::null())
        .spawn()
        .expect("sleep did not spawn");
    let pid = child.id();
    // Dropped without `wait`, which is what leaves it unreaped.
    drop(child);
    std::thread::sleep(SETTLE);

    assert!(
        is_a_zombie(pid),
        "pid {pid} exited unwaited-for and the process table does not call it a zombie \
         — every other case in this file is looking for something it cannot see"
    );
}

/// The gist shape: `kill_on_drop(true)`, dropped mid-`wait_with_output`.
#[tokio::test]
async fn kill_on_drop_leaves_nothing_behind() {
    let child = briefly()
        .kill_on_drop(true)
        .spawn()
        .expect("sleep did not spawn");
    let pid = child.id().expect("the child had no pid");
    let timed_out = tokio::time::timeout(PATIENCE, child.wait_with_output()).await;
    assert!(
        timed_out.is_err(),
        "the child finished inside the timeout, so nothing was dropped mid-wait"
    );

    tokio::time::sleep(SETTLE).await;
    assert!(!is_a_zombie(pid), "kill_on_drop left {pid} unreaped");
}

/// The deaf-capture shape: no `kill_on_drop`, dropped mid-wait.
///
/// `spawn` + `wait_with_output` rather than the `.output()` that `deaf.rs`
/// actually writes, because `.output()` never hands back a pid and this has to
/// name one. It is the same two steps `.output()` performs, with the same flag
/// unset, which is the thing under test.
///
/// ⚠ **If this fails, `src/deaf.rs` is #797's leak** — and the fix is the flag,
/// not a `SIGCHLD` handler, which #797 rules out for taking the exit status
/// `Session::reap` reads.
#[tokio::test]
async fn no_kill_on_drop_leaves_nothing_behind() {
    let child = briefly().spawn().expect("sleep did not spawn");
    let pid = child.id().expect("the child had no pid");
    let timed_out = tokio::time::timeout(PATIENCE, child.wait_with_output()).await;
    assert!(
        timed_out.is_err(),
        "the child finished inside the timeout, so nothing was dropped mid-wait"
    );

    tokio::time::sleep(SETTLE).await;
    assert!(!is_a_zombie(pid), "a dropped wait left {pid} unreaped");
}
