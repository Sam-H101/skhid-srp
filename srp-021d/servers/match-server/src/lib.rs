//! The 0.21d match server (`docs/notes/match.md`).

pub mod game;
mod server;
pub mod snapshot;
pub mod supplies;
pub mod transport;

pub use server::run;
