//! memview — web viewer for the Claude memory markdown corpus.

//! What the sessions *did* is read by the `reader` crate beside this one, and
//! reached as `reader::shell`, `reader::doing` and so on. It moved out because
//! the console must be able to read a command without linking a viewer; see
//! `reader/src/lib.rs` for the boundary that makes that safe.

pub mod access;
pub mod agents;
pub mod atomic;
pub mod blame;
pub mod bytes;
pub mod cites;
pub mod commits;
pub mod config;
pub mod couse;
pub mod dates;
pub mod error;
pub mod fresh;
pub mod index_history;
pub mod lint;
pub mod mine;
pub mod nextcloud;
pub mod rank;
pub mod routes;
pub mod session;
pub mod share;
pub mod stamped;
pub mod state;
pub mod store;
pub mod study;
pub mod tiers;
