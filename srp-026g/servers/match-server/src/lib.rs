//! The 0.21d match server (`../srp-021d/docs/notes/match.md`), and beside it
//! the 0.26g one ([`v026g`]).

pub mod game;
mod server;
pub mod snapshot;
pub mod supplies;
pub mod transport;
pub mod v026g;

pub use server::run;
