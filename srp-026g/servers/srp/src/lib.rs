//! What the `srp` binary is made of, as a library so that its tests can
//! start the same servers in-process (`tests/v026g_end_to_end.rs`).

pub mod bot;
pub mod v026g;

/// The input script of a bot: `match_server::v026g::drive`, here since stage
/// 0 (`srp-bot --drive`).
pub use match_server::v026g::drive;
