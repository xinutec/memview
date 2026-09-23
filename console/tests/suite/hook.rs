//! The hook every `Bash` call on this Mac runs through.

use std::io::Write as _;
use std::process::{Command, Stdio};

#[test]
fn a_console_that_cannot_be_reached_blocks_the_call() {
    // Exit 2 is Claude Code's "block this call"; any other failure lets it run.
    let mut hook = Command::new(env!("CARGO_BIN_EXE_console"))
        .arg("hook")
        // Nothing listens on port 9: the console is down.
        .env("CONSOLE_DESK_ADDR", "127.0.0.1:9")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the console binary starts");
    hook.stdin
        .take()
        .expect("stdin")
        .write_all(br#"{"hook_event_name":"PreToolUse"}"#)
        .expect("written");
    let ended = hook.wait_with_output().expect("it ends");
    assert_eq!(ended.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&ended.stderr).contains("could not be reached"));
}
