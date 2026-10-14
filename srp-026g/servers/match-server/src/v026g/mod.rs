//! The 0.26g match server (`docs/notes/match.md`,
//! `PLAN-026G.md` step 6): the sockets and the loop around
//! `survarium::v026g::match_msgs`, over the `udp-transport` crate.
//!
//! As for 0.21d, the clients simulate and the server does not: it takes
//! each client through the handshake, keeps the match clock, relays the
//! inputs, compares the clients' world hashes, gets world snapshots from
//! patched clients and starts or resyncs clients from them, and ends the
//! match. The 0.21d server (`crate::game`, `crate::server`) is the model;
//! `game.rs` here says what was carried over and what was not.
//!
//! - [`MatchRegistry`]: the lobby makes a match ([`MatchSetup`]) and gets
//!   the address to send its players to ([`MatchTicket`]);
//! - [`run`]: the match server, on the configured UDP port;
//! - [`ResultsSink`]: where each finished match's [`MatchResult`] goes.
//!
//! ```no_run
//! use std::sync::Arc;
//! use match_server::v026g::{self, MatchRegistry, MatchResult};
//!
//! let store = Arc::new(session::SessionStore::new());
//! let registry = Arc::new(MatchRegistry::from_config());
//! let (results, finished) = std::sync::mpsc::channel::<MatchResult>();
//! let server = {
//!     let (registry, store) = (registry.clone(), store.clone());
//!     std::thread::spawn(move || v026g::run(registry, store, results))
//! };
//! // The lobby: `registry.create(setup)` and the ticket to each player;
//! // `finished.recv()` for the rewards.
//! # let _ = (server, finished);
//! ```
//!
//! Settings (environment, read once at start):
//! - `SRP_BIND_HOST`, `SRP_PUBLIC_HOST`, `SRP_MATCH_PORT` (default 1236):
//!   the address, as for 0.21d (`vostok::config`);
//! - `SRP_SNAPSHOT_DIR`: where uploaded worlds are saved (default
//!   `logs/snapshots`; `off` or empty for none), as for 0.21d;
//! - `SRP_PACKET_LOG=<dir>`: the transport's raw datagram log;
//! - `SRP_LATE_INPUTS=discard|move` (default `discard`, what 0.21d does):
//!   what becomes of an input that comes after its tick is final: dropped
//!   (0xF1 / 0xF2), or moved to an open tick (0xEF / 0xF0, new in 0.26g);
//! - `SRP_RESYNC_WORLD_ONLY=on|off` (default `off`): resync a playing
//!   client with the world alone (0xE8 of the next generation, its clock
//!   left as it is) instead of the time and the world (0xE5, 0xE8);
//! - `SRP_RESTORE_LIMIT=<n>` (default 0, no limit): disconnect a client
//!   that is out of sync again after `n` resyncs in a row with no report
//!   of its that agreed in between;
//! - `SRP_MATCH_CLIENT_TIMEOUT_SECONDS` (default 10): how long a playing
//!   client may send nothing before it is taken as gone. A game window
//!   out of focus can stall for long (another server saw 58 s under
//!   Wine): raise it for tests with two clients on one PC. A client that
//!   loads its level has 60 s, or this if it is longer;
//! - test bots: no setting here. The lobby marks a match's places
//!   (`MatchPlayer::bot`) from `SRP_BOT_ACCOUNTS` (testing only;
//!   `session::v026g::bots`), and `game.rs` ("Bots") says what a bot's
//!   place is and is not;
//! - `SRP_CLOCK_CORRECTION=on|off` (default `on`): whether client clocks
//!   are brought to the match clock with 0xE7 (off: a client keeps the
//!   lag it was started with, and its inputs come that much later);
//! - `SRP_CHECK_INITIAL_WORLD=on|off` (default `on`): whether the world
//!   that starts a match is refused when it already has kills or other
//!   statistics in it;
//! - `SRP_RECORD_DIR=<dir>` (default off): every datagram of a match in
//!   and out, one file per match, kept when `SRP_RECORD_MIN_PLAYERS`
//!   (default 6, late joiners counted) have entered it; kept files over
//!   `SRP_RECORD_MAX_MB` (default 5000) together go oldest first
//!   ([`record`]).

mod game;
/// Match recordings (`SRP_RECORD_DIR`) and their reader.
pub mod record;
mod registry;
mod server;

#[cfg(test)]
mod tests;

/// The scripted 0.26g match client: what this crate's tests, `servers/srp`'s
/// end-to-end test and the test bot (`srp-bot`, `servers/srp/src/bot.rs`)
/// talk to the match server with.
pub mod scripted;
/// The name the tests know it by.
pub use scripted as test_client;

use std::path::PathBuf;
use std::time::Duration;

pub use registry::{
    CreateError, DRAW, JoinError, LiveMatch, LivePhase, LiveStanding, MatchEnd, MatchPlayer,
    MatchRegistry, MatchResult, MatchSetup, MatchTicket, PlayerResult, ResultsSink, Seat,
    static_id,
};
pub use server::{run, run_on};
pub use survarium::v026g::match_msgs::static_info::MatchProfile;

/// What becomes of an input that comes after its tick is final for the
/// others (`SRP_LATE_INPUTS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LateInputs {
    /// The owner is told to drop it (0xF1 / 0xF2), as 0.21d's 0x58. The
    /// player's keys then stay as its input before had them until its
    /// next input.
    Discard,
    /// It is moved to the first tick still open: the owner gets 0xEF /
    /// 0xF0, the others the input at the new tick, and all have the same
    /// list (PROVEN from the client's handler: `game.rs::place`). Dropped
    /// all the same where a move could not be told apart or would bring
    /// old keys back. LIKELY what the original server did (the messages
    /// exist for nothing else); not tried live, and not the default: the
    /// owner keeps 0.21d's discard, which is live-proven there.
    Move,
}

/// The waits a test may want shorter. The defaults are the 0.21d server's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Timing {
    /// A playing client that sends nothing for this long is gone.
    pub client_timeout: Duration,
    /// A client loading its level sends and acks nothing; it gets longer.
    pub loading_timeout: Duration,
    /// A made match no client has reached in this long is given up.
    pub unstarted: Duration,
    /// How often a running match asks for a fresh snapshot.
    pub refresh: Duration,
    /// Hash reports are judged once this old (match ms).
    pub hash_settle_ms: u32,
    /// A client is resynced once its hashes were wrong for this long
    /// (match ms, over its own reports) ...
    pub divergence_confirm_ms: u32,
    /// ... or for this long on the wall clock.
    pub divergence_confirm_max: Duration,
    /// The least time between two resyncs of one client.
    pub resync_interval: Duration,
    /// A mode rule that has its end reports it this long after (match ms).
    pub end_delay_ms: u32,
    /// After the end the clients get this long to hear it.
    pub linger: Duration,
    /// At the time limit, how long a last agreed snapshot is waited for.
    pub final_wait: Duration,
    /// How long the world sent to a game that has none yet is kept on its
    /// way before the sync starts over (`SRP_ENTRY_STATE_SECONDS`). A game
    /// that plays is resynced within the 1.75 s of the client's own table.
    pub entry_state: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            client_timeout: Duration::from_secs(10),
            loading_timeout: Duration::from_secs(60),
            unstarted: Duration::from_secs(120),
            refresh: Duration::from_secs(5),
            hash_settle_ms: 2_500,
            divergence_confirm_ms: 1_000,
            divergence_confirm_max: Duration::from_secs(3),
            resync_interval: Duration::from_secs(15),
            end_delay_ms: survarium::v026g::match_msgs::rules::END_DELAY_MS,
            linger: Duration::from_secs(5),
            final_wait: Duration::from_secs(7),
            entry_state: Duration::from_secs(4),
        }
    }
}

/// How the match server behaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    pub late_inputs: LateInputs,
    /// Resync a playing client with 0xE8 alone.
    pub resync_world_only: bool,
    /// Resyncs in a row without agreement after which a client is
    /// disconnected; 0 for no limit.
    pub restore_limit: u32,
    pub clock_correction: bool,
    pub check_initial_world: bool,
    /// The world goes to the clients as LZMA2 where that is smaller
    /// (`SRP_COMPRESS_STATE`).
    pub compress_state: bool,
    pub snapshot_dir: Option<PathBuf>,
    /// `SRP_RECORD_DIR`: the match recordings, if any.
    pub record: Option<record::RecordSettings>,
    pub timing: Timing,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            late_inputs: LateInputs::Discard,
            resync_world_only: false,
            restore_limit: 0,
            clock_correction: true,
            check_initial_world: true,
            compress_state: true,
            snapshot_dir: None,
            record: None,
            timing: Timing::default(),
        }
    }
}

/// Where accepted snapshots are kept unless `SRP_SNAPSHOT_DIR` says
/// otherwise: the 0.21d server's place.
const SNAPSHOT_DIR: &str = "logs/snapshots";

impl Settings {
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        Self {
            late_inputs: parse_late_inputs(var("SRP_LATE_INPUTS").as_deref()),
            resync_world_only: parse_switch(
                "SRP_RESYNC_WORLD_ONLY",
                var("SRP_RESYNC_WORLD_ONLY").as_deref(),
                false,
            ),
            restore_limit: parse_number(
                "SRP_RESTORE_LIMIT",
                var("SRP_RESTORE_LIMIT").as_deref(),
                0,
            ),
            clock_correction: parse_switch(
                "SRP_CLOCK_CORRECTION",
                var("SRP_CLOCK_CORRECTION").as_deref(),
                true,
            ),
            check_initial_world: parse_switch(
                "SRP_CHECK_INITIAL_WORLD",
                var("SRP_CHECK_INITIAL_WORLD").as_deref(),
                true,
            ),
            compress_state: parse_switch(
                "SRP_COMPRESS_STATE",
                var("SRP_COMPRESS_STATE").as_deref(),
                true,
            ),
            snapshot_dir: parse_snapshot_dir(var("SRP_SNAPSHOT_DIR").as_deref()),
            record: record::RecordSettings::from_env(),
            timing: {
                let client_timeout = Duration::from_secs(u64::from(
                    parse_number(
                        "SRP_MATCH_CLIENT_TIMEOUT_SECONDS",
                        var("SRP_MATCH_CLIENT_TIMEOUT_SECONDS").as_deref(),
                        10,
                    )
                    .max(2),
                ));
                Timing {
                    client_timeout,
                    // A client loading its level is never given less than
                    // a playing one.
                    loading_timeout: Timing::default().loading_timeout.max(client_timeout),
                    // Never under the client table's own 1.75 s. The game
                    // catches up 20 ticks for each second of it, and the
                    // ticks on the wire reach 6.35 s from the clock it
                    // waits at: a sync cuts this short where its snapshot
                    // is old (`game.rs`, `entry_lifetime`).
                    entry_state: Duration::from_secs(u64::from(
                        parse_number(
                            "SRP_ENTRY_STATE_SECONDS",
                            var("SRP_ENTRY_STATE_SECONDS").as_deref(),
                            4,
                        )
                        .clamp(2, 5),
                    )),
                    ..Timing::default()
                }
            },
        }
    }
}

fn parse_number(name: &str, value: Option<&str>, default: u32) -> u32 {
    match value.map(str::trim) {
        None | Some("") => default,
        Some(value) => value.parse().unwrap_or_else(|_| {
            log::error!("{name} is `{value}`; it must be a number");
            default
        }),
    }
}

fn parse_late_inputs(value: Option<&str>) -> LateInputs {
    match value
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        None | Some("") | Some("discard") => LateInputs::Discard,
        Some("move") => LateInputs::Move,
        Some(other) => {
            log::error!(
                "SRP_LATE_INPUTS is `{other}`; it must be `discard` or `move`. Using discard"
            );
            LateInputs::Discard
        }
    }
}

fn parse_switch(name: &str, value: Option<&str>, default: bool) -> bool {
    match value
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        None | Some("") => default,
        Some("on" | "1" | "true" | "yes") => true,
        Some("off" | "0" | "false" | "no") => false,
        Some(other) => {
            log::error!("{name} is `{other}`; it must be `on` or `off`");
            default
        }
    }
}

fn parse_snapshot_dir(value: Option<&str>) -> Option<PathBuf> {
    match value {
        Some(value) if value.trim().is_empty() || value.trim().eq_ignore_ascii_case("off") => None,
        Some(value) => Some(value.into()),
        None => Some(SNAPSHOT_DIR.into()),
    }
}
