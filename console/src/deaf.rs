//! What a session that has stopped reading looked like, written down before it is
//! restarted — the cure (stop and resume) destroys the evidence.
//!
//! * `sample` — where the main thread is; the signature is every sample parked in `kevent64`.
//! * `lsof` — whether fd 0 is still an open pipe.
//! * `ps` — CPU total, an idle loop against work.
//! * the tail of the transcript — what it had just done.
//!
//! Nothing here needs root. Whether stdin is still registered in the kqueue would
//! settle the cause, and needs `fs_usage` or `dtruss`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

/// How much of the transcript's tail to keep, in bytes: the last few turns.
const TAIL: u64 = 64 * 1024;

/// How long any one probe may take; `sample` runs for ten seconds.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(30);

/// Where captures go; overridable for tests.
pub fn evidence_root() -> PathBuf {
    if let Ok(set) = std::env::var("CONSOLE_DEAF_DIR") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".console")
        .join("deaf")
}

/// One probe: what to run, and what to call its output.
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

/// Capture a deaf session without root and return where it went. A failed capture
/// must not stop the session being reported.
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
        // Not `output()`, which hides the pid a zombie's origin needs.
        let Ok(probe) = tokio::process::Command::new(program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        else {
            tracing::warn!("{program} would not start for {id}");
            continue;
        };
        // Plain, not `{:?}`: read beside what `ps` prints.
        tracing::info!(
            "asking pid {} about {id} with {program}",
            probe.id().unwrap_or(0)
        );
        let said = tokio::time::timeout(PATIENCE, probe.wait_with_output()).await;
        let written = match said {
            Ok(Ok(out)) => {
                let mut body = out.stdout;
                body.extend_from_slice(&out.stderr);
                tokio::fs::write(into.join(name), body).await.err()
            }
            Ok(Err(why)) => {
                tracing::warn!("{program} ran for {id} and could not be waited for: {why}");
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

/// The last [`TAIL`] bytes of a file, from a line boundary.
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
