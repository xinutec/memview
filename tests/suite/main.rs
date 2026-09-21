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
