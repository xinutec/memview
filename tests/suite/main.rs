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

mod access;
mod agents;
mod atomic;
mod blame;
mod bytes;
mod ceiling;
mod cites;
mod commits;
mod corpus;
mod couse;
mod created;
mod dates;
mod filing;
mod flags;
mod index_history;
mod lint;
mod mine;
mod parity;
mod rank;
mod return_to;
mod said;
mod session;
mod shadow;
mod share;
mod staged;
mod stamped;
mod static_serving;
mod study;
mod telemetry;
mod tiers;
mod world;
