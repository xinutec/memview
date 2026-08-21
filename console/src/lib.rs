//! A front end for the live Claude Code sessions on this machine.
//!
//! The crate is a library so the tests can drive the API without a socket and
//! without a real CLI; `main.rs` is the process that binds a port. See
//! `docs/agent-console.md` for the design and the threat model.

pub mod api;
pub mod attest;
pub mod config;
pub mod deaf;
pub mod gist;
pub mod images;
pub mod marks;
pub mod modes;
pub mod parse;
pub mod past;
pub mod protocol;
pub mod roster;
pub mod session;
pub mod tasks;
pub mod tls;
pub mod trace;
pub mod usage;
pub mod zombies;
