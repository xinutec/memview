//! Every integration test of this crate that can share a process, in one binary.
//!
//! Cargo links a test harness per file and runs each as its own process, and
//! macOS assesses a freshly linked binary the first time it is executed — which
//! costs far more than running the tests inside it does. Declared here, the whole
//! crate's tests are one binary to assess rather than one per file.
//!
//! These run as threads of one process. A test that sets an environment
//! variable, changes the working directory, or reasons about the process table
//! by count rather than by pid contaminates its neighbours. Such a test keeps a
//! `tests/*.rs` of its own, and with it its own process.

mod activity;
mod concept;
mod doing;
mod effects;
mod javascript;
mod leaf;
mod naming;
mod oracle;
mod predict;
mod projection;
mod python;
mod python_syntax;
mod resolvable;
mod shell;
mod shell_files;
mod shell_ops;
mod sniff;
mod sql;
mod syntax;
mod transcript;
mod transcript_structure;
mod watermark;
