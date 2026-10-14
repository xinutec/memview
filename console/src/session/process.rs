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

/// Reap an adopted child when it ends, or it stays `<defunct>`: a blocking
/// `waitpid` per session. Not `SIGCHLD = SIG_IGN`, which would also take the exit
/// status [`Session::reap`] needs.
pub(super) fn reap_adopted(pid: u32) {
    tokio::task::spawn_blocking(move || {
        let mut status = 0;
        // SAFETY: `pid` is our child (`execve` keeps parentage); waitpid only reads.
        if unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) } < 0 {
            // ECHILD: already reaped by an earlier image.
            tracing::debug!(
                "adopted {pid} could not be waited on: {}",
                std::io::Error::last_os_error()
            );
        }
    });
}

/// Clear close-on-exec, so an upgraded image inherits the pipe, and set
/// non-blocking for tokio. False when the descriptor is gone.
pub fn keepable(fd: std::os::fd::RawFd) -> bool {
    // SAFETY: fcntl only reads and sets flags on a descriptor we own.
    unsafe {
        if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
            return false;
        }
        let flags = libc::fcntl(fd, libc::F_GETFL);
        flags != -1 && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) != -1
    }
}

/// Whether a `ps` listing shows a `claude` still running this conversation.
pub fn names_session(ps_output: &str, id: &str) -> bool {
    crate::past::words_of_claude_processes(ps_output)
        .iter()
        .any(|word| word == id)
}

/// Kill a stopped session's process once it is confirmed still that process: the
/// pid may be reused. When in doubt, leave it alive.
pub fn finish(pid: u32, id: &str) {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "args="])
        .output()
    else {
        tracing::warn!("could not ask ps about {pid}, so {id} was left alone");
        return;
    };
    if !names_session(&String::from_utf8_lossy(&output.stdout), id) {
        // The ordinary case: it exited when stdin closed.
        tracing::info!("{id} had already gone, so pid {pid} was left alone");
        return;
    }
    tracing::info!("{id} outlived its grace period — killing pid {pid}");
    // SAFETY: a pid just confirmed to be that session; ESRCH is ignored.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
}

/// When each child of `pid` started, in epoch milliseconds; `None` when it could
/// not be asked. A running tool shows only as a child.
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

/// Not askable off macOS.
#[cfg(not(target_os = "macos"))]
pub fn children_started(_pid: u32) -> Option<Vec<i64>> {
    None
}

/// When `child` started, if it is still running and still `parent`'s.
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
