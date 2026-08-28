//! Where memview's own files live, as opposed to Claude Code's (#1240).

/// ⚠ **One test, not two, because env vars are process-global.** Two tests that
/// each set and clear `MEMVIEW_DIR` pass alone and race each other under
/// cargo's default thread pool — a flake that appears only under load and reads
/// as an unrelated failure. The sequence is the test.
#[test]
fn our_directory_is_not_claude_codes() {
    unsafe {
        std::env::remove_var("MEMVIEW_DIR");
        std::env::remove_var("CLAUDE_DIR");
        std::env::remove_var("PROJECTS_DIR");
        std::env::set_var("HOME", "/home/example");
    }
    // The split is the point: a path into Anthropic's tree and a path into ours
    // are different kinds of thing. One `root` serving both is how nine of our
    // artefacts ended up loose in `~/.claude`.
    assert_eq!(
        reader::home::dir(),
        std::path::Path::new("/home/example/.claude/memview")
    );
    assert_eq!(
        reader::home::claude_dir(),
        std::path::Path::new("/home/example/.claude")
    );
    assert_eq!(
        reader::home::projects_dir(),
        std::path::Path::new("/home/example/.claude/projects")
    );

    unsafe {
        std::env::set_var("MEMVIEW_DIR", "/tmp/example-memview");
    }
    assert_eq!(
        reader::home::file("agents.json"),
        std::path::Path::new("/tmp/example-memview/agents.json")
    );
    // ⚠ Overriding ours must NOT move Claude Code's — they are separate roots.
    assert_eq!(
        reader::home::claude_dir(),
        std::path::Path::new("/home/example/.claude")
    );
}
