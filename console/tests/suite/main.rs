//! Every integration test of this crate that can share a process, in one binary.
//!
//! Cargo runs test binaries one after another and parallelises only the tests
//! WITHIN one, so a file whose tests sit on a timer holds the whole suite for as
//! long as they wait. Declaring them here instead lets that waiting overlap,
//! which is most of what this buys; linking one executable rather than many is
//! the smaller half.
//!
//! ⚠ **These run as threads of one process.** A test that sets an environment
//! variable, changes the working directory, or reasons about the process table
//! by count rather than by pid contaminates its neighbours. Such a test keeps a
//! `tests/*.rs` of its own, and with it its own process.

mod attest;
mod conversation;
mod drafts;
mod gate;
mod gist;
mod images;
mod orphan;
mod paging;
mod parse;
mod past;
mod protocol;
mod resume;
mod serving;
mod stream;
mod tasks;
mod usage;
