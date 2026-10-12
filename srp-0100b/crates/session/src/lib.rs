//! In-memory session/account store shared between the mock servers.
//!
//! It is a **mock** (see the repo `CLAUDE.md`): there is no real authentication
//! or persistence. An account is identified by a value derived from the client's
//! IP plus the email it signed in with, so the same client resolves to the same
//! account for the lifetime of the process. Each sign-in gets a fresh
//! `session_id`; the lobby (and later the match server) map that id back to the
//! shared [`Account`].
//!
//! The store lives in one process and is shared as an `Arc<SessionStore>` across
//! the server threads (see PLAN Phase 2). Accounts are handed out as
//! `Arc<Mutex<Account>>` so a mutation in the lobby is visible everywhere that
//! holds the same account.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use survarium::account::{faction_id, player_reputation, player_skill};
use survarium::player_profile;

/// Locking that survives a panic. The servers share this state across
/// threads, and a thread that panics while holding a lock poisons it; with
/// `lock().unwrap()` every server would then fail on it until srp restarts.
/// What the panicking thread left is used as it is.
pub trait LockExt<T> {
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The first `session_id` handed out. Kept at the historic value so the first
/// client in a fresh process gets the same id it always did.
const FIRST_SESSION_ID: u32 = 0x00_00_DD_00;

/// The id of an account's first bought item. The starting items have
/// `demo_item_id`s (1 000 000 + their dictionary id), so bought ones count on
/// from far above those.
pub const FIRST_BOUGHT_ITEM_ID: u32 = 2_000_000;

/// The money a new account starts with: enough for a few weapons, armour
/// and ammo at the 2013 prices; matches earn the rest.
pub const STARTING_MONEY: u32 = 20_000;

/// What a player did in one match, for the lobby to reward and charge when
/// the player is back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MatchResult {
    pub kills: u32,
    /// Batteries taken from the enemy base.
    pub steals: u32,
    /// Whether the player was still in the match when it finished.
    pub finished: bool,
    /// Rounds, medkits, bandages, anesthetics and traps used, by dict id.
    pub used: Vec<(u16, u32)>,
}

/// Full per-account state. This is the payload the lobby serves to the client
/// (and that the match server will later read for spawning). Defaults match the
/// values the lobby used to fabricate inline, so behaviour is unchanged.
pub struct Account {
    pub id: u32,
    pub name: String,
    /// The id the next bought item gets; each purchase takes one, so an id is
    /// never handed out twice.
    pub next_item_id: u32,

    pub generic_money: u32,
    pub premium_money: u32,
    pub skills_points: u8,

    pub total_experience: u32,
    pub next_level_experience: u32,
    pub prev_level_experience: u32,

    pub player_skills: [player_skill; 5],
    pub profile_contents: Vec<player_profile::raw::player_profile>, // 1..4
    pub inventory: Vec<player_profile::raw::inventory_item_instance>, // 0..
    pub reps: [player_reputation; 4],

    /// The accounts on its friend list, in the order they were added.
    pub friends: Vec<u32>,
    /// The accounts whose chat lines its client hides.
    pub ignored: Vec<u32>,
}

impl Account {
    /// Build a default account with the historic dummy contents, tagged with the
    /// given id and display name.
    pub fn new_dummy(id: u32, name: String) -> Self {
        Self {
            id,
            name,
            next_item_id: FIRST_BOUGHT_ITEM_ID,
            generic_money: STARTING_MONEY,
            premium_money: 100_000,
            skills_points: 75,
            // Level 0: the first level takes 500 experience.
            total_experience: 0,
            next_level_experience: 500,
            prev_level_experience: 0,
            player_skills: [
                player_skill {
                    skill_id: 1,
                    skill_points: 0,
                },
                player_skill {
                    skill_id: 2,
                    skill_points: 0,
                },
                player_skill {
                    skill_id: 3,
                    skill_points: 0,
                },
                player_skill {
                    skill_id: 4,
                    skill_points: 0,
                },
                player_skill {
                    skill_id: 5,
                    skill_points: 0,
                },
            ],

            profile_contents: vec![
                player_profile::raw::player_profile::new_dummy(id, 200_000, "server_profile_1"),
                player_profile::raw::player_profile::new_dummy(id, 400_000, "server_profile_2"),
                player_profile::raw::player_profile::new_dummy(id, 600_000, "server_profile_3"),
            ],

            inventory: player_profile::demo_inventory(),

            reps: [
                player_reputation {
                    faction_id: faction_id::scavengers,
                    padding: Default::default(),
                    reputation_points: 100,
                },
                player_reputation {
                    faction_id: faction_id::black_market,
                    padding: Default::default(),
                    reputation_points: 200,
                },
                player_reputation {
                    faction_id: faction_id::army,
                    padding: Default::default(),
                    reputation_points: 300,
                },
                player_reputation {
                    faction_id: faction_id::fringe_settlers,
                    padding: Default::default(),
                    reputation_points: 400,
                },
            ],

            friends: Vec::new(),
            ignored: Vec::new(),
        }
    }
}

/// How long a session may go unseen before it ends: the original login
/// server's `collect_no_ping_period` (`login_server/sources/entry_point.cpp`).
/// A signed-in client pings it every second (see the login server).
pub const SESSION_IDLE_LIMIT: Duration = Duration::from_secs(600);

/// One sign-in.
struct Session {
    account_id: u32,
    /// When the client last showed it was there: a presence ping, or a lobby,
    /// chat or match connection.
    last_seen: Instant,
    /// Lobby and chat connections open on it (see [`SessionHold`]).
    connections: u32,
    /// Signed out or idle too long, but still in a match or held by a
    /// connection: kept until neither, so a match result still reaches its
    /// account. No new connection can use it.
    ended: bool,
}

/// A lobby or chat connection on a session: the session counts as seen while
/// it is open, and doesn't end for being idle. Dropping it counts as a last
/// sighting.
pub struct SessionHold<'a> {
    store: &'a SessionStore,
    session_id: u32,
}

impl Drop for SessionHold<'_> {
    fn drop(&mut self) {
        if let Some(session) = self.store.sessions.locked().get_mut(&self.session_id) {
            session.connections = session.connections.saturating_sub(1);
            session.last_seen = Instant::now();
        }
        self.store.forget_if_done(self.session_id);
    }
}

/// Maps sign-ins to shared accounts. Cheap to clone behind an `Arc`.
///
/// Lock order: `sessions` may be taken while `matchmaker` is held, never the
/// other way round.
pub struct SessionStore {
    accounts: Mutex<HashMap<u32, Arc<Mutex<Account>>>>,
    sessions: Mutex<HashMap<u32, Session>>,
    next_session_id: AtomicU32,
    matchmaker: Mutex<Matchmaker>,
    selected_profiles: Mutex<HashMap<u32, player_profile::raw::player_profile>>,
    /// Match results not yet given to the lobby, by account.
    match_results: Mutex<HashMap<u32, Vec<MatchResult>>>,
}

impl SessionStore {
    /// A store whose matches start for a single player.
    pub fn new() -> Self {
        Self::with_queue_min_players(1)
    }

    /// A store that starts a new match only once `min_players` (1 to
    /// [`MAX_PLAYERS_PER_MATCH`]) are waiting for one; a match with room
    /// takes a player at once whatever this is.
    pub fn with_queue_min_players(min_players: u8) -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            next_session_id: AtomicU32::new(FIRST_SESSION_ID),
            matchmaker: Mutex::new(Matchmaker::new(min_players.clamp(1, MAX_PLAYERS_PER_MATCH))),
            selected_profiles: Mutex::new(HashMap::new()),
            match_results: Mutex::new(HashMap::new()),
        }
    }

    /// Resolve the account for a client, creating a default one on first sign-in.
    /// The id is derived from `(ip, email)`, so the same client maps to the same
    /// account. Other players see the display name, so an email shows only its
    /// part before the `@`; with nothing there, the name falls back to a unique
    /// value derived from the id.
    pub fn get_or_create_account(&self, ip: IpAddr, email: &str) -> u32 {
        let account_id = account_id_for(ip, email);
        self.accounts.locked().entry(account_id).or_insert_with(|| {
            let shown = email.split('@').next().unwrap_or_default();
            let name = if shown.is_empty() {
                format!("player_{account_id:08x}")
            } else {
                shown.to_string()
            };
            Arc::new(Mutex::new(Account::new_dummy(account_id, name)))
        });
        account_id
    }

    /// The account with this id, if there is one.
    pub fn account(&self, account_id: u32) -> Option<Arc<Mutex<Account>>> {
        self.accounts.locked().get(&account_id).cloned()
    }

    /// The accounts whose name contains `part`, ignoring ASCII case, by
    /// name: at most `limit`, `except` left out.
    pub fn find_accounts(&self, part: &[u8], except: u32, limit: usize) -> Vec<(u32, String)> {
        if part.is_empty() {
            return Vec::new();
        }
        let accounts: Vec<_> = self.accounts.locked().values().cloned().collect();
        let mut found: Vec<(u32, String)> = accounts
            .iter()
            .filter_map(|account| {
                let account = account.locked();
                let name = account.name.as_bytes();
                let matches = name
                    .windows(part.len())
                    .any(|window| window.eq_ignore_ascii_case(part));
                (account.id != except && matches).then(|| (account.id, account.name.clone()))
            })
            .collect();
        found.sort_by(|lhs, rhs| lhs.1.cmp(&rhs.1).then(lhs.0.cmp(&rhs.0)));
        found.truncate(limit);
        found
    }

    /// Allocate a fresh `session_id` bound to `account_id`.
    pub fn create_session(&self, account_id: u32) -> u32 {
        let session_id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        self.sessions.locked().insert(
            session_id,
            Session {
                account_id,
                last_seen: Instant::now(),
                connections: 0,
                ended: false,
            },
        );
        session_id
    }

    /// The shared account a `session_id` resolves to, if the session is known
    /// and hasn't ended.
    pub fn account_for_session(&self, session_id: u32) -> Option<Arc<Mutex<Account>>> {
        let account_id = self.live_account_id(session_id)?;
        self.accounts.locked().get(&account_id).cloned()
    }

    /// The account of a session that hasn't ended.
    fn live_account_id(&self, session_id: u32) -> Option<u32> {
        let sessions = self.sessions.locked();
        let session = sessions.get(&session_id).filter(|session| !session.ended)?;
        Some(session.account_id)
    }

    /// The account of a session, ended or not: a match result still goes to
    /// the account of a session that ended during its match.
    fn any_account_id(&self, session_id: u32) -> Option<u32> {
        Some(self.sessions.locked().get(&session_id)?.account_id)
    }

    /// A presence ping: the session is still there. `false` for a session
    /// that is unknown or has ended.
    pub fn touch(&self, session_id: u32) -> bool {
        match self.sessions.locked().get_mut(&session_id) {
            Some(session) if !session.ended => {
                session.last_seen = Instant::now();
                true
            }
            _ => false,
        }
    }

    /// Hold a session for a lobby or chat connection. `None` for a session
    /// that is unknown or has ended: the connection must be refused.
    pub fn hold(&self, session_id: u32) -> Option<SessionHold<'_>> {
        let mut sessions = self.sessions.locked();
        let session = sessions
            .get_mut(&session_id)
            .filter(|session| !session.ended)?;
        session.connections += 1;
        session.last_seen = Instant::now();
        Some(SessionHold {
            store: self,
            session_id,
        })
    }

    /// End a session: signed out, or idle too long. It leaves the queue, and
    /// no new connection can use it. A session in a match or held by a
    /// connection is kept, ended, until neither; then it is forgotten.
    /// `false` if it was unknown or had ended already.
    pub fn end_session(&self, session_id: u32) -> bool {
        match self.sessions.locked().get_mut(&session_id) {
            Some(session) if !session.ended => session.ended = true,
            _ => return false,
        }
        self.leave_queue(session_id);
        self.forget_if_done(session_id);
        true
    }

    /// End the sessions unseen for longer than `idle` at `now` which no
    /// connection holds and which aren't in a match or the queue (the
    /// original login server's collector, `query_processor.cpp`). Returns
    /// their ids.
    pub fn end_idle_sessions(&self, now: Instant, idle: Duration) -> Vec<u32> {
        let mut ended = Vec::new();
        let matchmaker = self.matchmaker.locked();
        let mut sessions = self.sessions.locked();
        for (session_id, session) in sessions.iter_mut() {
            if !session.ended
                && session.connections == 0
                && now.saturating_duration_since(session.last_seen) > idle
                && !matchmaker.assignments.contains_key(session_id)
                && !matchmaker.queue.contains(session_id)
            {
                session.ended = true;
                ended.push(*session_id);
            }
        }
        // Not in a match and not held: nothing needs them any more.
        sessions.retain(|session_id, _| !ended.contains(session_id));
        drop(sessions);
        drop(matchmaker);
        let mut selected_profiles = self.selected_profiles.locked();
        for session_id in &ended {
            selected_profiles.remove(session_id);
        }
        ended
    }

    /// How many sessions are kept, ended ones still in a match included.
    pub fn session_count(&self) -> usize {
        self.sessions.locked().len()
    }

    /// Forget an ended session nothing needs any more: no connection holds
    /// it and it isn't in a match.
    fn forget_if_done(&self, session_id: u32) {
        let matchmaker = self.matchmaker.locked();
        let mut sessions = self.sessions.locked();
        let done = !matchmaker.assignments.contains_key(&session_id)
            && sessions
                .get(&session_id)
                .is_some_and(|session| session.ended && session.connections == 0);
        if done {
            sessions.remove(&session_id);
        }
        drop(sessions);
        drop(matchmaker);
        if done {
            self.selected_profiles.locked().remove(&session_id);
        }
    }

    /// Place a session into a match, balancing the two teams, and remember the
    /// assignment. Idempotent: a session that re-readies gets the same match and
    /// team. The lobby calls this on `ReadyForMatch`; the match server will read
    /// it back via [`Self::match_assignment`] (Phase 4).
    pub fn join_match(&self, session_id: u32) -> Assignment {
        self.matchmaker.locked().join(session_id)
    }

    /// Persist the lobby's current profile edits, remember the profile selected
    /// for this match connection, and place the session in a match, or in
    /// the queue for one.
    pub fn ready_for_match(
        &self,
        session_id: u32,
        selected_profile: player_profile::raw::player_profile,
        profile_contents: &[player_profile::raw::player_profile],
    ) -> QueueState {
        self.selected_profiles
            .locked()
            .insert(session_id, selected_profile);

        if let Some(account) = self.account_for_session(session_id) {
            account.locked().profile_contents = profile_contents.to_vec();
        }

        self.matchmaker.locked().queue(session_id)
    }

    /// For a session in the queue, or placed from it and not yet told: where
    /// it stands now. `None` for any other session.
    pub fn poll_queue(&self, session_id: u32) -> Option<QueueState> {
        self.matchmaker.locked().poll(session_id)
    }

    /// The place of a session waiting in the queue, from 1.
    pub fn queue_place(&self, session_id: u32) -> Option<usize> {
        self.matchmaker.locked().place(session_id)
    }

    /// Take a session whose lobby connection ended out of the queue, and out
    /// of a match it was placed in without being told.
    pub fn leave_queue(&self, session_id: u32) {
        let mut matchmaker = self.matchmaker.locked();
        matchmaker.queue.retain(|waiting| *waiting != session_id);
        if matchmaker.untold.remove(&session_id) {
            matchmaker.leave(session_id);
        }
    }

    /// Keep what a session's player did in a match until the lobby takes it.
    pub fn record_match_result(&self, session_id: u32, result: MatchResult) {
        let Some(account_id) = self.any_account_id(session_id) else {
            return;
        };
        self.match_results
            .locked()
            .entry(account_id)
            .or_default()
            .push(result);
    }

    /// The match results of a session's account not taken yet, oldest first.
    pub fn take_match_results(&self, session_id: u32) -> Vec<MatchResult> {
        let Some(account_id) = self.live_account_id(session_id) else {
            return Vec::new();
        };
        self.match_results
            .locked()
            .remove(&account_id)
            .unwrap_or_default()
    }

    /// How many matches are being played.
    pub fn match_count(&self) -> usize {
        self.matchmaker.locked().matches.len()
    }

    /// The exact lobby profile selected by this session for its current match.
    pub fn selected_profile(&self, session_id: u32) -> Option<player_profile::raw::player_profile> {
        self.selected_profiles.locked().get(&session_id).copied()
    }

    /// The match/team a session was assigned, if it has readied up.
    pub fn match_assignment(&self, session_id: u32) -> Option<Assignment> {
        self.matchmaker.locked().assignment_for(session_id)
    }

    /// Every session placed in a match, with when it was placed: those on
    /// their way to it and those already there.
    pub fn placements(&self, match_id: u32) -> Vec<(u32, Instant)> {
        self.matchmaker.locked().placements(match_id)
    }

    /// Release a departed session's active-player slot. Account and selected
    /// profile data remain available so the same user may immediately rejoin.
    ///
    /// Leaving a match counts as a sighting, so a client without presence
    /// pings has the whole idle limit to come back to the lobby. A session
    /// that ended during the match is forgotten now.
    pub fn leave_match(&self, session_id: u32) -> Option<Assignment> {
        let left = self.matchmaker.locked().leave(session_id);
        if let Some(session) = self.sessions.locked().get_mut(&session_id) {
            session.last_seen = Instant::now();
        }
        self.forget_if_done(session_id);
        left
    }

    /// The match server's count of the player ids a match's clients still
    /// have for players who left: those ids can't be given to a newcomer, so
    /// the matchmaker counts them as taken places.
    pub fn set_held_player_ids(&self, match_id: u32, held: u8) {
        self.matchmaker.locked().set_held(match_id, held);
    }

    /// Move a session out of `full_match`, which had no player id left for
    /// it, into another match with room (a new one if none has).
    pub fn move_to_another_match(&self, session_id: u32, full_match: u32) -> Assignment {
        let mut matchmaker = self.matchmaker.locked();
        matchmaker.leave(session_id);
        matchmaker.join_except(session_id, Some(full_match))
    }
}

/// Where a session was placed: which match, and which team (`1` or `2`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub match_id: u32,
    pub team_id: u8,
}

/// Where a session stands after asking for a match.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum QueueState {
    Placed(Assignment),
    /// Waiting for enough players to start a match, at this place (from 1).
    Waiting {
        place: usize,
    },
}

/// First match id handed out. Kept at the historic constant the lobby used to
/// return, so the first/only client's match is unchanged.
const FIRST_MATCH_ID: u32 = 0x123;
/// Hard protocol/client limit: the stock game stores players in
/// `m_net_players[20]`.
pub const MAX_PLAYERS_PER_MATCH: u8 = 20;

/// Minimal mock matchmaker: fills the most populated match with a vacancy,
/// balancing teams. Departures release their assignment so the demo stays
/// drop-in/drop-out instead of accumulating permanent historical occupants.
struct Matchmaker {
    matches: Vec<Match>,
    next_match_id: u32,
    assignments: HashMap<u32, Assignment>, // session_id -> assignment
    /// How many waiting players start a new match.
    min_players: u8,
    /// Sessions waiting for a match, first come first.
    queue: Vec<u32>,
    /// Sessions placed from the queue by another player's arrival, which
    /// learn it when they next ask.
    untold: HashSet<u32>,
    /// When each assigned session was placed in its match.
    placed_at: HashMap<u32, Instant>,
}

struct Match {
    id: u32,
    team_counts: [u8; 2],
    /// Player ids its clients still have for players who left.
    held_ids: u8,
}

impl Match {
    fn player_count(&self) -> u8 {
        self.team_counts[0] + self.team_counts[1]
    }

    fn has_room(&self) -> bool {
        self.player_count().saturating_add(self.held_ids) < MAX_PLAYERS_PER_MATCH
    }
}

impl Matchmaker {
    fn new(min_players: u8) -> Self {
        Self {
            matches: Vec::new(),
            next_match_id: FIRST_MATCH_ID,
            assignments: HashMap::new(),
            min_players,
            queue: Vec::new(),
            untold: HashSet::new(),
            placed_at: HashMap::new(),
        }
    }

    /// Place a session in a match with room, or in the queue. Once the queue
    /// holds `min_players`, they all go to a new match together.
    fn queue(&mut self, session_id: u32) -> QueueState {
        self.untold.remove(&session_id);
        if let Some(assignment) = self.assignments.get(&session_id) {
            return QueueState::Placed(*assignment);
        }
        if self.matches.iter().any(Match::has_room) {
            self.queue.retain(|waiting| *waiting != session_id);
            return QueueState::Placed(self.join(session_id));
        }
        if !self.queue.contains(&session_id) {
            self.queue.push(session_id);
        }
        if self.queue.len() < usize::from(self.min_players) {
            return QueueState::Waiting {
                place: self.place(session_id).unwrap_or(1),
            };
        }
        for waiting in std::mem::take(&mut self.queue) {
            self.join(waiting);
            if waiting != session_id {
                self.untold.insert(waiting);
            }
        }
        QueueState::Placed(self.assignments[&session_id])
    }

    fn poll(&mut self, session_id: u32) -> Option<QueueState> {
        (self.queue.contains(&session_id) || self.untold.contains(&session_id))
            .then(|| self.queue(session_id))
    }

    fn place(&self, session_id: u32) -> Option<usize> {
        let index = self
            .queue
            .iter()
            .position(|waiting| *waiting == session_id)?;
        Some(index + 1)
    }

    fn join(&mut self, session_id: u32) -> Assignment {
        self.join_except(session_id, None)
    }

    fn join_except(&mut self, session_id: u32, excluded: Option<u32>) -> Assignment {
        if let Some(assignment) = self.assignments.get(&session_id) {
            return *assignment;
        }

        let match_index = self
            .matches
            .iter()
            .enumerate()
            .filter(|(_, game_match)| game_match.has_room() && Some(game_match.id) != excluded)
            .max_by(|(_, lhs), (_, rhs)| {
                lhs.player_count()
                    .cmp(&rhs.player_count())
                    .then_with(|| rhs.id.cmp(&lhs.id))
            })
            .map(|(index, _)| index)
            .unwrap_or_else(|| {
                let index = self.matches.len();
                self.matches.push(Match {
                    id: self.next_match_id,
                    team_counts: [0, 0],
                    held_ids: 0,
                });
                self.next_match_id += 1;
                index
            });

        let current = &mut self.matches[match_index];
        if !current.has_room() {
            unreachable!("match selection must return a vacancy");
        }

        // Put the new player on the smaller team (the first player goes to team 1).
        let team = usize::from(current.team_counts[0] > current.team_counts[1]);
        current.team_counts[team] += 1;

        let assignment = Assignment {
            match_id: current.id,
            team_id: team as u8 + 1,
        };
        self.assignments.insert(session_id, assignment);
        self.placed_at.insert(session_id, Instant::now());
        assignment
    }

    fn leave(&mut self, session_id: u32) -> Option<Assignment> {
        self.queue.retain(|waiting| *waiting != session_id);
        self.untold.remove(&session_id);
        self.placed_at.remove(&session_id);
        let assignment = self.assignments.remove(&session_id)?;
        if let Some(game_match) = self
            .matches
            .iter_mut()
            .find(|game_match| game_match.id == assignment.match_id)
        {
            let team = usize::from(assignment.team_id.saturating_sub(1));
            if let Some(team_count) = game_match.team_counts.get_mut(team) {
                *team_count = team_count.saturating_sub(1);
            }
        }
        self.matches
            .retain(|game_match| game_match.player_count() != 0);
        Some(assignment)
    }

    fn set_held(&mut self, match_id: u32, held: u8) {
        if let Some(game_match) = self
            .matches
            .iter_mut()
            .find(|game_match| game_match.id == match_id)
        {
            game_match.held_ids = held;
        }
    }

    fn assignment_for(&self, session_id: u32) -> Option<Assignment> {
        self.assignments.get(&session_id).copied()
    }

    fn placements(&self, match_id: u32) -> Vec<(u32, Instant)> {
        self.assignments
            .iter()
            .filter(|(_, assignment)| assignment.match_id == match_id)
            .filter_map(|(session_id, _)| Some((*session_id, *self.placed_at.get(session_id)?)))
            .collect()
    }
}

/// `SRP_QUEUE_MIN_PLAYERS`: how many waiting players start a new match (1
/// to [`MAX_PLAYERS_PER_MATCH`], default 1). Anything else is an error naming
/// the variable.
pub fn queue_min_players_from_env() -> Result<u8, String> {
    queue_min_players(std::env::var("SRP_QUEUE_MIN_PLAYERS").ok().as_deref())
}

fn queue_min_players(value: Option<&str>) -> Result<u8, String> {
    let Some(value) = value else {
        return Ok(1);
    };
    match value.trim().parse::<u8>() {
        Ok(count) if (1..=MAX_PLAYERS_PER_MATCH).contains(&count) => Ok(count),
        _ => Err(format!(
            "SRP_QUEUE_MIN_PLAYERS is `{value}`; it must be a number from 1 to \
             {MAX_PLAYERS_PER_MATCH}"
        )),
    }
}

/// Derive a stable, non-zero account id from the client IP and email. Not
/// collision-free (it's a hash), which is acceptable for a mock.
fn account_id_for(ip: IpAddr, email: &str) -> u32 {
    let mut hasher = DefaultHasher::new();
    ip.hash(&mut hasher);
    email.hash(&mut hasher);
    (hasher.finish() as u32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    #[test]
    fn same_client_resolves_to_same_account() {
        let store = SessionStore::new();
        let first = store.get_or_create_account(ip(127, 0, 0, 1), "a@b.c");
        let again = store.get_or_create_account(ip(127, 0, 0, 1), "a@b.c");
        assert_eq!(first, again);
    }

    #[test]
    fn different_email_or_ip_gives_different_account() {
        let store = SessionStore::new();
        let a = store.get_or_create_account(ip(127, 0, 0, 1), "a@b.c");
        let b = store.get_or_create_account(ip(127, 0, 0, 1), "other@b.c");
        let c = store.get_or_create_account(ip(10, 0, 0, 1), "a@b.c");
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn empty_email_is_accepted_with_fallback_name() {
        let store = SessionStore::new();
        let id = store.get_or_create_account(ip(127, 0, 0, 1), "");
        let session = store.create_session(id);
        let account = store.account_for_session(session).unwrap();
        assert_eq!(account.locked().name, format!("player_{id:08x}"));
    }

    #[test]
    fn sessions_start_at_the_historic_id_and_resolve() {
        let store = SessionStore::new();
        let id = store.get_or_create_account(ip(127, 0, 0, 1), "a@b.c");
        let first_session = store.create_session(id);
        let second_session = store.create_session(id);
        assert_eq!(first_session, FIRST_SESSION_ID);
        assert_eq!(second_session, FIRST_SESSION_ID + 1);

        let account = store.account_for_session(first_session).unwrap();
        assert_eq!(account.locked().id, id);
        assert!(store.account_for_session(0xFFFF_FFFF).is_none());
    }

    #[test]
    fn first_match_keeps_the_historic_match_and_team() {
        let store = SessionStore::new();
        let assignment = store.join_match(1);
        assert_eq!(assignment.match_id, FIRST_MATCH_ID);
        assert_eq!(assignment.team_id, 1);
    }

    #[test]
    fn two_sessions_share_a_match_on_opposite_teams() {
        let store = SessionStore::new();
        let a = store.join_match(1);
        let b = store.join_match(2);
        assert_eq!(a.match_id, b.match_id);
        assert_eq!(a.team_id, 1);
        assert_eq!(b.team_id, 2);
    }

    #[test]
    fn join_match_is_idempotent_per_session() {
        let store = SessionStore::new();
        let first = store.join_match(7);
        let again = store.join_match(7);
        assert_eq!(first, again);
        assert_eq!(store.match_assignment(7), Some(first));
        assert_eq!(store.match_assignment(999), None);
    }

    #[test]
    fn leaving_a_full_match_opens_its_slot_to_the_next_session() {
        let store = SessionStore::new();
        let first_match: Vec<_> = (0..u32::from(MAX_PLAYERS_PER_MATCH))
            .map(|session_id| store.join_match(session_id))
            .collect();
        assert!(
            first_match
                .iter()
                .all(|assignment| assignment.match_id == FIRST_MATCH_ID)
        );

        let overflow = store.join_match(100);
        assert_ne!(overflow.match_id, FIRST_MATCH_ID);
        assert_eq!(store.leave_match(7), Some(first_match[7]));
        assert_eq!(store.match_assignment(7), None);

        let replacement = store.join_match(101);
        assert_eq!(replacement.match_id, FIRST_MATCH_ID);
    }

    #[test]
    fn ready_for_match_preserves_the_selected_edited_profile() {
        let store = SessionStore::new();
        let id = store.get_or_create_account(ip(127, 0, 0, 1), "loadout@test");
        let session_id = store.create_session(id);
        let account = store.account_for_session(session_id).unwrap();
        let mut profiles = account.locked().profile_contents.clone();
        profiles[1].slots[player_profile::raw::profile_slot_enum::weapon1_slot].dict_id = 55;

        store.ready_for_match(session_id, profiles[1], &profiles);

        assert_eq!(
            store.selected_profile(session_id).unwrap().profile_id,
            profiles[1].profile_id
        );
        assert_eq!(
            store.selected_profile(session_id).unwrap().slots
                [player_profile::raw::profile_slot_enum::weapon1_slot]
                .dict_id,
            55
        );
        assert_eq!(account.locked().profile_contents, profiles);
    }

    #[test]
    fn ids_held_for_departed_players_fill_a_match() {
        let store = SessionStore::new();
        let first = store.join_match(1);
        store.set_held_player_ids(first.match_id, MAX_PLAYERS_PER_MATCH - 1);
        assert_ne!(store.join_match(2).match_id, first.match_id);

        store.set_held_player_ids(first.match_id, MAX_PLAYERS_PER_MATCH - 2);
        assert_eq!(store.join_match(3).match_id, first.match_id);
    }

    #[test]
    fn a_session_without_an_id_moves_to_another_match() {
        let store = SessionStore::new();
        let first = store.join_match(1);
        store.join_match(2);
        let moved = store.move_to_another_match(2, first.match_id);
        assert_ne!(moved.match_id, first.match_id);
        assert_eq!(store.match_assignment(2), Some(moved));
        // Its place in the first match is free again.
        assert_eq!(store.join_match(3).match_id, first.match_id);
    }

    #[test]
    fn a_name_shows_only_the_part_of_an_email_before_the_at() {
        let store = SessionStore::new();
        let name = |login: &str| {
            let id = store.get_or_create_account(ip(127, 0, 0, 1), login);
            let session = store.create_session(id);
            let account = store.account_for_session(session).unwrap();
            let name = account.locked().name.clone();
            (id, name)
        };
        assert_eq!(name("stepan@example.com").1, "stepan");
        assert_eq!(name("dummy0").1, "dummy0");
        let (id, fallback) = name("@example.com");
        assert_eq!(fallback, format!("player_{id:08x}"));
    }

    #[test]
    fn a_lock_poisoned_by_a_panic_still_opens() {
        let shared = Arc::new(Mutex::new(1));
        let holder = shared.clone();
        let _ = std::thread::spawn(move || {
            let _guard = holder.lock().unwrap();
            panic!("a server thread fails while holding the lock");
        })
        .join();
        assert!(shared.lock().is_err());
        assert_eq!(*shared.locked(), 1);
    }

    fn ready(store: &SessionStore, session_id: u32) -> QueueState {
        let profile = Account::new_dummy(1, String::new()).profile_contents[0];
        store.ready_for_match(session_id, profile, &[profile])
    }

    #[test]
    fn one_player_starts_a_match_at_once_by_default() {
        let store = SessionStore::new();
        assert!(matches!(ready(&store, 1), QueueState::Placed(_)));
        assert_eq!(store.poll_queue(1), None, "nothing to tell");
        assert_eq!(store.match_count(), 1);
    }

    #[test]
    fn a_new_match_waits_for_the_minimum_and_starts_with_all_of_them() {
        let store = SessionStore::with_queue_min_players(3);
        assert_eq!(ready(&store, 1), QueueState::Waiting { place: 1 });
        assert_eq!(ready(&store, 2), QueueState::Waiting { place: 2 });
        assert_eq!(store.poll_queue(1), Some(QueueState::Waiting { place: 1 }));
        assert_eq!(store.match_count(), 0);

        let QueueState::Placed(third) = ready(&store, 3) else {
            panic!("the third player starts the match");
        };
        // The others hear it when they next ask, once.
        assert_eq!(
            store.poll_queue(1).map(
                |state| matches!(state, QueueState::Placed(a) if a.match_id == third.match_id)
            ),
            Some(true)
        );
        assert!(matches!(store.poll_queue(2), Some(QueueState::Placed(_))));
        assert_eq!(store.poll_queue(1), None);
        assert_eq!(store.match_count(), 1);

        // A match with room takes a newcomer at once.
        assert!(matches!(ready(&store, 4), QueueState::Placed(a) if a.match_id == third.match_id));
    }

    #[test]
    fn leaving_the_queue_frees_the_place() {
        let store = SessionStore::with_queue_min_players(3);
        ready(&store, 1);
        ready(&store, 2);
        store.leave_match(1);
        assert_eq!(store.queue_place(2), Some(1));
        store.leave_queue(2);
        assert_eq!(store.poll_queue(2), None);
        assert_eq!(ready(&store, 3), QueueState::Waiting { place: 1 });
    }

    #[test]
    fn a_player_gone_before_hearing_of_its_match_leaves_it() {
        let store = SessionStore::with_queue_min_players(2);
        ready(&store, 1);
        ready(&store, 2);
        store.leave_queue(1);
        assert_eq!(store.match_assignment(1), None);
        assert!(store.match_assignment(2).is_some());
    }

    #[test]
    fn the_queue_minimum_is_checked() {
        assert_eq!(queue_min_players(None), Ok(1));
        assert_eq!(queue_min_players(Some(" 4 ")), Ok(4));
        for bad in ["0", "21", "two"] {
            let error = queue_min_players(Some(bad)).unwrap_err();
            assert!(error.contains("SRP_QUEUE_MIN_PLAYERS"), "{error}");
        }
    }

    #[test]
    fn match_results_wait_for_the_lobby_by_account() {
        let store = SessionStore::new();
        let id = store.get_or_create_account(ip(127, 0, 0, 1), "fighter");
        let session = store.create_session(id);
        let result = MatchResult {
            kills: 2,
            finished: true,
            ..MatchResult::default()
        };
        store.record_match_result(session, result.clone());
        store.record_match_result(0xFFFF_FFFF, result.clone());

        // Another sign-in of the same account gets them.
        let later = store.create_session(id);
        assert_eq!(store.take_match_results(later), [result]);
        assert!(store.take_match_results(session).is_empty());
    }

    fn signed_in(store: &SessionStore, email: &str) -> u32 {
        let account_id = store.get_or_create_account(ip(127, 0, 0, 1), email);
        store.create_session(account_id)
    }

    fn later(seconds: u64) -> Instant {
        Instant::now() + Duration::from_secs(seconds)
    }

    #[test]
    fn accounts_are_found_by_part_of_their_name() {
        let store = SessionStore::new();
        let ivan = store.get_or_create_account(ip(127, 0, 0, 1), "Ivanov@mail");
        let petr = store.get_or_create_account(ip(127, 0, 0, 1), "petrovich");
        let ivo = store.get_or_create_account(ip(127, 0, 0, 2), "ivo");
        assert_eq!(
            store.find_accounts(b"IV", 0, 10),
            [(ivan, "Ivanov".to_string()), (ivo, "ivo".to_string()),]
        );
        assert_eq!(
            store.find_accounts(b"iv", ivo, 1),
            [(ivan, "Ivanov".to_string())]
        );
        assert!(store.find_accounts(b"", 0, 10).is_empty());
        assert!(store.account(petr).is_some());
        assert!(store.account(0xBAD).is_none());
    }

    #[test]
    fn an_ended_session_is_refused_and_forgotten() {
        let store = SessionStore::new();
        let session = signed_in(&store, "leaver");
        assert!(store.touch(session));
        assert!(store.end_session(session));
        assert!(!store.end_session(session));
        assert!(!store.touch(session));
        assert!(store.hold(session).is_none());
        assert!(store.account_for_session(session).is_none());
        assert_eq!(store.session_count(), 0);
        assert!(!store.touch(0xFFFF_FFFF));
    }

    #[test]
    fn a_session_ends_after_the_idle_limit_and_not_before() {
        let store = SessionStore::new();
        let idle = signed_in(&store, "idle");
        let pinging = signed_in(&store, "pinging");
        assert!(
            store
                .end_idle_sessions(later(599), SESSION_IDLE_LIMIT)
                .is_empty()
        );

        // A ping counts from when it came.
        let limit = Duration::from_millis(50);
        std::thread::sleep(limit * 2);
        assert!(store.touch(pinging));
        assert_eq!(store.end_idle_sessions(Instant::now(), limit), [idle]);
        assert!(store.account_for_session(pinging).is_some());
        assert!(store.account_for_session(idle).is_none());
    }

    #[test]
    fn a_held_queued_or_playing_session_never_idles_out() {
        let store = SessionStore::new();
        let held = signed_in(&store, "lobby");
        let playing = signed_in(&store, "playing");
        let queued = signed_in(&store, "queued");
        let hold = store.hold(held).unwrap();
        store.join_match(playing);
        store.matchmaker.locked().queue.push(queued);
        assert!(
            store
                .end_idle_sessions(later(3600), SESSION_IDLE_LIMIT)
                .is_empty()
        );

        // Its connection's end is its last sighting.
        drop(hold);
        assert!(
            store
                .end_idle_sessions(later(599), SESSION_IDLE_LIMIT)
                .is_empty()
        );
        assert_eq!(
            store.end_idle_sessions(later(601), SESSION_IDLE_LIMIT),
            [held]
        );
    }

    #[test]
    fn a_session_that_ends_in_a_match_still_gets_its_result() {
        let store = SessionStore::new();
        let session = signed_in(&store, "quitter");
        store.join_match(session);
        assert!(store.end_session(session));
        assert!(store.account_for_session(session).is_none());
        assert_eq!(store.session_count(), 1);

        // The match server records it, then lets it go.
        store.record_match_result(session, MatchResult::default());
        store.leave_match(session);
        assert_eq!(store.session_count(), 0);
        let again = signed_in(&store, "quitter");
        assert_eq!(store.take_match_results(again), [MatchResult::default()]);
    }

    #[test]
    fn ending_a_session_takes_it_out_of_the_queue() {
        let store = SessionStore::with_queue_min_players(2);
        let first = signed_in(&store, "first");
        let second = signed_in(&store, "second");
        let profile = Account::new_dummy(1, String::new()).profile_contents[0];
        store.ready_for_match(first, profile, &[profile]);
        let _hold = store.hold(first).unwrap();
        assert!(store.end_session(first));
        assert_eq!(store.queue_place(first), None);
        // Still held by its connection, but no new one gets in.
        assert_eq!(store.session_count(), 2);
        assert!(store.hold(first).is_none());
        assert_eq!(
            store.ready_for_match(second, profile, &[profile]),
            QueueState::Waiting { place: 1 }
        );
    }

    #[test]
    fn leaving_a_match_counts_as_a_sighting() {
        let store = SessionStore::new();
        let session = signed_in(&store, "back");
        store.join_match(session);
        std::thread::sleep(Duration::from_millis(20));
        let before_leaving = Instant::now();
        store.leave_match(session);
        let idle = Duration::from_millis(10);
        assert!(
            store
                .end_idle_sessions(before_leaving + idle, idle)
                .is_empty()
        );
    }
}
