//! The hand-off between the lobby and the match server: the lobby makes a
//! match here ([`MatchRegistry::create`]) and tells its players the address;
//! the match server finds each connecting client's place by its session and
//! hands the finished match back as a [`MatchResult`] ([`ResultsSink`]).
//!
//! The 0.21d server reads the same from `session::Matchmaking`
//! (`seat`, `formed_match`, `end_match`); this is its 0.26g counterpart, and
//! it depends on no lobby code.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use survarium::v026g::dictionary::Dictionary;
use survarium::v026g::maps::{self, MapMode};
use survarium::v026g::match_msgs::MAX_PLAYERS;
use survarium::v026g::match_msgs::players::Supplies;
use survarium::v026g::match_msgs::snapshot::PlayerStatistics;
use survarium::v026g::match_msgs::static_info::{
    MatchOptions, MatchProfile, SCORE_EVENTS, SCORE_WEIGHTS, StaticInfo, StaticInfoError,
};

/// Set in every [`static_id`], clear in every match id.
const STATIC_ID_BIT: u32 = 0x8000_0000;

/// The id the static info of match `match_id` carries: what the client logs
/// as `Match id` and seeds the match with (the anomalies, the victory
/// items, each player's object: `static_info.md`). The same for every
/// client of the match, and never a match id.
///
/// It must differ from the id the lobby's `82` gives. The client keeps the
/// id of the last static info it took for as long as it runs (cleared only
/// by the constructor, 0x3183D2), and `network_client::connect_to_match`
/// 0x21BEB0 returns at once when `82` names that id (0x21BEBC..0x21BED5,
/// PROVEN). With the two equal, as they were until 2026-10-12, a client
/// whose match connection broke took the `82` back to its match and did
/// nothing: it stayed on "waiting for the server" until it was started
/// again (seen live that day).
pub fn static_id(match_id: u32) -> u32 {
    match_id | STATIC_ID_BIT
}

/// One player of a match to be made. The roster place is the player's
/// position in [`MatchSetup::players`].
#[derive(Clone, Debug, PartialEq)]
pub struct MatchPlayer {
    /// The session the client signs its match connection with (0xD8).
    pub session_id: u32,
    pub account_id: u32,
    /// What the others see and play against. Its `team` (0 or 1) and
    /// `squad` (0 for none) are the player's team and squad in the match.
    pub profile: MatchProfile,
    /// **Testing only** (`SRP_BOT_ACCOUNTS`; false for every player
    /// otherwise): the place is a scripted client's (`srp-bot`), which
    /// cannot simulate the game. It is a player for the others (roster,
    /// 0xE9, relays), and no witness of the world: its hash reports and
    /// uploads are not taken, it is never asked for a world, and the
    /// trust and resync rules count the other clients only (`game.rs`,
    /// "Bots").
    pub bot: bool,
}

/// A match the lobby formed.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchSetup {
    /// A playable `maps_dict` id: the map, the mode and its timings
    /// (`survarium::v026g::maps`).
    pub map_id: u8,
    /// Logged by the client only.
    pub match_level: u16,
    /// The players who will really come: the roster's size is the N of the
    /// clients' start rule (the countdown starts once `floor(0.8 × N)` of
    /// them, and at least one, are in). 1 to 20.
    pub players: Vec<MatchPlayer>,
    /// The clients' score weights; `None` for the game's own.
    pub score_weights: Option<[f32; SCORE_EVENTS]>,
}

/// Where the players of a made match connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchTicket {
    /// What the lobby's `82` names. The static info carries
    /// [`static_id`] of it.
    pub match_id: u32,
    /// The match server's public host, as configured (`SRP_PUBLIC_HOST`).
    pub host: String,
    pub port: u16,
}

impl MatchTicket {
    /// `host:port`.
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Why a match could not be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreateError {
    /// No players, or more than 20.
    Roster(usize),
    /// Not a map and mode the client can play.
    Map(u8),
    /// Two places have the same session.
    DuplicateSession(u32),
    /// A session still has a place in a match that is not over.
    AlreadySeated { session_id: u32, match_id: u32 },
    /// The clients could not hold these options (too long a name, an item
    /// their dictionary lacks, ...).
    StaticInfo(StaticInfoError),
}

impl std::fmt::Display for CreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Roster(players) => write!(f, "a roster of {players}"),
            Self::Map(id) => write!(f, "map id {id} cannot be played"),
            Self::DuplicateSession(session) => {
                write!(f, "session {session:#x} has two places")
            }
            Self::AlreadySeated {
                session_id,
                match_id,
            } => write!(
                f,
                "session {session_id:#x} still has a place in match {match_id:#x}"
            ),
            Self::StaticInfo(error) => write!(f, "the match options: {error}"),
        }
    }
}

impl std::error::Error for CreateError {}

/// A place in a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seat {
    pub match_id: u32,
    /// The roster place: the index in [`MatchSetup::players`].
    pub place: u8,
}

/// A roster place as the match server keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Place {
    pub session_id: u32,
    pub account_id: u32,
    pub name: String,
    pub team: u8,
    /// [`MatchPlayer::bot`].
    pub bot: bool,
}

/// A made match, until it is over.
pub(crate) struct Formed {
    pub match_id: u32,
    pub map: &'static MapMode,
    pub match_level: u16,
    pub places: Vec<Place>,
    /// The profiles the clients were given, by place: what a snapshot's
    /// player blocks are read against.
    pub profiles: Vec<MatchProfile>,
    pub score_weights: [f32; SCORE_EVENTS],
    pub info: StaticInfo,
    pub formed_at: Instant,
}

#[derive(Default)]
struct Inner {
    matches: HashMap<u32, std::sync::Arc<Formed>>,
    seats: HashMap<u32, Seat>,
    last_id: u32,
}

/// The matches made and not yet over, shared by the lobby (which makes
/// them) and the match server (which runs them).
pub struct MatchRegistry {
    host: String,
    port: u16,
    inner: Mutex<Inner>,
}

impl MatchRegistry {
    /// A registry for a match server the clients reach at `host:port`.
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            host: host.to_owned(),
            port,
            inner: Mutex::new(Inner {
                last_id: random() | 1,
                ..Inner::default()
            }),
        }
    }

    /// The registry of the configured match server (`SRP_PUBLIC_HOST`,
    /// `SRP_MATCH_PORT`: the settings the 0.21d server has).
    pub fn from_config() -> Self {
        let config = &vostok::config::get().match_server;
        Self::new(&config.public_host, config.port)
    }

    /// The port the clients are told.
    pub fn port(&self) -> u16 {
        self.port
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        // A panic elsewhere must not stop every match.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Makes a match: the static info is built here, once, so what the
    /// clients cannot hold is refused before anyone is told to connect.
    /// The players then connect to the ticket's address with their
    /// sessions; a match no client reaches within two minutes is given up
    /// (its result says so).
    pub fn create(&self, setup: MatchSetup) -> Result<MatchTicket, CreateError> {
        self.create_at(setup, Instant::now())
    }

    pub(crate) fn create_at(
        &self,
        setup: MatchSetup,
        now: Instant,
    ) -> Result<MatchTicket, CreateError> {
        if setup.players.is_empty() || setup.players.len() > MAX_PLAYERS {
            return Err(CreateError::Roster(setup.players.len()));
        }
        let map = maps::map_mode(setup.map_id)
            .filter(|map| map.is_playable())
            .ok_or(CreateError::Map(setup.map_id))?;
        let mut inner = self.inner();
        for (place, player) in setup.players.iter().enumerate() {
            if setup.players[..place]
                .iter()
                .any(|other| other.session_id == player.session_id)
            {
                return Err(CreateError::DuplicateSession(player.session_id));
            }
            if let Some(seat) = inner.seats.get(&player.session_id) {
                return Err(CreateError::AlreadySeated {
                    session_id: player.session_id,
                    match_id: seat.match_id,
                });
            }
        }
        let mut match_id = inner.last_id;
        loop {
            // Never 0, never one in use, never a [`static_id`].
            match_id = match_id.wrapping_mul(0x9E37_79B1).wrapping_add(0x7F4A_7C15);
            if match_id != 0
                && match_id & STATIC_ID_BIT == 0
                && !inner.matches.contains_key(&match_id)
            {
                break;
            }
        }
        inner.last_id = match_id;
        let score_weights = setup.score_weights.unwrap_or(SCORE_WEIGHTS);
        let places = setup
            .players
            .iter()
            .map(|player| Place {
                session_id: player.session_id,
                account_id: player.account_id,
                name: player.profile.name.clone(),
                team: player.profile.team,
                bot: player.bot,
            })
            .collect();
        let sessions: Vec<u32> = setup
            .players
            .iter()
            .map(|player| player.session_id)
            .collect();
        let options = MatchOptions {
            players: setup
                .players
                .into_iter()
                .map(|player| player.profile)
                .collect(),
            map_id: setup.map_id,
            match_id: static_id(match_id),
            match_level: setup.match_level,
            score_weights,
        };
        let info =
            StaticInfo::new(&options, Dictionary::shared()).map_err(CreateError::StaticInfo)?;
        let profiles = options.players.clone();
        for (place, session_id) in sessions.into_iter().enumerate() {
            inner.seats.insert(
                session_id,
                Seat {
                    match_id,
                    place: place as u8,
                },
            );
        }
        inner.matches.insert(
            match_id,
            std::sync::Arc::new(Formed {
                match_id,
                map,
                match_level: setup.match_level,
                places,
                profiles,
                score_weights,
                info,
                formed_at: now,
            }),
        );
        let bots = inner.matches[&match_id]
            .places
            .iter()
            .filter(|place| place.bot)
            .count();
        log::info!(
            "Match {match_id:#x} made: map {} ({}, mode {}), {} players{}",
            map.id,
            map.name,
            map.mode,
            options.players.len(),
            if bots == 0 {
                String::new()
            } else {
                format!(", {bots} of them test bots (SRP_BOT_ACCOUNTS): no witnesses of the world")
            }
        );
        Ok(MatchTicket {
            match_id,
            host: self.host.clone(),
            port: self.port,
        })
    }

    /// The place a session has in a match that is not over.
    pub fn seat(&self, session_id: u32) -> Option<Seat> {
        self.inner().seats.get(&session_id).copied()
    }

    /// Gives up a session's place (the player will not come, or left for
    /// good): it can be put in another match. A client of it that still
    /// connects is turned away.
    pub fn release(&self, session_id: u32) -> Option<Seat> {
        self.inner().seats.remove(&session_id)
    }

    /// The account's place in the match is now `session_id`'s: the player
    /// signed in again while the match goes on, and its client signs its
    /// match connection (0xD8) with the new session. The session it was
    /// seated under has no place any more. `None`, and nothing changed,
    /// when the match is not open, the account has no place in it, or the
    /// session is seated elsewhere.
    pub fn reseat(&self, match_id: u32, account_id: u32, session_id: u32) -> Option<Seat> {
        let mut inner = self.inner();
        let place = inner
            .matches
            .get(&match_id)?
            .places
            .iter()
            .position(|place| place.account_id == account_id)?;
        let seat = Seat {
            match_id,
            place: place as u8,
        };
        if inner
            .seats
            .get(&session_id)
            .is_some_and(|held| *held != seat)
        {
            return None;
        }
        inner.seats.retain(|_, held| *held != seat);
        inner.seats.insert(session_id, seat);
        Some(seat)
    }

    /// The match has its result: its players are free for another match
    /// while it lingers for its clients to hear the end.
    pub(crate) fn free_seats(&self, match_id: u32) {
        self.inner()
            .seats
            .retain(|_, seat| seat.match_id != match_id);
    }

    /// Whether the match is made and not over.
    pub fn is_open(&self, match_id: u32) -> bool {
        self.inner().matches.contains_key(&match_id)
    }

    /// The matches made and not over.
    pub fn match_count(&self) -> usize {
        self.inner().matches.len()
    }

    pub(crate) fn formed(&self, match_id: u32) -> Option<std::sync::Arc<Formed>> {
        self.inner().matches.get(&match_id).cloned()
    }

    /// Every open match and when it was made.
    pub(crate) fn matches(&self) -> Vec<(u32, Instant)> {
        self.inner()
            .matches
            .values()
            .map(|formed| (formed.match_id, formed.formed_at))
            .collect()
    }

    /// The match is over: its seats are free.
    pub(crate) fn end(&self, match_id: u32) -> Option<std::sync::Arc<Formed>> {
        let mut inner = self.inner();
        inner.seats.retain(|_, seat| seat.match_id != match_id);
        inner.matches.remove(&match_id)
    }
}

/// A number that differs from run to run, for the first match id.
fn random() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    hasher.finish() as u32
}

/// Why a match was over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchEnd {
    /// The mode's own rule: a team brought every battery home (modes 2 and
    /// 3), or has no tickets and nobody alive (mode 4).
    ModeRule,
    /// The match's time ran out, or it waited its whole waiting time for
    /// players without starting ([`MatchResult::played_ms`] is then
    /// `None`).
    TimeLimit,
    /// Everyone who was in it left.
    Deserted,
    /// No client ever connected.
    NoShow,
}

/// The standing value for "no team": a draw.
pub const DRAW: u8 = survarium::v026g::match_msgs::rules::DRAW;

/// One roster place of a finished match.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerResult {
    /// The roster place.
    pub place: u8,
    pub session_id: u32,
    pub account_id: u32,
    pub name: String,
    pub team: u8,
    /// Whether the player was ever in the match (0xE9 went out for it).
    /// One whose game never got that far neither won nor lost.
    pub entered: bool,
    /// Whether it was in the match when it ended.
    pub present_at_end: bool,
    /// Whether it left (0xE0, a timeout or a disconnect) at least once.
    pub left: bool,
    /// How long it was in the match, in match ms, over all its stays.
    pub time_in_match_ms: u32,
    /// Its counters in the last trusted snapshot, by event and kit
    /// (`survarium::v026g::match_msgs::snapshot::event`). `None`: no
    /// trusted snapshot had statistics.
    pub statistics: Option<PlayerStatistics>,
    /// The scoreboard's score from those counters and the match's weights;
    /// the client rounds each event as it happens, so its own last digit
    /// can differ [LIKELY].
    pub score: u32,
    /// From the kd-stats rule of that snapshot when it read, else from the
    /// counters.
    pub kills: u16,
    pub deaths: u16,
    /// The longest killstreak any trusted snapshot showed.
    pub longest_killstreak: u16,
    /// What the player had left in the last trusted snapshot: alive or
    /// not, the kit in use, and of every item of its profile what is in
    /// store, in its slots and in its weapons
    /// (`survarium::v026g::match_msgs::players`). `None`: no trusted
    /// snapshot, the player was not in it, or its block did not read.
    pub supplies: Option<Supplies>,
}

/// How a match ended, for the results screen and the rewards. Every match
/// made gives exactly one.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchResult {
    pub match_id: u32,
    pub map_id: u8,
    /// `survarium::v026g::maps::mode`.
    pub game_mode: u8,
    pub match_level: u16,
    pub end: MatchEnd,
    /// 0 or 1, [`DRAW`], or `None` when no trusted snapshot gave a
    /// standing (nobody is to be paid as a winner then).
    pub winner: Option<u8>,
    /// Batteries home (modes 2, 3) or tickets left (mode 4), from the last
    /// trusted snapshot.
    pub team_points: [u16; 2],
    /// The match clock when it ended, in ms.
    pub duration_ms: u32,
    /// How long it was played after the countdown; `None` if play never
    /// began.
    pub played_ms: Option<u32>,
    /// The match time of the snapshot the standing and the statistics are
    /// from. A snapshot is trusted when its CRC is what the clients
    /// reported for its tick (two of them, or the only one in the match)
    /// and its numbers are possible.
    pub trusted_snapshot_ms: Option<u32>,
    /// How many clients vouched for that snapshot: 2 or more, or 1 when
    /// only one client was in the match then (it could claim anything
    /// within what is possible; the rewards code may want to pay less).
    /// 0 with no trusted snapshot. A test bot ([`MatchPlayer::bot`]) is
    /// never one of them, nor one of the clients "in the match" here.
    pub trusted_by: u8,
    /// By roster place.
    pub players: Vec<PlayerResult>,
}

/// Where finished matches go: the lobby's results and rewards code.
///
/// Called on the match server's thread, once for every match made, before
/// the clients hear that the match is over (they are back in the lobby a
/// moment later and ask for the results). Keep it short, or hand the result
/// to another thread: a `std::sync::mpsc::Sender<MatchResult>` is a sink.
pub trait ResultsSink: Send {
    fn match_ended(&mut self, result: MatchResult);
}

impl ResultsSink for std::sync::mpsc::Sender<MatchResult> {
    fn match_ended(&mut self, result: MatchResult) {
        // Nobody listening is the listener's choice.
        let _ = self.send(result);
    }
}

impl<F: FnMut(MatchResult) + Send> ResultsSink for F {
    fn match_ended(&mut self, result: MatchResult) {
        self(result);
    }
}
