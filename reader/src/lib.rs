//! What the agents on this machine did, read out of the text they wrote.
//!
//! Four layers, each refusing to do the next one's job: [`shell`] reads the
//! syntax, [`shell_ops`] reads the meaning, [`shell_files`] projects that onto
//! files, and [`doing`] holds what became of it. [`python`] is a second grammar
//! inside the first, because thousands of the corpus's `Bash` calls *are*
//! Python and the work is invisible without it. [`activity`] names the kinds of
//! work a call belongs to.
//!
//! ## Why this is a crate and not a module of the viewer
//!
//! There are two binaries in this workspace and they sit at different privilege
//! levels. `memview` is a read-only viewer that runs on an internet-facing
//! host; `console` spawns Claude Code processes on the root-of-truth Mac. The
//! console's own manifest states that it depends on nothing from the viewer,
//! so that no configuration of a viewer can turn into a way to execute code
//! here — and that rule is right and stays.
//!
//! It has already cost something, though. The console re-implemented transcript
//! lookup and shipped a bug the viewer had solved months earlier: a session
//! whose transcript sat beside a directory of the same name opened with no
//! history and no name, while its 119 MB of conversation sat right there. The
//! knowledge was in the same repository, one module away, and the boundary is
//! what kept it there.
//!
//! ⚠ **So the way through that boundary is a leaf, not an exception.** What
//! lives here runs nothing, expands nothing, opens no socket, serves no
//! request and touches no filesystem beyond the bytes it is handed. There is
//! nothing in it to misconfigure, which is exactly why both privilege levels
//! may link it — and why the dependency list in `Cargo.toml` is part of the
//! design rather than an implementation detail. A crate that grew an HTTP
//! client would stop being safe to share without anything failing to compile.
//!
//! ## What is deliberately NOT here
//!
//! **How much of a file to read, and how to decode it.** The two callers work
//! at opposite scales: the viewer mines gigabytes by scanning raw bytes and
//! never parsing JSON, while the console seeks the last few kilobytes of one
//! live file and decodes typed events. Either one forced into the other's shape
//! is markedly worse at its own job. So this crate owns *what the bytes mean* —
//! the vocabulary, the rules, the domains — and each caller keeps its own
//! decoder over them.
//!
//! **Anything that knows whose work it was.** Rosters, commit histories,
//! rankings and routes stay in the viewer; processes, sockets and permissions
//! stay in the console. This crate answers "what does this text mean", never
//! "who did it" or "what should happen next".

pub mod activity;
pub mod doing;
pub mod python;
pub mod shell;
pub mod shell_files;
pub mod shell_ops;
