//! The session processes as the operating system sees them: descriptors carried
//! across an upgrade, reaping, and the process table.

/// The three pipes to a session's process, by number — so they can outlive this
/// image. See [`crate::roster::Roster::handover`].
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Fds {
    pub stdin: std::os::fd::RawFd,
    pub stdout: std::os::fd::RawFd,
    pub stderr: std::os::fd::RawFd,
}

/// Wait on an adopted child, so the kernel can let go of it when it ends. An
/// adopted session has no [`Child`] to wait on, so without this each one that ends
/// stays `<defunct>`.
///
/// A blocking `waitpid`, one thread per adopted session. Not `SIGCHLD` =
/// `SIG_IGN`: that reaps every child and takes the exit status [`Session::reap`]
/// reads, so a clean exit would arrive as `code: None` — this file has a test
/// against it. The status is dropped: end-of-file has already declared an
/// adopted session over ([`Session::read_from`]).
pub(super) fn reap_adopted(pid: u32) {
    tokio::task::spawn_blocking(move || {
        let mut status = 0;
        // SAFETY: `pid` is a child of this process — `execve` does not change parentage
        // — and `waitpid` only reads its exit status.
        if unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) } < 0 {
            // ECHILD is the ordinary case for a session already reaped by an earlier image.
            tracing::debug!(
                "adopted {pid} could not be waited on: {}",
                std::io::Error::last_os_error()
            );
        }
    });
}

/// Take a descriptor out of close-on-exec, and make it non-blocking. Rust sets
/// `O_CLOEXEC` on every pipe it creates, so without the first half an upgraded
/// image inherits nothing; the second is tokio's requirement. False when the
/// descriptor is gone.
pub fn keepable(fd: std::os::fd::RawFd) -> bool {
    // SAFETY: fcntl on a descriptor this process owns; both calls only read or
    // set flags and cannot invalidate it.
    unsafe {
        if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
            return false;
        }
        let flags = libc::fcntl(fd, libc::F_GETFL);
        flags != -1 && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) != -1
    }
}

/// Whether a `ps` listing shows this conversation still being run. Read through
/// [`crate::past::words_of_claude_processes`], so a line that merely mentions
/// the id is not a `claude`.
pub fn names_session(ps_output: &str, id: &str) -> bool {
    crate::past::words_of_claude_processes(ps_output)
        .iter()
        .any(|word| word == id)
}

/// Kill a stopped session's process, after checking it is still that process: a
/// pid is not a handle, this one is up to thirty seconds old, and a late SIGKILL
/// at a reused pid is a fault nothing can trace. Anything unreadable is not a
/// kill — leaving a process alive is the recoverable half.
pub fn finish(pid: u32, id: &str) {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "args="])
        .output()
    else {
        tracing::warn!("could not ask ps about {pid}, so {id} was left alone");
        return;
    };
    if !names_session(&String::from_utf8_lossy(&output.stdout), id) {
        // The ordinary case: the session took its stdin closing as the exit it is.
        tracing::info!("{id} had already gone, so pid {pid} was left alone");
        return;
    }
    tracing::info!("{id} outlived its grace period — killing pid {pid}");
    // SAFETY: a kill to a pid this console started and has just confirmed is still
    // running that session; ESRCH is ignored.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
}

/// When each child of `pid` started, in epoch milliseconds. `None` when the
/// question could not be put, which is not the same as no children.
///
/// What a running tool looks like from outside: Claude Code reports nothing
/// between allowing a call and its result, but a command it runs is its child.
#[cfg(target_os = "macos")]
pub fn children_started(pid: u32) -> Option<Vec<i64>> {
    const MOST: usize = 256;
    let mut children: Vec<libc::pid_t> = vec![0; MOST];
    let room = (MOST * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    // SAFETY: the buffer is `room` bytes and the call writes at most that many.
    let listed =
        unsafe { libc::proc_listchildpids(pid as libc::pid_t, children.as_mut_ptr().cast(), room) };
    let listed = usize::try_from(listed).ok()?.min(MOST);
    Some(
        children[..listed]
            .iter()
            .filter(|&&child| child > 0)
            .filter_map(|&child| started(child, pid))
            .collect(),
    )
}

/// Elsewhere the question cannot be put. The console runs on macOS; this keeps
/// it compiling where it is only built.
#[cfg(not(target_os = "macos"))]
pub fn children_started(_pid: u32) -> Option<Vec<i64>> {
    None
}

/// When `child` started, if it is still running and still `parent`'s: a pid
/// listed a moment ago may have ended, or been reused, since.
#[cfg(target_os = "macos")]
fn started(child: libc::pid_t, parent: u32) -> Option<i64> {
    // SAFETY: an all-zero `proc_bsdinfo` is a valid value of a plain C struct.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: `info` is `size` bytes, which is what `PROC_PIDTBSDINFO` fills.
    let filled = unsafe {
        libc::proc_pidinfo(
            child,
            libc::PROC_PIDTBSDINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    (filled == size && info.pbi_ppid == parent)
        .then(|| info.pbi_start_tvsec as i64 * 1000 + info.pbi_start_tvusec as i64 / 1000)
}
