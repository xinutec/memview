//! What a session that has stopped reading looked like, written down before it
//! is restarted.
//!
//! ⚠ **The cure destroys the evidence, and it has now destroyed it twice.** On
//! 2026-08-08 the `hardware` session went deaf at 08:47 and again at 09:37; the
//! only thing that gets it reading again is a stop and a resume, and by the time
//! anyone had decided that was what had happened, the process it happened to was
//! gone. The second episode was captured by hand, which is how the one fact
//! anybody has about the failure — fd 0 was still an open pipe with 16 KB in
//! it, so nothing had closed or broken, the process simply was not draining
//! it — came to be known at all.
//!
//! So the console captures it itself, at the moment it first concludes the
//! session is deaf, and does it before offering the cure. A third episode is
//! then comparable with the first two instead of re-derived by hand.
//!
//! **What is taken, and why each:**
//!
//! * `sample` — where the main thread is. The measured signature is 8,173 of
//!   ~9,800 samples parked in `kevent64` with no work under it.
//! * `lsof` — whether fd 0 is still there and still a pipe. It was.
//! * `ps` — the CPU total, which distinguishes an idle loop from work. The deaf
//!   process burned a flat ~5%.
//! * the tail of the transcript — what the session had just done, and the
//!   `end_turn` that says it was not mid-turn.
//!
//! Nothing here needs root, which is deliberate: the one thing that would settle
//! the root cause — whether stdin is still registered in the process's kqueue —
//! needs `fs_usage` or `dtruss` and therefore does need it, and that is not a
//! thing this console is going to ask for.

use std::path::{Path, PathBuf};

/// How much of the transcript's tail to keep, in bytes. Enough for the last few
/// turns, which is what says whether the session was working when it stopped.
const TAIL: u64 = 64 * 1024;

/// How long any one probe may take. `sample` is asked for ten seconds of
/// samples, so this is that plus room to write them out; the others answer at
/// once or are not going to.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(30);

/// Where captures go. Overridable for the same reason
/// [`crate::images::images_root`] is: this one *writes*, and a test has no home
/// directory worth writing to.
pub fn evidence_root() -> PathBuf {
    if let Ok(set) = std::env::var("CONSOLE_DEAF_DIR") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".console")
        .join("deaf")
}

/// One probe: what to run, and what to call what it says.
///
/// A table rather than four call sites, so that adding one is a line and so the
/// order they are taken in is visible. `sample` first, because it is the only
/// one whose subject moves while it is being asked.
fn probes(pid: u32) -> Vec<(&'static str, &'static str, Vec<String>)> {
    let pid = pid.to_string();
    vec![
        ("sample.txt", "sample", vec![pid.clone(), "10".into()]),
        (
            "lsof.txt",
            "lsof",
            vec!["-n".into(), "-P".into(), "-p".into(), pid.clone()],
        ),
        (
            "ps.txt",
            "ps",
            vec![
                "-o".into(),
                "pid,stat,%cpu,time,etime,wchan,command".into(),
                "-p".into(),
                pid,
            ],
        ),
    ]
}

/// Capture everything that can be known about a deaf session without root, and
/// return where it was put.
///
/// **Errors are reported and not propagated.** This runs on the way to telling
/// somebody their session is stuck, and a capture that failed must not be the
/// reason they are not told — the alarm is the useful half and the evidence is
/// the half that is only useful later.
pub async fn capture(
    root: &Path,
    id: &str,
    pid: u32,
    transcript: Option<&Path>,
    stamp: &str,
) -> Option<PathBuf> {
    let into = root.join(format!("{id}-{stamp}"));
    if let Err(why) = tokio::fs::create_dir_all(&into).await {
        tracing::warn!("could not make a place to capture {id}: {why}");
        return None;
    }
    for (name, program, args) in probes(pid) {
        let said = tokio::time::timeout(
            PATIENCE,
            tokio::process::Command::new(program).args(&args).output(),
        )
        .await;
        let written = match said {
            Ok(Ok(out)) => {
                let mut body = out.stdout;
                body.extend_from_slice(&out.stderr);
                tokio::fs::write(into.join(name), body).await.err()
            }
            Ok(Err(why)) => {
                tracing::warn!("{program} would not run for {id}: {why}");
                continue;
            }
            Err(_) => {
                tracing::warn!("{program} did not answer within {PATIENCE:?} for {id}");
                continue;
            }
        };
        if let Some(why) = written {
            tracing::warn!("could not write {name} for {id}: {why}");
        }
    }
    if let Some(path) = transcript
        && let Some(tail) = tail(path).await
        && let Err(why) = tokio::fs::write(into.join("transcript-tail.jsonl"), tail).await
    {
        tracing::warn!("could not write the transcript tail for {id}: {why}");
    }
    tracing::info!("captured a deaf session to {}", into.display());
    Some(into)
}

/// The last [`TAIL`] bytes of a file, from the first line boundary inside them.
///
/// From a boundary rather than from the byte: a transcript line is JSON, and
/// half of one at the top of the file is a thing that has to be recognised and
/// skipped by whoever reads it later, at exactly the moment they are trying to
/// understand something else.
async fn tail(path: &Path) -> Option<Vec<u8>> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut file = tokio::fs::File::open(path).await.ok()?;
    let size = file.metadata().await.ok()?.len();
    let from = size.saturating_sub(TAIL);
    file.seek(std::io::SeekFrom::Start(from)).await.ok()?;
    let mut body = Vec::new();
    file.read_to_end(&mut body).await.ok()?;
    if from == 0 {
        return Some(body);
    }
    let start = body.iter().position(|byte| *byte == b'\n')?;
    Some(body[start + 1..].to_vec())
}
