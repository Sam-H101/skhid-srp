//! Experimental `GrowingRoster` uses `matchmaking_policy`: two minutes from
//! the second queued player, no arrival extensions, full balanced matches early.
//! Whole squads are selected together, team sizes differ by at most one, and
//! team-average levels are balanced without a level-range admission restriction.
//! Standard matchmaking below retains its existing rules.
//!
//! 0.21d matchmaking. A 0.21d match has a fixed roster: every client is sent
//! the whole player list (0x51) before it loads, and a player not on it can
//! never join (`resources/notes/v021d/match_start_spec.md`). So players wait
//! in the queue for a gathering window, and then everyone waiting goes to a
//! new match together, up to its 20 places.
//! - A match needs two parties: two players alone, or a squad and at least
//!   one player who is not of it (alone, or of another squad). The window
//!   starts when the second party joins; a player or a squad alone waits
//!   for as long as it takes, and if the queue drops back to one party,
//!   the window starts over.
//! - A queue that looks at levels (0.26g's, [`Matchmaking::poll_with`]) is
//!   stricter, and its rules are, for the players:
//!   1. A full match starts at once.
//!   2. 10 or more waiting: it starts when nobody has pressed Play for 30 s.
//!   3. 4 to 9 waiting: a countdown of 60 s starts when the queue gets its
//!      4th player; each time somebody presses Play, 30 s are added to what
//!      is left of it.
//!   4. No countdown runs longer than 2 minutes from its start.
//!   5. Fewer than 4 players, or only one side (one player alone, one squad
//!      alone), wait; if the queue drops under 4 the countdown is cancelled
//!      and starts anew at the 4th.
//! - A squad queues as one group and plays on one team, so a squad alone
//!   would have nobody against it: it is one party, whatever its size.
//!   (Until 2026-10-12 the count was of players, and two squadmates alone
//!   got a match with nobody in it but them.)
//! - A seat is kept for the whole match, so a player whose game or
//!   connection broke can come back to it, also from a new sign-in. A
//!   player who left the match by its own will (its client says so on the
//!   match link: [`Matchmaking::give_up_seat`]) has given the seat up:
//!   Play queues it for another match. (Until 2026-10-12 Play sent it back
//!   to the match it had just left, for as long as that match ran.)

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::LockExt;
use crate::matchmaking_policy::{self as policy, Team};

/// `player_profile[20]` in the client's `match_options`.
pub const MAX_PLAYERS_PER_MATCH: usize = 20;
/// The first match id handed out.
const FIRST_MATCH_ID: u32 = 0x123;
/// With this many waiting (5 v 5), a match starts once nobody has joined the
/// queue for [`DEFAULT_QUIET_PERIOD`], without waiting out the whole window.
pub const DEFAULT_FILL_PLAYERS: usize = 10;
pub const DEFAULT_QUIET_PERIOD: Duration = Duration::from_secs(30);
/// How long the queue gathers players by default.
pub const DEFAULT_GATHER_WINDOW: Duration = Duration::from_secs(60);
/// A queue that looks at levels (0.26g's) starts no match with fewer players
/// than this (`SRP_MATCH_MIN_PLAYERS`).
pub const DEFAULT_MIN_PLAYERS: usize = 4;
/// Each time somebody presses Play while the countdown runs, that much is
/// added to what is left of it (`SRP_MATCH_JOIN_EXTENSION_SECONDS`).
pub const DEFAULT_JOIN_EXTENSION: Duration = Duration::from_secs(30);
/// No countdown runs longer than this from its start
/// (`SRP_MATCH_MAX_COUNTDOWN_SECONDS`).
pub const DEFAULT_MAX_COUNTDOWN: Duration = Duration::from_secs(120);

/// A player who pressed Play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Player {
    pub session_id: u32,
    pub account_id: u32,
    /// The profile it plays with.
    pub profile_id: u32,
}

/// A roster place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seat {
    pub player: Player,
    /// 0 or 1.
    pub team: u8,
    /// Players of one squad share a non-zero id within the match.
    pub squad: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormedMatch {
    pub id: u32,
    pub pool: QueuePool,
    pub fixed_teams: bool,
    /// The roster: a seat's index is the player's index in the match.
    pub seats: Vec<Seat>,
}

/// Where a player who pressed Play stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueState {
    /// `place` counts from 1 over the players waiting.
    Waiting {
        place: usize,
        total: usize,
        since: Instant,
    },
    Placed {
        match_id: u32,
        team: u8,
        index: usize,
    },
}

/// The levels a player is looked for a match at before it has waited
/// (0.26g: `match_maker_defines.items_levels_modifiers`' `item_level_min`
/// and `item_level_max` of its row; `crate::v026g::matchmaking`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Level {
    pub min: u8,
    pub max: u8,
}

/// The rules of a queue that looks at levels (0.26g's). Without them
/// ([`Matchmaking::poll`]) a match takes whoever waits, up to
/// [`MAX_PLAYERS_PER_MATCH`], as 0.21d's does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelRules {
    /// The most players of a side; a match has two sides.
    pub team_size: usize,
    /// A waiting player's range of levels grows by one at each end every
    /// `step` it has waited (what the 0.26g client shows as its search
    /// range: 0x225655). Zero: levels do not matter.
    pub step: Duration,
}

impl LevelRules {
    /// The most players of a match.
    pub fn match_size(&self) -> usize {
        (self.team_size.max(1) * 2).min(MAX_PLAYERS_PER_MATCH)
    }

    /// How many levels the range has grown by after `waited`; `None` when
    /// levels do not matter.
    fn widened(&self, waited: Duration) -> Option<u128> {
        (!self.step.is_zero()).then(|| waited.as_millis() / self.step.as_millis().max(1))
    }
}

/// Clients from different pools never share a newly formed match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum QueuePool {
    #[default]
    Standard,
    GrowingRoster,
}

/// An immutable queue observation; admission checks it again before committing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedGroup {
    pub players: Vec<Player>,
    /// By player; empty in a queue without levels.
    levels: Vec<Level>,
    pub squad: bool,
    pub since: Instant,
    pub pool: QueuePool,
}

impl QueuedGroup {
    pub fn population(&self) -> Team {
        Team {
            players: self.players.len(),
            level_sum: (0..self.players.len())
                .map(|i| u32::from(self.player_level(i)))
                .sum(),
        }
    }

    /// The group is looked for a match by its highest player [LIKELY: the
    /// client takes the highest of an account's kits the same way].
    fn level(&self) -> Level {
        self.levels
            .iter()
            .copied()
            .max_by_key(|level| (level.min, level.max))
            .unwrap_or_default()
    }

    /// Does this group's widened search range include an existing match?
    pub fn accepts_level(&self, level: u8, now: Instant, rules: &LevelRules) -> bool {
        let own = self.level();
        let distance = u128::from(own.min.saturating_sub(level))
            + u128::from(level.saturating_sub(own.max.max(own.min)));
        rules
            .widened(now.saturating_duration_since(self.since))
            .is_none_or(|widened| distance <= widened)
    }

    /// The level of a player, 0 without levels.
    pub fn player_level(&self, index: usize) -> u8 {
        self.levels.get(index).map_or(0, |level| level.min)
    }

    /// How many parties it is: a squad is one (its players came together
    /// and play on one team), every other player one of its own.
    fn parties(&self) -> usize {
        if self.squad { 1 } else { self.players.len() }
    }
}

struct Countdown {
    since: Instant,
    extra: Duration,
}

struct Inner {
    growing_gather: Duration,
    population_exponent: f64,
    gather: Duration,
    fill_players: usize,
    quiet: Duration,
    /// The fewest players a leveled queue starts a match with.
    min_players: usize,
    /// What a press of Play adds to the countdown left.
    extension: Duration,
    /// The longest a countdown runs.
    max_countdown: Duration,
    /// When someone last pressed Play.
    last_joined: Option<Instant>,
    pool_last_joined: HashMap<QueuePool, Instant>,
    pool_countdowns: HashMap<QueuePool, Countdown>,
    /// When the queue got its second party (a player alone or a squad:
    /// [`QueuedGroup::parties`]); `None` while it has fewer.
    gathering_since: Option<Instant>,
    next_match_id: u32,
    queue: Vec<QueuedGroup>,
    matches: HashMap<u32, Vec<Seat>>,
    match_pools: HashMap<u32, QueuePool>,
    fixed_teams: HashSet<u32>,
    formed_at: HashMap<u32, Instant>,
    /// The seated sessions a lobby connection has sent to their match. A
    /// seat is sent by whichever connection the session has, also one made
    /// after the one that pressed Play dropped.
    told: HashSet<u32>,
    /// The seats given up ([`Matchmaking::give_up_seat`]), by match and
    /// account. The place stays in the roster, which is fixed.
    given_up: HashSet<(u32, u32)>,
}

pub struct Matchmaking {
    inner: Mutex<Inner>,
}

impl Matchmaking {
    pub fn new(gather: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner {
                growing_gather: env_number("SRP_GROWING_GATHER_SECONDS")
                    .map_or(policy::GATHER_WINDOW, Duration::from_secs),
                population_exponent: env_number::<f64>("SRP_GROWING_POPULATION_EXPONENT")
                    .filter(|p| p.is_finite() && *p > 0.0 && *p <= 8.0)
                    .unwrap_or(1.0),
                gather,
                fill_players: fill_from_env(),
                quiet: quiet_from_env(),
                min_players: env_number("SRP_MATCH_MIN_PLAYERS").unwrap_or(DEFAULT_MIN_PLAYERS),
                extension: env_number("SRP_MATCH_JOIN_EXTENSION_SECONDS")
                    .map_or(DEFAULT_JOIN_EXTENSION, Duration::from_secs),
                max_countdown: env_number("SRP_MATCH_MAX_COUNTDOWN_SECONDS")
                    .map_or(DEFAULT_MAX_COUNTDOWN, Duration::from_secs),
                last_joined: None,
                pool_last_joined: HashMap::new(),
                pool_countdowns: HashMap::new(),
                gathering_since: None,
                next_match_id: FIRST_MATCH_ID,
                queue: Vec::new(),
                matches: HashMap::new(),
                match_pools: HashMap::new(),
                fixed_teams: HashSet::new(),
                formed_at: HashMap::new(),
                told: HashSet::new(),
                given_up: HashSet::new(),
            }),
        }
    }

    /// Configure the experimental pool independently of standard matchmaking.
    pub fn set_growing_rules(&self, gather: Duration, population_exponent: f64) {
        assert!(
            population_exponent.is_finite()
                && population_exponent > 0.0
                && population_exponent <= 8.0
        );
        let mut inner = self.inner.locked();
        inner.growing_gather = gather;
        inner.population_exponent = population_exponent;
    }

    pub fn population_exponent(&self) -> f64 {
        self.inner.locked().population_exponent
    }

    /// The start rules of the leveled queue ([`Self::poll_with`]) in place
    /// of the environment's: the fewest players for a match, what a press
    /// of Play adds to the countdown left, and the longest a countdown runs.
    pub fn with_start_rules(
        self,
        min_players: usize,
        extension: Duration,
        longest: Duration,
    ) -> Self {
        self.set_start_rules(min_players, extension, longest);
        self
    }

    /// [`Self::with_start_rules`] on a queue already made.
    pub fn set_start_rules(&self, min_players: usize, extension: Duration, longest: Duration) {
        let mut inner = self.inner.locked();
        inner.min_players = min_players;
        inner.extension = extension;
        inner.max_countdown = longest;
    }

    /// Players press Play: one alone, or a whole squad (`squad`) once all
    /// its members are ready. A player with a seat in a match being played
    /// goes back to it instead, under its new session if it signed in again.
    /// A squad is one party, and a match needs two: a squad alone in the
    /// queue waits for somebody who is not of it.
    pub fn queue(&self, players: &[Player], squad: bool, now: Instant) {
        self.queue_leveled(players, &[], squad, now);
    }

    /// [`Self::queue`] with each player's level (`levels[i]` is
    /// `players[i]`'s; a player without one has level 0), for a queue
    /// polled with [`Self::poll_with`].
    pub fn queue_leveled(&self, players: &[Player], levels: &[Level], squad: bool, now: Instant) {
        self.queue_in_pool(players, levels, squad, now, QueuePool::Standard);
    }

    pub fn queue_in_pool(
        &self,
        players: &[Player],
        levels: &[Level],
        squad: bool,
        now: Instant,
        pool: QueuePool,
    ) {
        let mut inner = self.inner.locked();
        let mut waiting = Vec::new();
        let mut waiting_levels = Vec::new();
        for (index, player) in players.iter().enumerate() {
            let before = inner
                .seat_of_account_mut(player.account_id)
                .map(|seat| std::mem::replace(&mut seat.player.session_id, player.session_id));
            match before {
                // Pressing Play sends it to its match again.
                Some(session_id) => {
                    inner.told.remove(&session_id);
                }
                None => {
                    waiting.push(*player);
                    waiting_levels.push(levels.get(index).copied().unwrap_or_default());
                }
            }
        }
        if waiting.is_empty() {
            return;
        }
        let sessions: HashSet<u32> = waiting.iter().map(|player| player.session_id).collect();
        let accounts: HashSet<u32> = waiting.iter().map(|player| player.account_id).collect();
        inner.queue.retain(|group| {
            !group.players.iter().any(|player| {
                sessions.contains(&player.session_id) || accounts.contains(&player.account_id)
            })
        });
        let joined = waiting.len();
        let counting = inner.pool_countdowns.contains_key(&pool);
        if pool == QueuePool::GrowingRoster && !squad {
            for (player, level) in waiting.into_iter().zip(waiting_levels) {
                inner.queue.push(QueuedGroup {
                    players: vec![player],
                    levels: vec![level],
                    squad: false,
                    since: now,
                    pool,
                });
            }
        } else {
            inner.queue.push(QueuedGroup {
                players: waiting,
                levels: waiting_levels,
                squad,
                since: now,
                pool,
            });
        }
        if pool == QueuePool::Standard {
            inner.last_joined = Some(now);
        }
        inner.pool_last_joined.insert(pool, now);
        inner.track(now);
        let extension = inner.extension;
        if pool == QueuePool::Standard
            && counting
            && let Some(countdown) = inner.pool_countdowns.get_mut(&pool)
        {
            countdown.extra += extension;
        }
        log::info!(
            "Queue: {joined} joined{}; {} waiting",
            if squad { " as a squad" } else { "" },
            inner.waiting()
        );
    }

    /// Read waiting parties without forming new matches.
    pub fn queued_groups(&self, pool: QueuePool) -> Vec<QueuedGroup> {
        self.inner
            .locked()
            .queue
            .iter()
            .filter(|group| group.pool == pool)
            .cloned()
            .collect()
    }

    /// Commit only if the observed party still waits and the host reserves it.
    /// `reserve` runs under the queue lock and must not call matchmaking again.
    pub fn append_group(
        &self,
        match_id: u32,
        group: &QueuedGroup,
        team: u8,
        now: Instant,
        reserve: impl FnOnce(usize, &[Seat]) -> bool,
    ) -> bool {
        self.append_batch(match_id, &[(group.clone(), team)], now, reserve)
    }

    /// Reserve all selected squads and solos atomically, preserving separate squad ids.
    /// A stale queue observation or a host refusal leaves every group queued.
    pub fn append_batch(
        &self,
        match_id: u32,
        groups: &[(QueuedGroup, u8)],
        now: Instant,
        reserve: impl FnOnce(usize, &[Seat]) -> bool,
    ) -> bool {
        let mut inner = self.inner.locked();
        if groups.is_empty()
            || inner.fixed_teams.contains(&match_id)
            || inner.match_pools.get(&match_id) != Some(&QueuePool::GrowingRoster)
        {
            return false;
        }
        let Some(roster) = inner.matches.get(&match_id) else {
            return false;
        };
        let first = roster.len();
        let mut positions = Vec::new();
        let mut seats = Vec::new();
        let mut accounts = HashSet::new();
        let mut sessions = HashSet::new();
        let mut squad = roster.iter().map(|seat| seat.squad).max().unwrap_or(0);
        for (group, team) in groups {
            let Some(position) = inner.queue.iter().position(|waiting| waiting == group) else {
                return false;
            };
            if group.pool != QueuePool::GrowingRoster
                || *team > 1
                || group.players.is_empty()
                || positions.contains(&position)
            {
                return false;
            }
            positions.push(position);
            let squad_id = if group.squad {
                let Some(next) = squad.checked_add(1) else {
                    return false;
                };
                squad = next;
                squad
            } else {
                0
            };
            for &player in &group.players {
                if !accounts.insert(player.account_id)
                    || !sessions.insert(player.session_id)
                    || inner.live_seat_of_account(player.account_id).is_some()
                    || roster
                        .iter()
                        .any(|seat| seat.player.account_id == player.account_id)
                {
                    return false;
                }
                seats.push(Seat {
                    player,
                    team: *team,
                    squad: squad_id,
                });
            }
        }
        if seats.len() > MAX_PLAYERS_PER_MATCH - first || !reserve(first, &seats) {
            return false;
        }
        inner
            .matches
            .get_mut(&match_id)
            .expect("match checked under lock")
            .extend(seats);
        positions.sort_unstable();
        for position in positions.into_iter().rev() {
            inner.queue.remove(position);
        }
        inner.track(now);
        true
    }

    /// Replace changed search levels, restarting the affected parties at
    /// the back of the queue. All members of a squad restart together.
    /// Unchanged parties and seats in already formed matches are untouched.
    /// Returns the players whose search restarted.
    pub fn refresh_levels(&self, levels: &[(u32, Level)], now: Instant) -> Vec<Player> {
        let mut inner = self.inner.locked();
        let mut restarted = Vec::new();
        let mut players = Vec::new();
        let queue = std::mem::take(&mut inner.queue);
        for mut group in queue {
            let mut changed = false;
            for (player, level) in group.players.iter().zip(&mut group.levels) {
                if let Some((_, current)) = levels.iter().find(|(id, _)| *id == player.account_id)
                    && level != current
                {
                    *level = *current;
                    changed = true;
                }
            }
            if changed {
                group.since = now;
                players.extend_from_slice(&group.players);
                restarted.push(group);
            } else {
                inner.queue.push(group);
            }
        }
        if !restarted.is_empty() {
            // Nobody joined or left: keep the shared countdown, including
            // its maximum duration. Only the parties' range timers restart.
            inner.queue.extend(restarted);
        }
        players
    }

    /// A player leaves the queue, and its squad with it.
    pub fn leave_queue(&self, session_id: u32) {
        let mut inner = self.inner.locked();
        let before = inner.waiting();
        inner.queue.retain(|group| {
            !group
                .players
                .iter()
                .any(|player| player.session_id == session_id)
        });
        inner.track(Instant::now());
        let after = inner.waiting();
        if after != before {
            log::info!(
                "Queue: {} left (session {session_id:#x}); {after} waiting",
                before - after
            );
        }
    }

    /// Where a session stands now; `None` if it is neither waiting nor
    /// seated. Starts the matches whose players have gathered.
    pub fn poll(&self, session_id: u32, now: Instant) -> Option<QueueState> {
        self.poll_with(session_id, now, None)
    }

    /// [`Self::poll`], forming matches by `rules` when given: sides of at
    /// most [`LevelRules::team_size`], of players whose levels are in the
    /// range of the one the match is formed around.
    pub fn poll_with(
        &self,
        session_id: u32,
        now: Instant,
        rules: Option<&LevelRules>,
    ) -> Option<QueueState> {
        let mut inner = self.inner.locked();
        match rules {
            Some(rules) => inner.form_leveled(now, rules),
            None => inner.form(now),
        }
        if let Some((match_id, index, seat)) = inner.seat_of_session(session_id) {
            return Some(QueueState::Placed {
                match_id,
                team: seat.team,
                index,
            });
        }
        let total = inner.queue.iter().map(|group| group.players.len()).sum();
        let mut before = 0;
        for group in &inner.queue {
            if group
                .players
                .iter()
                .any(|player| player.session_id == session_id)
            {
                return Some(QueueState::Waiting {
                    place: before + 1,
                    total,
                    since: group.since,
                });
            }
            before += group.players.len();
        }
        None
    }

    /// Players waiting for matchmaking, counting every squad member separately.
    /// Reading this count does not form matches or advance the queue.
    pub fn queued_players(&self) -> usize {
        self.inner.locked().waiting()
    }

    /// How many players wait at each level (a level over the last counts
    /// as the last).
    pub fn waiting_by_level<const N: usize>(&self) -> [u16; N] {
        let inner = self.inner.locked();
        let mut counts = [0_u16; N];
        if N == 0 {
            return counts;
        }
        for group in &inner.queue {
            for index in 0..group.players.len() {
                let level = usize::from(group.player_level(index)).min(N - 1);
                counts[level] = counts[level].saturating_add(1);
            }
        }
        counts
    }

    /// Seat two given teams in a new match at once, each on its own side
    /// (a clan match: who waits for it is the lobby's to keep). `None`,
    /// and nothing changed, when a team is empty, the two do not fit a
    /// match, or one of the players has a seat in a match being played.
    /// The players leave the queue.
    pub fn seat_teams(&self, teams: [&[Player]; 2], now: Instant) -> Option<u32> {
        self.seat_teams_in_pool(teams, now, QueuePool::Standard)
    }

    pub fn seat_teams_in_pool(
        &self,
        teams: [&[Player]; 2],
        now: Instant,
        pool: QueuePool,
    ) -> Option<u32> {
        let mut inner = self.inner.locked();
        let players = || teams.iter().flat_map(|team| team.iter());
        if teams.iter().any(|team| team.is_empty())
            || players().count() > MAX_PLAYERS_PER_MATCH
            || players().any(|player| inner.seat_of_account_mut(player.account_id).is_some())
        {
            return None;
        }
        let accounts: HashSet<u32> = players().map(|player| player.account_id).collect();
        inner.queue.retain(|group| {
            !group
                .players
                .iter()
                .any(|player| accounts.contains(&player.account_id))
        });
        inner.track(now);
        let id = inner.next_match_id;
        inner.next_match_id += 1;
        let seats: Vec<Seat> = teams
            .iter()
            .enumerate()
            .flat_map(|(team, players)| {
                players.iter().map(move |player| Seat {
                    player: *player,
                    team: team as u8,
                    squad: 0,
                })
            })
            .collect();
        log::info!(
            "Match {id:#x} formed for two teams of {} and {} players",
            teams[0].len(),
            teams[1].len()
        );
        inner.matches.insert(id, seats);
        inner.match_pools.insert(id, pool);
        inner.fixed_teams.insert(id);
        inner.formed_at.insert(id, now);
        Some(id)
    }

    /// The account's player left its match by its own will: the seat is
    /// given up, and Play queues the player for another match instead of
    /// sending it back. Its place stays in the roster (nobody else can
    /// take it). Returns the match, or `None` if the account has no seat.
    pub fn give_up_seat(&self, account_id: u32) -> Option<u32> {
        let mut inner = self.inner.locked();
        let (match_id, session_id) = {
            let (match_id, _, seat) = inner.live_seat_of_account(account_id)?;
            (match_id, seat.player.session_id)
        };
        inner.given_up.insert((match_id, account_id));
        inner.told.remove(&session_id);
        Some(match_id)
    }

    /// A lobby connection sent the session to its match.
    pub fn mark_told(&self, session_id: u32) {
        self.inner.locked().told.insert(session_id);
    }

    /// Whether the session was sent to the match it is seated in.
    pub fn is_told(&self, session_id: u32) -> bool {
        self.inner.locked().told.contains(&session_id)
    }

    /// Compatibility of an account's retained seat, if it has one.
    pub fn pool_of_account(&self, account_id: u32) -> Option<QueuePool> {
        let inner = self.inner.locked();
        let (id, _, _) = inner.live_seat_of_account(account_id)?;
        inner.match_pools.get(&id).copied()
    }

    /// The match and roster index of a session's seat.
    pub fn seat(&self, session_id: u32) -> Option<(u32, usize)> {
        let inner = self.inner.locked();
        inner
            .seat_of_session(session_id)
            .map(|(match_id, index, _)| (match_id, index))
    }

    /// A match being played, as it was formed.
    pub fn formed_match(&self, match_id: u32) -> Option<FormedMatch> {
        let inner = self.inner.locked();
        let seats = inner.matches.get(&match_id)?.clone();
        Some(FormedMatch {
            id: match_id,
            pool: inner
                .match_pools
                .get(&match_id)
                .copied()
                .unwrap_or_default(),
            fixed_teams: inner.fixed_teams.contains(&match_id),
            seats,
        })
    }

    /// The match is over: its seats are free.
    pub fn end_match(&self, match_id: u32) {
        let mut inner = self.inner.locked();
        if let Some(seats) = inner.matches.remove(&match_id) {
            for seat in seats {
                inner.told.remove(&seat.player.session_id);
            }
        }
        inner.given_up.retain(|(id, _)| *id != match_id);
        inner.formed_at.remove(&match_id);
        inner.match_pools.remove(&match_id);
        inner.fixed_teams.remove(&match_id);
    }

    /// The matches being played, with when each formed.
    pub fn matches(&self) -> Vec<(u32, Instant)> {
        let inner = self.inner.locked();
        inner.formed_at.iter().map(|(id, at)| (*id, *at)).collect()
    }

    pub fn match_count(&self) -> usize {
        self.inner.locked().matches.len()
    }

    /// The sessions waiting or seated: they must not end for being idle.
    pub fn busy_sessions(&self) -> HashSet<u32> {
        let inner = self.inner.locked();
        let waiting = inner.queue.iter().flat_map(|group| &group.players);
        let seated = inner.matches.values().flatten().map(|seat| &seat.player);
        waiting
            .chain(seated)
            .map(|player| player.session_id)
            .collect()
    }
}

impl Inner {
    /// The players waiting.
    fn waiting(&self) -> usize {
        self.queue.iter().map(|group| group.players.len()).sum()
    }

    /// The account's seat, unless it was given up.
    fn seat_of_account_mut(&mut self, account_id: u32) -> Option<&mut Seat> {
        let given_up = &self.given_up;
        self.matches
            .iter_mut()
            .filter(|(match_id, _)| !given_up.contains(&(**match_id, account_id)))
            .flat_map(|(_, seats)| seats)
            .find(|seat| seat.player.account_id == account_id)
    }

    /// [`Self::seat_of_account_mut`], with the match and the roster index.
    fn live_seat_of_account(&self, account_id: u32) -> Option<(u32, usize, &Seat)> {
        self.matches.iter().find_map(|(match_id, seats)| {
            if self.given_up.contains(&(*match_id, account_id)) {
                return None;
            }
            let index = seats
                .iter()
                .position(|seat| seat.player.account_id == account_id)?;
            Some((*match_id, index, &seats[index]))
        })
    }

    /// The session's seat, unless it was given up.
    fn seat_of_session(&self, session_id: u32) -> Option<(u32, usize, &Seat)> {
        self.matches.iter().find_map(|(match_id, seats)| {
            let index = seats.iter().position(|seat| {
                seat.player.session_id == session_id
                    && !self.given_up.contains(&(*match_id, seat.player.account_id))
            })?;
            Some((*match_id, index, &seats[index]))
        })
    }

    /// Standard countdowns require two parties and the configured minimum.
    /// Growing countdowns start at two players; formation separately requires
    /// balanced opposing teams, so a lone squad still cannot start a match.
    fn track(&mut self, now: Instant) {
        for pool in [QueuePool::Standard, QueuePool::GrowingRoster] {
            let parties: usize = self
                .queue
                .iter()
                .filter(|group| group.pool == pool)
                .map(QueuedGroup::parties)
                .sum();
            let players: usize = self
                .queue
                .iter()
                .filter(|group| group.pool == pool)
                .map(|group| group.players.len())
                .sum();
            let minimum = if pool == QueuePool::GrowingRoster {
                2
            } else {
                self.min_players
            };
            if players < minimum || (pool == QueuePool::Standard && parties < 2) {
                self.pool_countdowns.remove(&pool);
            } else {
                self.pool_countdowns.entry(pool).or_insert(Countdown {
                    since: now,
                    extra: Duration::ZERO,
                });
            }
        }
        let parties: usize = self
            .queue
            .iter()
            .filter(|group| group.pool == QueuePool::Standard)
            .map(QueuedGroup::parties)
            .sum();
        if parties < 2 {
            self.gathering_since = None;
        } else if self.gathering_since.is_none() {
            self.gathering_since = Some(now);
        }
    }

    /// Start a match for those waiting once the queue has had two parties for
    /// the gathering window, or at once when a match is full. Groups go in
    /// the order they came, as many as fit.
    fn form(&mut self, now: Instant) {
        loop {
            self.track(now);
            let Some(since) = self.gathering_since else {
                return;
            };
            let total: usize = self
                .queue
                .iter()
                .filter(|group| group.pool == QueuePool::Standard)
                .map(|group| group.players.len())
                .sum();
            let settled = total >= self.fill_players
                && self
                    .last_joined
                    .is_none_or(|joined| now.saturating_duration_since(joined) >= self.quiet);
            if now.saturating_duration_since(since) < self.gather
                && total < MAX_PLAYERS_PER_MATCH
                && !settled
            {
                return;
            }
            let mut chosen = Vec::new();
            let mut count = 0;
            for (index, group) in self.queue.iter().enumerate() {
                if group.pool != QueuePool::Standard {
                    continue;
                }
                let size = group.players.len();
                if count + size <= MAX_PLAYERS_PER_MATCH {
                    count += size;
                    chosen.push(index);
                }
            }
            // Each side needs somebody: a squad alone is one side.
            let parties: usize = chosen
                .iter()
                .map(|&index| self.queue[index].parties())
                .sum();
            if parties < 2 {
                return;
            }
            let mut taken: Vec<QueuedGroup> = chosen
                .iter()
                .rev()
                .map(|&index| self.queue.remove(index))
                .collect();
            taken.reverse();
            let id = self.next_match_id;
            self.next_match_id += 1;
            let seats = seat(taken);
            log::info!(
                "Match {id:#x} formed for {} players: {:?}",
                seats.len(),
                seats
                    .iter()
                    .map(|seat| (seat.player.session_id, seat.team))
                    .collect::<Vec<_>>()
            );
            self.matches.insert(id, seats);
            self.match_pools.insert(id, QueuePool::Standard);
            self.formed_at.insert(id, now);
            // Whoever is left waits a new window.
            self.gathering_since = None;
        }
    }

    /// [`Self::form`] by `rules`. A match is formed around a waiting group
    /// (the one that waited longest first) from the groups whose level is
    /// in its range, which starts as its own [`Level`] and grows with its
    /// wait. The conditions, counted over those groups, are the ones in the
    /// module's docs: at least two parties and `min_players` players, and
    /// either a full match, or 10 or more players and nobody pressing Play
    /// for the quiet period, or fewer and the countdown over (60 s from the
    /// `min_players`th player, a press of Play adding 30 s to what is left);
    /// no countdown runs longer than `max_countdown`. So a level only
    /// delays a match until the range has grown over it.
    fn form_leveled(&mut self, now: Instant, rules: &LevelRules) {
        self.form_growing(now, rules.team_size);
        let size = rules.match_size();
        let min_players = self.min_players.min(size);
        loop {
            self.track(now);
            let found = (0..self.queue.len()).find_map(|anchor| {
                let pool = self.queue[anchor].pool;
                if pool != QueuePool::Standard {
                    return None;
                }
                let countdown = self.pool_countdowns.get(&pool)?;
                let elapsed = now.saturating_duration_since(countdown.since);
                let longest = self.max_countdown.max(self.gather);
                let window_over = elapsed >= (self.gather + countdown.extra).min(longest);
                let capped = elapsed >= longest;
                let quiet = self
                    .pool_last_joined
                    .get(&pool)
                    .is_none_or(|joined| now.saturating_duration_since(*joined) >= self.quiet);
                let near = self.in_range_of(anchor, now, rules);
                let chosen = fitting(&self.queue, near, size, rules.team_size);
                let players: usize = chosen
                    .iter()
                    .map(|&index| self.queue[index].players.len())
                    .sum();
                let parties: usize = chosen
                    .iter()
                    .map(|&index| self.queue[index].parties())
                    .sum();
                let ready = parties >= 2
                    && players >= min_players
                    && (players >= size
                        || if players >= self.fill_players {
                            quiet || capped
                        } else {
                            window_over
                        });
                ready.then_some(chosen)
            });
            let Some(mut chosen) = found else {
                return;
            };
            let pool = self.queue[chosen[0]].pool;
            // In the order they came.
            chosen.sort_unstable();
            let mut taken: Vec<QueuedGroup> = chosen
                .iter()
                .rev()
                .map(|&index| self.queue.remove(index))
                .collect();
            taken.reverse();
            let id = self.next_match_id;
            self.next_match_id += 1;
            let seats = seat_balanced(taken, rules.team_size);
            log::info!(
                "Match {id:#x} formed for {} players: {:?}",
                seats.len(),
                seats
                    .iter()
                    .map(|seat| (seat.player.session_id, seat.team))
                    .collect::<Vec<_>>()
            );
            self.matches.insert(id, seats);
            self.match_pools.insert(id, pool);
            self.formed_at.insert(id, now);
            // Whoever is left waits a new window.
            self.gathering_since = None;
            self.pool_countdowns.remove(&pool);
        }
    }

    fn form_growing(&mut self, now: Instant, capacity: usize) {
        loop {
            self.track(now);
            let Some(countdown) = self.pool_countdowns.get(&QueuePool::GrowingRoster) else {
                return;
            };
            let order: Vec<_> = self
                .queue
                .iter()
                .enumerate()
                .filter(|(_, group)| group.pool == QueuePool::GrowingRoster)
                .map(|(index, _)| index)
                .collect();
            let waiting: usize = order
                .iter()
                .map(|&index| self.queue[index].players.len())
                .sum();
            if waiting < capacity.min(MAX_PLAYERS_PER_MATCH / 2) * 2
                && now.saturating_duration_since(countdown.since) < self.growing_gather
            {
                return;
            }
            let groups: Vec<_> = order
                .iter()
                .map(|&index| self.queue[index].population())
                .collect();
            let Some(placement) = policy::place([Team::default(); 2], &groups, capacity) else {
                return;
            };
            if placement.population() < capacity.min(MAX_PLAYERS_PER_MATCH / 2) * 2
                && now.saturating_duration_since(countdown.since) < self.growing_gather
            {
                return;
            }
            let mut seats = Vec::new();
            let mut squad = 0;
            let mut chosen = Vec::new();
            for (index, team) in placement.assignments {
                let index = order[index];
                let group = &self.queue[index];
                let squad_id = if group.squad {
                    squad += 1;
                    squad
                } else {
                    0
                };
                seats.extend(group.players.iter().map(|&player| Seat {
                    player,
                    team,
                    squad: squad_id,
                }));
                chosen.push(index);
            }
            for index in chosen.into_iter().rev() {
                self.queue.remove(index);
            }
            let id = self.next_match_id;
            self.next_match_id += 1;
            self.matches.insert(id, seats);
            self.match_pools.insert(id, QueuePool::GrowingRoster);
            self.formed_at.insert(id, now);
            self.pool_countdowns.remove(&QueuePool::GrowingRoster);
        }
    }

    /// The groups whose level is in the range of the group at `anchor`
    /// now: itself first, then the nearest levels first, and of those the
    /// ones that came first.
    fn in_range_of(&self, anchor: usize, now: Instant, rules: &LevelRules) -> Vec<usize> {
        let own = self.queue[anchor].level();
        let waited = now.saturating_duration_since(self.queue[anchor].since);
        let widened = rules.widened(waited);
        // How far outside the anchor's own levels a group is.
        let distance = |index: usize| {
            let level = u128::from(self.queue[index].level().min);
            let (min, max) = (u128::from(own.min), u128::from(own.max.max(own.min)));
            min.saturating_sub(level) + level.saturating_sub(max)
        };
        let mut near: Vec<usize> = (0..self.queue.len())
            .filter(|&index| {
                self.queue[index].pool == self.queue[anchor].pool
                    && (index == anchor || widened.is_none_or(|widened| distance(index) <= widened))
            })
            .collect();
        near.sort_by_key(|&index| (index != anchor, distance(index), index));
        near
    }
}

/// Of the groups at `order`, in that order, as many as fit a match of
/// `size` players whose two sides both fit `team_size`.
fn fitting(queue: &[QueuedGroup], order: Vec<usize>, size: usize, team_size: usize) -> Vec<usize> {
    let mut chosen = Vec::new();
    let mut sizes = Vec::new();
    for index in order {
        sizes.push(queue[index].players.len());
        if sizes.iter().sum::<usize>() <= size && sides_fit(&sizes, team_size) {
            chosen.push(index);
        } else {
            sizes.pop();
        }
    }
    chosen
}

/// Whether groups of these sizes can be split into two sides of at most
/// `team_size` players.
fn sides_fit(sizes: &[usize], team_size: usize) -> bool {
    let total: usize = sizes.iter().sum();
    if total > 2 * team_size || team_size >= 64 {
        return false;
    }
    // Bit `n`: some of the groups make a side of `n` players.
    let mut sums: u64 = 1;
    for &size in sizes {
        if size > team_size {
            return false;
        }
        sums |= sums << size;
    }
    (0..=team_size.min(total)).any(|side| sums >> side & 1 == 1 && total - side <= team_size)
}

/// [`seat`], with sides of at most `team_size` and as even as the groups
/// allow: first in players, then in the sum of their levels. The teams are
/// `seat`'s whenever no other split is better.
fn seat_balanced(groups: Vec<QueuedGroup>, team_size: usize) -> Vec<Seat> {
    let count = groups.len();
    let sizes: Vec<i64> = groups
        .iter()
        .map(|group| group.players.len() as i64)
        .collect();
    let levels: Vec<i64> = groups
        .iter()
        .map(|group| {
            (0..group.players.len())
                .map(|index| i64::from(group.player_level(index)))
                .sum()
        })
        .collect();
    // What a split is worth, the less the better: players over a side's
    // size, then the difference in players, then in levels. Bit `i` of
    // `mask`: group `i` is on team 1.
    let score = |mask: u32| {
        let mut players = [0_i64; 2];
        let mut level = [0_i64; 2];
        for index in 0..count {
            let team = (mask >> index & 1) as usize;
            players[team] += sizes[index];
            level[team] += levels[index];
        }
        let over: i64 = players
            .iter()
            .map(|&side| (side - team_size as i64).max(0))
            .sum();
        (
            over,
            (players[0] - players[1]).abs(),
            (level[0] - level[1]).abs(),
        )
    };
    let mut best = seat_teams_of(&groups)
        .iter()
        .enumerate()
        .fold(0_u32, |mask, (index, &team)| {
            mask | u32::from(team) << index
        });
    // The largest group (of those, the one that came first) stays on team
    // 0, as `seat` has it.
    let fixed = (0..count).max_by_key(|&index| (sizes[index], std::cmp::Reverse(index)));
    if count <= MAX_PLAYERS_PER_MATCH {
        let mut best_score = score(best);
        for mask in 0..1_u32 << count {
            if fixed.is_some_and(|fixed| mask >> fixed & 1 == 1) {
                continue;
            }
            let this = score(mask);
            if this < best_score {
                best = mask;
                best_score = this;
            }
        }
    }
    let teams: Vec<u8> = (0..count).map(|index| (best >> index & 1) as u8).collect();
    seats_of(groups, &teams)
}

/// [`seat`]'s teams, by group: the largest first, each on the team with
/// fewer players.
fn seat_teams_of(groups: &[QueuedGroup]) -> Vec<u8> {
    let mut by_size: Vec<usize> = (0..groups.len()).collect();
    by_size.sort_by_key(|&index| std::cmp::Reverse(groups[index].players.len()));
    let mut team_of = vec![0; groups.len()];
    let mut sizes = [0; 2];
    for index in by_size {
        let team = usize::from(sizes[1] < sizes[0]);
        team_of[index] = team as u8;
        sizes[team] += groups[index].players.len();
    }
    team_of
}

/// Teams of the groups: the largest first, each on the team with fewer
/// players. Seats keep the order the groups came in.
fn seat(groups: Vec<QueuedGroup>) -> Vec<Seat> {
    let team_of = seat_teams_of(&groups);
    seats_of(groups, &team_of)
}

/// The seats of groups on the teams given, in the order the groups came.
fn seats_of(groups: Vec<QueuedGroup>, team_of: &[u8]) -> Vec<Seat> {
    let mut seats = Vec::new();
    let mut next_squad = 0;
    for (group, &team) in groups.into_iter().zip(team_of) {
        let squad = if group.squad && group.players.len() > 1 {
            next_squad += 1;
            next_squad
        } else {
            0
        };
        seats.extend(group.players.into_iter().map(|player| Seat {
            player,
            team,
            squad,
        }));
    }
    seats
}

/// What the leveled queue (0.26g's) is set to, for the pages that tell
/// the players: the fewest players for a match, what a press of Play adds
/// to the countdown, and the longest a countdown runs.
pub fn leveled_start_rules_from_env() -> (usize, Duration, Duration) {
    (
        env_number("SRP_MATCH_MIN_PLAYERS").unwrap_or(DEFAULT_MIN_PLAYERS),
        env_number("SRP_MATCH_JOIN_EXTENSION_SECONDS")
            .map_or(DEFAULT_JOIN_EXTENSION, Duration::from_secs),
        env_number("SRP_MATCH_MAX_COUNTDOWN_SECONDS")
            .map_or(DEFAULT_MAX_COUNTDOWN, Duration::from_secs),
    )
}

/// A number from the environment; `None` if it is unset or bad.
fn env_number<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
}

/// `SRP_MATCH_FILL_PLAYERS` (default 10) and `SRP_MATCH_QUIET_SECONDS`
/// (default 30): with that many waiting and nobody joining for that long, the
/// match starts before the window ends. A bad value falls back to the default.
pub fn fill_from_env() -> usize {
    std::env::var("SRP_MATCH_FILL_PLAYERS")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(DEFAULT_FILL_PLAYERS)
}

pub fn quiet_from_env() -> Duration {
    std::env::var("SRP_MATCH_QUIET_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .map_or(DEFAULT_QUIET_PERIOD, Duration::from_secs)
}

/// `SRP_MATCH_GATHER_SECONDS`: how long the queue gathers players before a
/// match starts (default 30; 0 starts one at once). Anything else is an
/// error naming the variable.
pub fn gather_window_from_env() -> Result<Duration, String> {
    gather_window(std::env::var("SRP_MATCH_GATHER_SECONDS").ok().as_deref())
}

fn gather_window(value: Option<&str>) -> Result<Duration, String> {
    let Some(value) = value else {
        return Ok(DEFAULT_GATHER_WINDOW);
    };
    match value.trim().parse::<u16>() {
        Ok(seconds) => Ok(Duration::from_secs(seconds.into())),
        _ => Err(format!(
            "SRP_MATCH_GATHER_SECONDS is `{value}`; it must be a number of seconds"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: u32) -> Player {
        Player {
            session_id: id,
            account_id: 1000 + id,
            profile_id: 1,
        }
    }

    #[test]
    fn incompatible_pools_have_separate_rosters_and_gather_windows() {
        let mm = Matchmaking::new(Duration::from_secs(60)).with_start_rules(
            2,
            Duration::ZERO,
            Duration::from_secs(3600),
        );
        let now = Instant::now();
        mm.queue_in_pool(&[player(1)], &[at(0)], false, now, QueuePool::Standard);
        mm.queue_in_pool(&[player(2)], &[at(0)], false, now, QueuePool::GrowingRoster);
        mm.poll_with(1, after(now, 60), Some(&EIGHT));
        assert_eq!(mm.match_count(), 0);
        mm.queue_in_pool(
            &[player(3)],
            &[at(0)],
            false,
            after(now, 60),
            QueuePool::GrowingRoster,
        );
        mm.poll_with(2, after(now, 179), Some(&EIGHT));
        assert_eq!(mm.match_count(), 0);
        mm.poll_with(2, after(now, 180), Some(&EIGHT));
        let (id, _) = mm.seat(2).unwrap();
        assert_eq!(mm.seat(3).map(|seat| seat.0), Some(id));
        assert!(mm.seat(1).is_none());
        let formed = mm.formed_match(id).unwrap();
        assert_eq!(formed.pool, QueuePool::GrowingRoster);
        assert!(!formed.fixed_teams);
        assert_eq!(formed.seats.len(), 2);
    }

    #[test]
    fn growing_queue_does_not_shorten_the_legacy_gather_window() {
        let mm = Matchmaking::new(Duration::from_secs(60));
        let now = Instant::now();
        mm.queue(&[player(1), player(2)], false, now);
        let growing: Vec<_> = (3..23).map(player).collect();
        mm.queue_in_pool(&growing, &[], false, now, QueuePool::GrowingRoster);
        mm.poll(1, now);
        assert_eq!(mm.match_count(), 0);
        mm.poll(1, after(now, 60));
        assert!(mm.seat(1).is_some());
        assert!(mm.seat(3).is_none());
        assert_eq!(mm.match_count(), 1);
    }

    #[test]
    fn growing_wait_starts_at_two_and_arrivals_do_not_extend_it() {
        let mm = Matchmaking::new(Duration::from_secs(60));
        let now = Instant::now();
        mm.queue_in_pool(&[player(1)], &[at(0)], false, now, QueuePool::GrowingRoster);
        mm.poll_with(1, after(now, 300), Some(&TEN));
        assert!(mm.seat(1).is_none());
        mm.queue_in_pool(
            &[player(2)],
            &[at(100)],
            false,
            after(now, 300),
            QueuePool::GrowingRoster,
        );
        mm.queue_in_pool(
            &[player(3)],
            &[at(200)],
            false,
            after(now, 419),
            QueuePool::GrowingRoster,
        );
        mm.poll_with(1, after(now, 419), Some(&TEN));
        assert!(mm.seat(1).is_none());
        mm.poll_with(1, after(now, 420), Some(&TEN));
        let formed = mm.formed_match(mm.seat(1).unwrap().0).unwrap();
        assert_eq!(formed.seats.len(), 3);
        assert_eq!(
            formed
                .seats
                .iter()
                .filter(|s| s.team == 0)
                .count()
                .abs_diff(formed.seats.iter().filter(|s| s.team == 1).count()),
            1
        );
    }

    #[test]
    fn growing_timer_resets_below_two_and_full_balanced_matches_start_early() {
        let mm = Matchmaking::new(Duration::ZERO);
        let now = Instant::now();
        mm.queue_in_pool(
            &[player(1), player(2)],
            &[],
            false,
            now,
            QueuePool::GrowingRoster,
        );
        mm.leave_queue(2);
        mm.queue_in_pool(
            &[player(3)],
            &[],
            false,
            after(now, 100),
            QueuePool::GrowingRoster,
        );
        mm.poll_with(1, after(now, 219), Some(&TEN));
        assert!(mm.seat(1).is_none());
        mm.poll_with(1, after(now, 220), Some(&TEN));
        assert!(mm.seat(1).is_some());
        let mm = Matchmaking::new(Duration::ZERO);
        let players: Vec<_> = (1..=20).map(player).collect();
        mm.queue_in_pool(&players, &[], false, now, QueuePool::GrowingRoster);
        mm.poll_with(1, now, Some(&TEN));
        let formed = mm.formed_match(mm.seat(1).unwrap().0).unwrap();
        assert_eq!(formed.seats.iter().filter(|s| s.team == 0).count(), 10);
        assert_eq!(formed.seats.iter().filter(|s| s.team == 1).count(), 10);
    }

    #[test]
    fn gathering_expiry_does_not_start_three_against_one() {
        let mm = Matchmaking::new(Duration::ZERO);
        let now = Instant::now();
        mm.queue_in_pool(
            &[player(1), player(2), player(3)],
            &[],
            true,
            now,
            QueuePool::GrowingRoster,
        );
        mm.queue_in_pool(&[player(4)], &[], false, now, QueuePool::GrowingRoster);
        mm.poll_with(1, after(now, 120), Some(&TEN));
        assert_eq!(mm.match_count(), 0);
        mm.queue_in_pool(
            &[player(5)],
            &[],
            false,
            after(now, 121),
            QueuePool::GrowingRoster,
        );
        mm.poll_with(1, after(now, 121), Some(&TEN));
        assert_eq!(
            mm.formed_match(mm.seat(1).unwrap().0).unwrap().seats.len(),
            5
        );
    }

    #[test]
    fn appending_a_party_checks_the_queue_again_and_commits_only_after_host_success() {
        let mm = Matchmaking::new(Duration::ZERO).with_start_rules(
            2,
            Duration::ZERO,
            Duration::from_secs(3600),
        );
        mm.set_growing_rules(Duration::ZERO, 1.0);
        let now = Instant::now();
        mm.queue_in_pool(
            &[player(1), player(2)],
            &[at(0); 2],
            false,
            now,
            QueuePool::GrowingRoster,
        );
        mm.poll_with(1, now, Some(&EIGHT));
        let id = mm.seat(1).unwrap().0;
        mm.queue_in_pool(
            &[player(3), player(4)],
            &[at(0); 2],
            true,
            now,
            QueuePool::GrowingRoster,
        );
        let group = mm.queued_groups(QueuePool::GrowingRoster).pop().unwrap();
        assert!(!mm.append_group(id, &group, 0, now, |_, _| false));
        assert_eq!(mm.formed_match(id).unwrap().seats.len(), 2);
        assert_eq!(mm.queued_players(), 2);
        mm.leave_queue(3);
        assert!(!mm.append_group(id, &group, 0, now, |_, _| panic!(
            "stale party reached host"
        )));
        mm.queue_in_pool(
            &[player(3), player(4)],
            &[at(0); 2],
            true,
            after(now, 1),
            QueuePool::GrowingRoster,
        );
        let group = mm.queued_groups(QueuePool::GrowingRoster).pop().unwrap();
        assert!(mm.append_group(id, &group, 0, now, |first, seats| {
            assert_eq!(first, 2);
            assert_eq!(seats.len(), 2);
            assert_ne!(seats[0].squad, 0);
            assert_eq!(seats[0].squad, seats[1].squad);
            true
        }));
        assert_eq!(mm.seat(3), Some((id, 2)));
        assert_eq!(mm.seat(4), Some((id, 3)));
        assert_eq!(mm.queued_players(), 0);
        mm.give_up_seat(player(3).account_id);
        mm.queue_in_pool(
            &[player(3)],
            &[at(0)],
            false,
            after(now, 2),
            QueuePool::GrowingRoster,
        );
        let group = mm.queued_groups(QueuePool::GrowingRoster).pop().unwrap();
        assert!(!mm.append_group(id, &group, 0, now, |_, _| panic!(
            "duplicate account reached host"
        )));
    }

    #[test]
    fn queued_players_counts_members_without_forming_matches() {
        let mm = Matchmaking::new(Duration::ZERO);
        let now = Instant::now();
        assert_eq!(mm.queued_players(), 0);
        mm.queue(&[player(1), player(2)], true, now);
        mm.queue(&[player(3)], false, now);
        assert_eq!(mm.queued_players(), 3);
        assert_eq!(mm.queued_players(), 3);
        assert_eq!(mm.match_count(), 0);
        mm.leave_queue(3);
        assert_eq!(mm.queued_players(), 2);
        mm.queue(&[player(3)], false, now);
        let Some(QueueState::Placed { match_id, .. }) = mm.poll(3, now) else {
            panic!("match should form");
        };
        assert_eq!(mm.queued_players(), 0);
        assert_eq!(mm.match_count(), 1);
        mm.end_match(match_id);
        assert_eq!(mm.queued_players(), 0);
    }

    #[test]
    fn batches_are_atomic_and_keep_each_squads_identity() {
        let mm = Matchmaking::new(Duration::ZERO);
        mm.set_growing_rules(Duration::ZERO, 1.0);
        let now = Instant::now();
        mm.queue_in_pool(
            &[player(1), player(2), player(3), player(4), player(5)],
            &[],
            false,
            now,
            QueuePool::GrowingRoster,
        );
        mm.poll_with(1, now, Some(&TEN));
        let id = mm.seat(1).unwrap().0;
        mm.queue_in_pool(
            &[player(6), player(7)],
            &[],
            true,
            now,
            QueuePool::GrowingRoster,
        );
        mm.queue_in_pool(
            &[player(8), player(9), player(10)],
            &[],
            true,
            now,
            QueuePool::GrowingRoster,
        );
        let groups = mm.queued_groups(QueuePool::GrowingRoster);
        let batch = vec![(groups[0].clone(), 0), (groups[1].clone(), 1)];
        assert!(!mm.append_batch(id, &batch, now, |_, seats| {
            assert_eq!(seats.len(), 5);
            false
        }));
        assert_eq!(mm.queued_players(), 5);
        assert_eq!(mm.formed_match(id).unwrap().seats.len(), 5);
        assert!(mm.append_batch(id, &batch, now, |first, seats| {
            assert_eq!(first, 5);
            assert_eq!(seats[0].squad, seats[1].squad);
            assert_eq!(seats[2].squad, seats[4].squad);
            assert_ne!(seats[0].squad, seats[2].squad);
            assert_ne!(seats[0].squad, 0);
            true
        }));
        assert_eq!(mm.queued_players(), 0);
        assert_eq!(mm.formed_match(id).unwrap().seats.len(), 10);
        assert!(!mm.append_batch(id, &batch, now, |_, _| panic!("stale batch reserved")));
    }

    /// A leveled queue as it was before the start rules of 2026-10-13: two
    /// players are enough, and a press of Play adds nothing.
    fn classic(gather: Duration) -> Matchmaking {
        Matchmaking::new(gather).with_start_rules(2, Duration::ZERO, Duration::from_secs(3600))
    }

    fn after(start: Instant, seconds: u64) -> Instant {
        start + Duration::from_secs(seconds)
    }

    #[test]
    fn a_seat_is_told_once_until_play_or_the_end_of_its_match() {
        let start = Instant::now();
        let matchmaking = Matchmaking::new(Duration::ZERO);
        matchmaking.queue(&[player(1)], false, start);
        matchmaking.queue(&[player(2)], false, start);
        let Some(QueueState::Placed { match_id, .. }) = matchmaking.poll(1, start) else {
            panic!("not placed");
        };
        assert!(!matchmaking.is_told(1));
        matchmaking.mark_told(1);
        assert!(matchmaking.is_told(1));
        // Play again (back to its match, from a new sign-in): to be sent again.
        matchmaking.queue(
            &[Player {
                session_id: 7,
                ..player(1)
            }],
            false,
            start,
        );
        assert!(!matchmaking.is_told(1));
        assert!(!matchmaking.is_told(7));
        matchmaking.mark_told(7);
        matchmaking.mark_told(2);
        matchmaking.end_match(match_id);
        assert!(!matchmaking.is_told(7));
        assert!(!matchmaking.is_told(2));
    }

    #[test]
    fn players_gather_for_the_window_then_start_together() {
        let start = Instant::now();
        let matchmaking = Matchmaking::new(Duration::from_secs(10));
        matchmaking.queue(&[player(1)], false, start);
        matchmaking.queue(&[player(2)], false, after(start, 5));
        assert_eq!(
            matchmaking.poll(2, after(start, 14)),
            Some(QueueState::Waiting {
                place: 2,
                total: 2,
                since: after(start, 5)
            })
        );
        assert_eq!(matchmaking.match_count(), 0);

        let Some(QueueState::Placed {
            match_id,
            team,
            index,
        }) = matchmaking.poll(2, after(start, 15))
        else {
            panic!("the window ended");
        };
        assert_eq!((match_id, team, index), (FIRST_MATCH_ID, 1, 1));
        assert_eq!(
            matchmaking.poll(1, after(start, 15)),
            Some(QueueState::Placed {
                match_id,
                team: 0,
                index: 0
            })
        );
        assert_eq!(matchmaking.seat(2), Some((match_id, 1)));
        assert_eq!(matchmaking.formed_match(match_id).unwrap().seats.len(), 2);
    }

    #[test]
    fn one_player_alone_waits_and_the_window_starts_with_the_second() {
        let start = Instant::now();
        let matchmaking = Matchmaking::new(Duration::from_secs(60));
        matchmaking.queue(&[player(1)], false, start);
        // Long past the window: still waiting alone.
        assert!(matches!(
            matchmaking.poll(1, after(start, 600)),
            Some(QueueState::Waiting { .. })
        ));
        matchmaking.queue(&[player(2)], false, after(start, 700));
        assert!(matches!(
            matchmaking.poll(1, after(start, 759)),
            Some(QueueState::Waiting { total: 2, .. })
        ));
        assert!(matches!(
            matchmaking.poll(1, after(start, 760)),
            Some(QueueState::Placed { .. })
        ));
    }

    #[test]
    fn the_window_starts_over_when_the_queue_is_down_to_one() {
        let start = Instant::now();
        let matchmaking = Matchmaking::new(Duration::from_secs(60));
        matchmaking.queue(&[player(1)], false, start);
        matchmaking.queue(&[player(2)], false, after(start, 10));
        matchmaking.leave_queue(2);
        matchmaking.queue(&[player(3)], false, after(start, 50));
        // 60 s after player 2 joined, but only 20 s after the queue had two again.
        assert!(matches!(
            matchmaking.poll(1, after(start, 70)),
            Some(QueueState::Waiting { .. })
        ));
        assert!(matches!(
            matchmaking.poll(1, after(start, 110)),
            Some(QueueState::Placed { .. })
        ));
    }

    #[test]
    fn ten_waiting_start_once_nobody_joins_for_a_while() {
        let start = Instant::now();
        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        for id in 1..=10 {
            mm.queue(&[player(id)], false, after(start, id.into()));
        }
        // Someone joined at 10 s: the quiet period (30 s) runs from there.
        assert!(
            mm.poll(1, after(start, 39))
                .is_some_and(|s| matches!(s, QueueState::Waiting { .. }))
        );
        assert!(matches!(
            mm.poll(1, after(start, 40)),
            Some(QueueState::Placed { .. })
        ));
        assert_eq!(mm.match_count(), 1);
    }

    #[test]
    fn nine_waiting_still_wait_the_whole_window() {
        let start = Instant::now();
        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        for id in 1..=9 {
            mm.queue(&[player(id)], false, start);
        }
        assert!(matches!(
            mm.poll(1, after(start, 59)),
            Some(QueueState::Waiting { .. })
        ));
        assert!(matches!(
            mm.poll(1, after(start, 60)),
            Some(QueueState::Placed { .. })
        ));
    }

    #[test]
    fn thirty_five_players_make_a_full_match_then_one_when_settled_and_the_rest_wait() {
        let start = Instant::now();
        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        for id in 1..=35 {
            mm.queue(&[player(id)], false, start);
        }
        // 20 at once, 15 left: they have been quiet since the start.
        mm.poll(1, start);
        assert_eq!(mm.match_count(), 1);
        mm.poll(1, after(start, 30));
        assert_eq!(mm.match_count(), 2);
        assert!(matches!(
            mm.poll(35, after(start, 30)),
            Some(QueueState::Placed { .. })
        ));
    }

    #[test]
    fn a_full_match_starts_at_once_and_the_rest_wait() {
        let start = Instant::now();
        let matchmaking = Matchmaking::new(Duration::from_secs(60));
        for id in 0..21 {
            matchmaking.queue(&[player(id)], false, start);
        }
        assert!(matches!(
            matchmaking.poll(0, start),
            Some(QueueState::Placed { .. })
        ));
        assert_eq!(
            matchmaking.poll(20, start),
            Some(QueueState::Waiting {
                place: 1,
                total: 1,
                since: start
            })
        );
    }

    #[test]
    fn a_squad_plays_on_one_team() {
        let start = Instant::now();
        let matchmaking = Matchmaking::new(Duration::ZERO);
        matchmaking.queue(&[player(1)], false, start);
        matchmaking.queue(&[player(2), player(3), player(4)], true, start);
        matchmaking.queue(&[player(5)], false, start);
        matchmaking.poll(1, start);
        let seats = matchmaking.formed_match(FIRST_MATCH_ID).unwrap().seats;
        let teams: Vec<_> = seats.iter().map(|seat| (seat.team, seat.squad)).collect();
        assert_eq!(teams, [(1, 0), (0, 1), (0, 1), (0, 1), (1, 0)]);
    }

    #[test]
    fn two_given_teams_are_seated_at_once_each_on_its_side() {
        let start = Instant::now();
        let matchmaking = Matchmaking::new(Duration::from_secs(60));
        // One of them also waits in the queue, with another player.
        matchmaking.queue(&[player(2)], false, start);
        matchmaking.queue(&[player(9)], false, start);
        assert_eq!(matchmaking.seat_teams([&[player(1)], &[]], start), None);
        let id = matchmaking
            .seat_teams([&[player(1), player(2)], &[player(3)]], start)
            .unwrap();
        let seats = matchmaking.formed_match(id).unwrap().seats;
        let teams: Vec<_> = seats
            .iter()
            .map(|seat| (seat.player.session_id, seat.team, seat.squad))
            .collect();
        assert_eq!(teams, [(1, 0, 0), (2, 0, 0), (3, 1, 0)]);
        assert_eq!(
            matchmaking.poll(3, start),
            Some(QueueState::Placed {
                match_id: id,
                team: 1,
                index: 2
            })
        );
        // The queue lost the seated player and is one player again.
        assert_eq!(
            matchmaking.poll(9, after(start, 600)),
            Some(QueueState::Waiting {
                place: 1,
                total: 1,
                since: start
            })
        );
        // A seated player is in no second match.
        assert_eq!(
            matchmaking.seat_teams([&[player(3)], &[player(4)]], start),
            None
        );
        matchmaking.end_match(id);
        assert!(
            matchmaking
                .seat_teams([&[player(3)], &[player(4)]], start)
                .is_some()
        );
    }

    #[test]
    fn a_squad_alone_waits_for_somebody_who_is_not_of_it() {
        let start = Instant::now();
        let matchmaking = Matchmaking::new(Duration::from_secs(60));
        matchmaking.queue(&[player(1), player(2)], true, start);
        // Two players, but both of one side: long past the window, no match.
        assert_eq!(
            matchmaking.poll(1, after(start, 600)),
            Some(QueueState::Waiting {
                place: 1,
                total: 2,
                since: start
            })
        );
        assert_eq!(matchmaking.match_count(), 0);
        // One player alone is enough, and the window starts with it.
        matchmaking.queue(&[player(3)], false, after(start, 600));
        assert!(matches!(
            matchmaking.poll(1, after(start, 659)),
            Some(QueueState::Waiting { total: 3, .. })
        ));
        assert!(matches!(
            matchmaking.poll(1, after(start, 660)),
            Some(QueueState::Placed { .. })
        ));
        let seats = matchmaking.formed_match(FIRST_MATCH_ID).unwrap().seats;
        let team = |session: u32| {
            seats
                .iter()
                .find(|seat| seat.player.session_id == session)
                .unwrap()
                .team
        };
        assert_eq!(team(1), team(2));
        assert_ne!(team(1), team(3));
    }

    #[test]
    fn a_squad_is_left_waiting_when_the_other_side_leaves() {
        let start = Instant::now();
        let matchmaking = Matchmaking::new(Duration::from_secs(60));
        matchmaking.queue(&[player(1), player(2), player(3)], true, start);
        matchmaking.queue(&[player(4)], false, after(start, 10));
        matchmaking.leave_queue(4);
        assert!(matches!(
            matchmaking.poll(1, after(start, 300)),
            Some(QueueState::Waiting { total: 3, .. })
        ));
        // Another squad is somebody too; the window starts over with it.
        matchmaking.queue(&[player(5), player(6)], true, after(start, 300));
        assert!(matches!(
            matchmaking.poll(1, after(start, 359)),
            Some(QueueState::Waiting { total: 5, .. })
        ));
        assert!(matches!(
            matchmaking.poll(5, after(start, 360)),
            Some(QueueState::Placed { .. })
        ));
        assert_eq!(matchmaking.match_count(), 1);
    }

    #[test]
    fn leaving_takes_the_whole_squad_out() {
        let start = Instant::now();
        let matchmaking = Matchmaking::new(Duration::from_secs(10));
        matchmaking.queue(&[player(1), player(2)], true, start);
        matchmaking.leave_queue(2);
        assert_eq!(matchmaking.poll(1, after(start, 20)), None);
        assert!(matchmaking.busy_sessions().is_empty());
    }

    /// A player who left its match by its own will is not sent back to
    /// it: Play queues it, and it can be seated in another match while the
    /// one it left still runs.
    #[test]
    fn a_player_who_left_its_match_queues_for_another() {
        let start = Instant::now();
        let matchmaking = Matchmaking::new(Duration::ZERO);
        matchmaking.queue(&[player(1), player(2)], false, start);
        matchmaking.poll(1, start);
        matchmaking.mark_told(2);

        // Without a word of its own, Play sends it back.
        matchmaking.queue(&[player(2)], false, after(start, 10));
        assert_eq!(matchmaking.seat(2), Some((FIRST_MATCH_ID, 1)));

        assert_eq!(
            matchmaking.give_up_seat(player(2).account_id),
            Some(FIRST_MATCH_ID)
        );
        assert_eq!(matchmaking.seat(2), None);
        assert!(!matchmaking.is_told(2));
        assert_eq!(matchmaking.give_up_seat(player(2).account_id), None);
        // The roster is as it was formed, and the other keeps its seat.
        assert_eq!(
            matchmaking
                .formed_match(FIRST_MATCH_ID)
                .unwrap()
                .seats
                .len(),
            2
        );
        assert_eq!(matchmaking.seat(1), Some((FIRST_MATCH_ID, 0)));

        // Play: it waits for a second player, as anybody.
        matchmaking.queue(&[player(2)], false, after(start, 20));
        assert!(matches!(
            matchmaking.poll(2, after(start, 20)),
            Some(QueueState::Waiting {
                place: 1,
                total: 1,
                ..
            })
        ));
        matchmaking.queue(&[player(3)], false, after(start, 30));
        let second = match matchmaking.poll(2, after(start, 30)) {
            Some(QueueState::Placed { match_id, .. }) => match_id,
            other => panic!("not seated in a new match: {other:?}"),
        };
        assert_ne!(second, FIRST_MATCH_ID);
        assert_eq!(matchmaking.match_count(), 2);

        // The match it left ends: nothing of it is left, and the new seat
        // stands.
        matchmaking.end_match(FIRST_MATCH_ID);
        assert_eq!(matchmaking.seat(2).map(|(id, _)| id), Some(second));
        // In the new match it has a seat to give up like any other.
        assert_eq!(matchmaking.give_up_seat(player(2).account_id), Some(second));
    }

    #[test]
    fn a_seated_player_goes_back_to_its_match_from_a_new_sign_in() {
        let start = Instant::now();
        let matchmaking = Matchmaking::new(Duration::ZERO);
        matchmaking.queue(&[player(1), player(2)], false, start);
        matchmaking.poll(1, start);
        let again = Player {
            session_id: 77,
            ..player(2)
        };
        matchmaking.queue(&[again], false, after(start, 30));
        assert_eq!(matchmaking.seat(77), Some((FIRST_MATCH_ID, 1)));
        assert_eq!(matchmaking.seat(2), None);

        matchmaking.end_match(FIRST_MATCH_ID);
        assert_eq!(matchmaking.poll(77, after(start, 31)), None);
        assert_eq!(matchmaking.match_count(), 0);
    }

    const EIGHT: LevelRules = LevelRules {
        team_size: 8,
        step: Duration::from_secs(4),
    };

    fn at(level: u8) -> Level {
        Level {
            min: level,
            max: level + 1,
        }
    }

    /// The players of each team of a match, by session.
    fn teams(matchmaking: &Matchmaking, match_id: u32) -> [Vec<u32>; 2] {
        let mut teams = [Vec::new(), Vec::new()];
        for seat in matchmaking.formed_match(match_id).unwrap().seats {
            teams[usize::from(seat.team)].push(seat.player.session_id);
        }
        teams
    }

    #[test]
    fn sixteen_fill_a_match_of_eight_a_side_and_the_rest_wait() {
        let start = Instant::now();
        let mm = Matchmaking::new(Duration::from_secs(60));
        for id in 1..=20 {
            mm.queue_leveled(&[player(id)], &[at(3)], false, start);
        }
        assert!(matches!(
            mm.poll_with(1, start, Some(&EIGHT)),
            Some(QueueState::Placed { .. })
        ));
        let teams = teams(&mm, FIRST_MATCH_ID);
        assert_eq!((teams[0].len(), teams[1].len()), (8, 8));
        // The first sixteen that came; the other four wait a countdown.
        assert!(teams.iter().flatten().all(|&session| session <= 16));
        assert_eq!(
            mm.poll_with(17, after(start, 59), Some(&EIGHT)),
            Some(QueueState::Waiting {
                place: 1,
                total: 4,
                since: start
            })
        );
        assert!(matches!(
            mm.poll_with(17, after(start, 60), Some(&EIGHT)),
            Some(QueueState::Placed { .. })
        ));
        assert_eq!(mm.match_count(), 2);
    }

    #[test]
    fn the_side_size_is_the_rules() {
        let start = Instant::now();
        let mm = Matchmaking::new(Duration::ZERO);
        let rules = LevelRules {
            team_size: 2,
            step: Duration::ZERO,
        };
        assert_eq!(rules.match_size(), 4);
        for id in 1..=5 {
            mm.queue_leveled(&[player(id)], &[at(0)], false, start);
        }
        mm.poll_with(1, start, Some(&rules));
        let teams = teams(&mm, FIRST_MATCH_ID);
        assert_eq!((teams[0].len(), teams[1].len()), (2, 2));
        // A side is never more than the client's roster takes.
        let large = LevelRules {
            team_size: 50,
            step: Duration::ZERO,
        };
        assert_eq!(large.match_size(), MAX_PLAYERS_PER_MATCH);
    }

    #[test]
    fn two_players_still_start_after_the_window_and_one_waits_alone() {
        let start = Instant::now();
        let mm = classic(Duration::from_secs(60));
        mm.queue_leveled(&[player(1)], &[at(0)], false, start);
        assert!(matches!(
            mm.poll_with(1, after(start, 900), Some(&EIGHT)),
            Some(QueueState::Waiting { total: 1, .. })
        ));
        mm.queue_leveled(&[player(2)], &[at(0)], false, after(start, 900));
        assert!(matches!(
            mm.poll_with(1, after(start, 959), Some(&EIGHT)),
            Some(QueueState::Waiting { total: 2, .. })
        ));
        assert_eq!(
            mm.poll_with(1, after(start, 960), Some(&EIGHT)),
            Some(QueueState::Placed {
                match_id: FIRST_MATCH_ID,
                team: 0,
                index: 0
            })
        );
        assert_eq!(
            mm.poll_with(2, after(start, 960), Some(&EIGHT)),
            Some(QueueState::Placed {
                match_id: FIRST_MATCH_ID,
                team: 1,
                index: 1
            })
        );
    }

    /// Seen live on 2026-10-12: two squadmates alone in the queue were
    /// given a match after the window, both on one team and nobody against
    /// them.
    #[test]
    fn a_squad_alone_gets_no_match_by_levels_either() {
        let start = Instant::now();
        let mm = classic(Duration::from_secs(60));
        mm.queue_leveled(&[player(1), player(2)], &[at(0); 2], true, start);
        assert_eq!(
            mm.poll_with(1, after(start, 900), Some(&EIGHT)),
            Some(QueueState::Waiting {
                place: 1,
                total: 2,
                since: start
            })
        );
        assert_eq!(mm.match_count(), 0);
        // One player alone is somebody: a match once the window that
        // begins with it is over, the squad on one side.
        mm.queue_leveled(&[player(3)], &[at(0)], false, after(start, 900));
        assert!(matches!(
            mm.poll_with(1, after(start, 959), Some(&EIGHT)),
            Some(QueueState::Waiting { total: 3, .. })
        ));
        let Some(QueueState::Placed { match_id, .. }) =
            mm.poll_with(3, after(start, 960), Some(&EIGHT))
        else {
            panic!("the window ended at 960 s");
        };
        assert_eq!(teams(&mm, match_id), [vec![1, 2], vec![3]]);
    }

    #[test]
    fn a_squad_is_not_matched_with_itself_while_another_is_out_of_its_levels() {
        let start = Instant::now();
        let mm = classic(Duration::ZERO);
        mm.queue_leveled(&[player(1), player(2)], &[at(0); 2], true, start);
        mm.queue_leveled(&[player(3)], &[at(5)], false, start);
        // Two parties wait, so the window (none here) is over; but each is
        // out of the other's range (0..=1 and 5) for 4 steps of 4 s.
        assert!(matches!(
            mm.poll_with(1, after(start, 15), Some(&EIGHT)),
            Some(QueueState::Waiting { total: 3, .. })
        ));
        assert_eq!(mm.match_count(), 0);
        assert!(matches!(
            mm.poll_with(1, after(start, 16), Some(&EIGHT)),
            Some(QueueState::Placed { .. })
        ));
        assert_eq!(teams(&mm, FIRST_MATCH_ID), [vec![1, 2], vec![3]]);
    }

    #[test]
    fn a_squad_stays_on_one_side_and_the_sides_are_even() {
        let start = Instant::now();
        let mm = classic(Duration::ZERO);
        mm.queue_leveled(&[player(1)], &[at(0)], false, start);
        mm.queue_leveled(&[player(2), player(3), player(4)], &[at(0); 3], true, start);
        mm.queue_leveled(&[player(5), player(6)], &[at(0); 2], true, start);
        mm.queue_leveled(&[player(7)], &[at(0)], false, start);
        mm.queue_leveled(&[player(8)], &[at(0)], false, start);
        mm.poll_with(1, start, Some(&EIGHT));
        let seats = mm.formed_match(FIRST_MATCH_ID).unwrap().seats;
        let of = |session: u32| {
            let seat = seats
                .iter()
                .find(|seat| seat.player.session_id == session)
                .unwrap();
            (seat.team, seat.squad)
        };
        assert_eq!(of(2), of(3));
        assert_eq!(of(2), of(4));
        assert_eq!(of(5), of(6));
        assert_ne!(of(2).1, 0);
        assert_ne!(of(5).1, of(2).1);
        assert_eq!((of(1).1, of(7).1), (0, 0));
        // Four a side: the three with one alone, the two with two alone.
        let teams = teams(&mm, FIRST_MATCH_ID);
        assert_eq!((teams[0].len(), teams[1].len()), (4, 4));
        // The largest group is on team 0, as without levels.
        assert_eq!(of(2).0, 0);
    }

    #[test]
    fn squads_that_do_not_fit_two_sides_are_not_put_together() {
        // Three squads of five: no two sides of eight hold them all.
        assert!(!sides_fit(&[5, 5, 5], 8));
        assert!(sides_fit(&[5, 5, 3, 3], 8));
        assert!(sides_fit(&[8, 8], 8));
        assert!(!sides_fit(&[9], 8));
        assert!(!sides_fit(&[8, 8, 1], 8));
        assert!(sides_fit(&[], 8));

        let start = Instant::now();
        let mm = Matchmaking::new(Duration::from_secs(60));
        for squad in 0..3 {
            let players: Vec<Player> = (1..=5).map(|id| player(squad * 10 + id)).collect();
            mm.queue_leveled(&players, &[at(0); 5], true, start);
        }
        mm.queue_leveled(&[player(41)], &[at(0)], false, start);
        let start = after(start, 60);
        mm.poll_with(1, start, Some(&EIGHT));
        // The first two squads and the one alone; the third squad waits
        // a new window.
        let teams = teams(&mm, FIRST_MATCH_ID);
        assert_eq!(teams[0].len() + teams[1].len(), 11);
        assert!(teams.iter().all(|team| team.len() <= 8));
        assert!(matches!(
            mm.poll_with(21, start, Some(&EIGHT)),
            Some(QueueState::Waiting { total: 5, .. })
        ));
        assert!(matches!(
            mm.poll_with(41, start, Some(&EIGHT)),
            Some(QueueState::Placed { .. })
        ));
    }

    #[test]
    fn the_sides_are_even_in_levels_too() {
        let start = Instant::now();
        let mm = Matchmaking::new(Duration::ZERO);
        let rules = LevelRules {
            team_size: 8,
            step: Duration::ZERO,
        };
        for (id, level) in [(1, 15), (2, 14), (3, 1), (4, 0)] {
            mm.queue_leveled(&[player(id)], &[at(level)], false, start);
        }
        mm.poll_with(1, start, Some(&rules));
        // 15 + 0 against 14 + 1, not 15 + 1 against 14 + 0.
        let mut teams = teams(&mm, FIRST_MATCH_ID);
        teams.sort();
        assert_eq!(teams, [vec![1, 4], vec![2, 3]]);
    }

    #[test]
    fn the_range_of_levels_grows_with_the_wait() {
        let start = Instant::now();
        let mm = classic(Duration::ZERO);
        // Levels 2 and 9: 6 outside the first's 2..=3.
        mm.queue_leveled(&[player(1)], &[at(2)], false, start);
        mm.queue_leveled(&[player(2)], &[at(9)], false, start);
        assert!(matches!(
            mm.poll_with(1, after(start, 23), Some(&EIGHT)),
            Some(QueueState::Waiting { total: 2, .. })
        ));
        // 4 s a level: after 24 s the range is -4..=9.
        assert!(matches!(
            mm.poll_with(1, after(start, 24), Some(&EIGHT)),
            Some(QueueState::Placed { .. })
        ));
        assert!(matches!(
            mm.poll_with(2, after(start, 24), Some(&EIGHT)),
            Some(QueueState::Placed { .. })
        ));
    }

    #[test]
    fn the_range_reaches_down_as_well_as_up() {
        let start = Instant::now();
        let mm = classic(Duration::ZERO);
        // The one who waited longest is the higher: 9 down to 2 is 7.
        mm.queue_leveled(&[player(1)], &[at(9)], false, start);
        mm.queue_leveled(&[player(2)], &[at(2)], false, after(start, 20));
        // The second's own range reaches 9 after 24 s of its wait (44 s);
        // the first's reaches 2 after 28 s of its own.
        assert!(matches!(
            mm.poll_with(1, after(start, 27), Some(&EIGHT)),
            Some(QueueState::Waiting { .. })
        ));
        assert!(matches!(
            mm.poll_with(1, after(start, 28), Some(&EIGHT)),
            Some(QueueState::Placed { .. })
        ));
    }

    #[test]
    fn nobody_waits_for_ever_whatever_the_levels() {
        let start = Instant::now();
        let mm = classic(Duration::from_secs(60));
        // The lowest and the highest level there is.
        mm.queue_leveled(&[player(1)], &[at(0)], false, start);
        mm.queue_leveled(&[player(2)], &[Level { min: 15, max: 15 }], false, start);
        // 14 levels outside 0..=1: 56 s at 4 s a level, within the window.
        assert!(matches!(
            mm.poll_with(1, after(start, 60), Some(&EIGHT)),
            Some(QueueState::Placed { .. })
        ));
        // And with a slow step, as soon as the range is there.
        let slow = LevelRules {
            team_size: 8,
            step: Duration::from_secs(60),
        };
        let mm = classic(Duration::from_secs(60));
        mm.queue_leveled(&[player(1)], &[at(0)], false, start);
        mm.queue_leveled(&[player(2)], &[Level { min: 15, max: 15 }], false, start);
        assert!(matches!(
            mm.poll_with(1, after(start, 14 * 60 - 1), Some(&slow)),
            Some(QueueState::Waiting { .. })
        ));
        assert!(matches!(
            mm.poll_with(1, after(start, 14 * 60), Some(&slow)),
            Some(QueueState::Placed { .. })
        ));
    }

    #[test]
    fn players_of_near_levels_go_first_and_the_far_one_is_not_held_back_by_them() {
        let start = Instant::now();
        let mm = Matchmaking::new(Duration::from_secs(60));
        // One of level 0 came first; sixteen of level 12 after it.
        mm.queue_leveled(&[player(100)], &[at(0)], false, start);
        for id in 1..=16 {
            mm.queue_leveled(&[player(id)], &[at(12)], false, after(start, 1));
        }
        // The sixteen fill a match at once, around the first of them.
        assert!(matches!(
            mm.poll_with(1, after(start, 1), Some(&EIGHT)),
            Some(QueueState::Placed { .. })
        ));
        assert_eq!(
            mm.poll_with(100, after(start, 1), Some(&EIGHT)),
            Some(QueueState::Waiting {
                place: 1,
                total: 1,
                since: start
            })
        );
    }

    #[test]
    fn with_more_than_a_match_waiting_the_nearest_levels_are_taken() {
        let start = Instant::now();
        let mm = Matchmaking::new(Duration::ZERO);
        let rules = LevelRules {
            team_size: 2,
            step: Duration::from_secs(1),
        };
        // The first is of level 5; five more of levels 9, 6, 1, 5, 4.
        for (id, level) in [(1, 5), (2, 9), (3, 6), (4, 1), (5, 5), (6, 4)] {
            mm.queue_leveled(&[player(id)], &[at(level)], false, start);
        }
        mm.poll_with(1, after(start, 30), Some(&rules));
        let mut seated: Vec<u32> = teams(&mm, FIRST_MATCH_ID).into_iter().flatten().collect();
        seated.sort_unstable();
        // 5, 6 (in 5..=6), 5 again, then 4: not 9 or 1.
        assert_eq!(seated, [1, 3, 5, 6]);
    }

    #[test]
    fn a_squad_is_looked_for_by_its_highest_player() {
        let start = Instant::now();
        let mm = classic(Duration::ZERO);
        mm.queue_leveled(&[player(1), player(2)], &[at(1), at(10)], true, start);
        mm.queue_leveled(&[player(3)], &[at(10)], false, start);
        assert!(matches!(
            mm.poll_with(3, start, Some(&EIGHT)),
            Some(QueueState::Placed { .. })
        ));
        assert_eq!(mm.formed_match(FIRST_MATCH_ID).unwrap().seats.len(), 3);
    }

    #[test]
    fn the_waiting_are_counted_by_level() {
        let start = Instant::now();
        let mm = Matchmaking::new(Duration::from_secs(60));
        mm.queue_leveled(&[player(1), player(2)], &[at(1), at(10)], true, start);
        mm.queue_leveled(&[player(3)], &[at(10)], false, start);
        mm.queue(&[player(4)], false, start);
        mm.queue_leveled(&[player(5)], &[at(30)], false, start);
        let counts: [u16; 20] = mm.waiting_by_level();
        let mut expected = [0; 20];
        expected[0] = 1;
        expected[1] = 1;
        expected[10] = 2;
        expected[19] = 1;
        assert_eq!(counts, expected);
        mm.leave_queue(3);
        assert_eq!(mm.waiting_by_level::<20>()[10], 1);
    }

    #[test]
    fn a_queue_without_levels_is_not_changed_by_the_rules_of_another() {
        // 0.21d's poll: twenty in a match, whatever was queued with levels.
        let start = Instant::now();
        let mm = Matchmaking::new(Duration::from_secs(60));
        for id in 1..=20 {
            mm.queue_leveled(&[player(id)], &[at((id % 16) as u8)], false, start);
        }
        assert!(matches!(mm.poll(1, start), Some(QueueState::Placed { .. })));
        assert_eq!(mm.formed_match(FIRST_MATCH_ID).unwrap().seats.len(), 20);
    }

    const TEN: LevelRules = LevelRules {
        team_size: 10,
        step: Duration::ZERO,
    };

    /// Joins `count` players (ids from `first`) one by one, `gap` s apart
    /// from `at_s`; returns the time of the last.
    fn join_all(mm: &Matchmaking, first: u32, count: u32, at_s: u64, gap: u64, start: Instant) {
        for n in 0..count {
            mm.queue_leveled(
                &[player(first + n)],
                &[at(0)],
                false,
                after(start, at_s + u64::from(n) * gap),
            );
        }
    }

    fn placed(mm: &Matchmaking, session: u32, at_s: u64, start: Instant) -> bool {
        matches!(
            mm.poll_with(session, after(start, at_s), Some(&TEN)),
            Some(QueueState::Placed { .. })
        )
    }

    #[test]
    fn fewer_than_four_players_wait_however_long() {
        let start = Instant::now();
        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        join_all(&mm, 1, 3, 0, 1, start);
        assert!(!placed(&mm, 1, 5000, start));
        // The 4th starts a countdown of 60 s.
        join_all(&mm, 4, 1, 5000, 0, start);
        assert!(!placed(&mm, 1, 5059, start));
        assert!(placed(&mm, 1, 5060, start));
    }

    #[test]
    fn each_press_of_play_adds_thirty_seconds_to_what_is_left() {
        let start = Instant::now();
        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        join_all(&mm, 1, 4, 0, 0, start);
        // 4th at 0: ends at 60. A 5th at 50 (10 s left) makes it 90.
        join_all(&mm, 5, 1, 50, 0, start);
        assert!(!placed(&mm, 1, 89, start));
        assert!(placed(&mm, 1, 90, start));
    }

    #[test]
    fn changing_equipment_does_not_restart_the_shared_countdown() {
        // Four players use the gathering countdown; ten use the quiet period.
        for (count, changed_at, starts_at) in [(4, 59, 60), (10, 29, 30)] {
            let start = Instant::now();
            let mm = Matchmaking::new(Duration::from_secs(60)).with_start_rules(
                4,
                Duration::from_secs(15),
                Duration::from_secs(120),
            );
            {
                let mut inner = mm.inner.locked();
                inner.fill_players = 10;
                inner.quiet = Duration::from_secs(30);
            }
            for id in 1..=count {
                mm.queue_leveled(&[player(id)], &[at(0)], false, start);
            }
            let levels: Vec<_> = (1..=count)
                .map(|id| (player(id).account_id, at(10)))
                .collect();
            assert_eq!(
                mm.refresh_levels(&levels, after(start, changed_at)).len(),
                count as usize
            );
            assert!(matches!(
                mm.poll_with(1, after(start, changed_at), Some(&EIGHT)),
                Some(QueueState::Waiting { since, .. }) if since == after(start, changed_at)
            ));
            assert!(matches!(
                mm.poll_with(1, after(start, starts_at), Some(&EIGHT)),
                Some(QueueState::Placed { .. })
            ));
            // Once seated, changes cannot put players back into search.
            assert!(
                mm.refresh_levels(
                    &[(player(1).account_id, at(0))],
                    after(start, starts_at + 1)
                )
                .is_empty()
            );
        }
    }

    #[test]
    fn refreshing_growing_levels_invalidates_old_batches_without_extending_gathering() {
        let mm = Matchmaking::new(Duration::ZERO);
        mm.set_growing_rules(Duration::ZERO, 1.0);
        let now = Instant::now();
        mm.queue_in_pool(
            &[player(1), player(2)],
            &[],
            false,
            now,
            QueuePool::GrowingRoster,
        );
        mm.poll_with(1, now, Some(&TEN));
        let running = mm.seat(1).unwrap().0;
        mm.set_growing_rules(Duration::from_secs(120), 1.0);
        mm.queue_in_pool(
            &[player(3), player(4)],
            &[at(0); 2],
            true,
            now,
            QueuePool::GrowingRoster,
        );
        mm.queue_in_pool(&[player(5)], &[at(0)], false, now, QueuePool::GrowingRoster);
        let before = mm.queued_groups(QueuePool::GrowingRoster);
        let changed = mm.refresh_levels(&[(player(3).account_id, at(10))], after(now, 119));
        assert_eq!(changed, [player(3), player(4)]);
        assert!(!mm.append_batch(
            running,
            &[(before[0].clone(), 0), (before[1].clone(), 1)],
            after(now, 119),
            |_, _| panic!("stale levels reached the match host")
        ));
        let after = mm.queued_groups(QueuePool::GrowingRoster);
        assert_eq!(after[1].population().level_sum, 10);
        assert_eq!(after[1].pool, QueuePool::GrowingRoster);
        mm.poll_with(3, now + Duration::from_secs(119), Some(&TEN));
        assert!(mm.seat(3).is_none());
        mm.poll_with(3, now + Duration::from_secs(120), Some(&TEN));
        let formed = mm.formed_match(mm.seat(3).unwrap().0).unwrap();
        assert_eq!(formed.seats.len(), 3);
        let squad: Vec<_> = formed.seats.iter().filter(|seat| seat.squad != 0).collect();
        assert_eq!(squad.len(), 2);
        assert_eq!(squad[0].team, squad[1].team);
    }

    #[test]
    fn a_countdown_never_runs_longer_than_two_minutes() {
        let start = Instant::now();
        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        join_all(&mm, 1, 4, 0, 0, start);
        // One more player every 20 s would push the end out for ever.
        join_all(&mm, 5, 5, 20, 20, start);
        assert!(!placed(&mm, 1, 119, start));
        assert!(placed(&mm, 1, 120, start));
        assert_eq!(mm.formed_match(FIRST_MATCH_ID).unwrap().seats.len(), 9);
    }

    #[test]
    fn ten_or_more_start_after_thirty_quiet_seconds_and_twenty_at_once() {
        let start = Instant::now();
        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        join_all(&mm, 1, 10, 0, 5, start);
        // The 10th joined at 45 s, and the countdown (from the 4th, at 15 s)
        // is over: it waits for 30 s without a press (75 s).
        assert!(!placed(&mm, 1, 74, start));
        assert!(placed(&mm, 1, 75, start));
        assert_eq!(mm.formed_match(FIRST_MATCH_ID).unwrap().seats.len(), 10);

        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        join_all(&mm, 1, 20, 0, 0, start);
        assert!(placed(&mm, 1, 0, start));
        assert_eq!(mm.formed_match(FIRST_MATCH_ID).unwrap().seats.len(), 20);
    }

    #[test]
    fn ten_or_more_that_keep_coming_start_by_two_minutes() {
        let start = Instant::now();
        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        // 10 at 0 s, then one every 20 s: never quiet for 30 s.
        join_all(&mm, 1, 10, 0, 0, start);
        join_all(&mm, 11, 5, 20, 20, start);
        assert!(!placed(&mm, 1, 119, start));
        assert!(placed(&mm, 1, 120, start));
    }

    #[test]
    fn the_countdown_is_cancelled_under_four_and_starts_anew() {
        let start = Instant::now();
        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        join_all(&mm, 1, 4, 0, 0, start);
        mm.leave_queue(4);
        join_all(&mm, 4, 1, 50, 0, start);
        // Not 60 s from the first 4th, but from the new one at 50 s.
        assert!(!placed(&mm, 1, 109, start));
        assert!(placed(&mm, 1, 110, start));
    }

    #[test]
    fn a_squad_alone_waits_even_with_four_players() {
        let start = Instant::now();
        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        let squad: Vec<Player> = (1..=4).map(player).collect();
        mm.queue_leveled(&squad, &[at(0); 4], true, start);
        assert!(!placed(&mm, 1, 5000, start));
    }

    #[test]
    fn a_squad_counts_its_players_toward_four_but_is_one_side() {
        let start = Instant::now();
        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        // A squad of 3 and one player: 4 players, two sides.
        let squad: Vec<Player> = (1..=3).map(player).collect();
        mm.queue_leveled(&squad, &[at(0); 3], true, start);
        mm.queue_leveled(&[player(4)], &[at(0)], false, after(start, 10));
        // The countdown starts with the 4th player, at 10 s.
        assert!(!placed(&mm, 1, 69, start));
        assert!(placed(&mm, 1, 70, start));
        let seats = mm.formed_match(FIRST_MATCH_ID).unwrap().seats;
        let team = |session: u32| {
            seats
                .iter()
                .find(|seat| seat.player.session_id == session)
                .unwrap()
                .team
        };
        assert_eq!((team(1), team(2)), (team(3), team(3)));
        assert_ne!(team(1), team(4));
    }

    #[test]
    fn two_squads_of_two_make_a_match_on_two_sides() {
        let start = Instant::now();
        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        mm.queue_leveled(&[player(1), player(2)], &[at(0); 2], true, start);
        mm.queue_leveled(&[player(3), player(4)], &[at(0); 2], true, start);
        assert!(placed(&mm, 1, 60, start));
        let seats = mm.formed_match(FIRST_MATCH_ID).unwrap().seats;
        let teams: Vec<_> = seats.iter().map(|seat| (seat.team, seat.squad)).collect();
        assert_eq!(teams, [(0, 1), (0, 1), (1, 2), (1, 2)]);
    }

    #[test]
    fn squads_are_never_split_when_a_match_is_full() {
        let start = Instant::now();
        let mm = Matchmaking::new(DEFAULT_GATHER_WINDOW);
        // 20 places: two squads of 9 and two players; a squad of 5 waits.
        for first in [1_u32, 20] {
            let squad: Vec<Player> = (first..first + 9).map(player).collect();
            mm.queue_leveled(&squad, &[at(0); 9], true, start);
        }
        mm.queue_leveled(&[player(40)], &[at(0)], false, start);
        mm.queue_leveled(&[player(41)], &[at(0)], false, start);
        let squad: Vec<Player> = (50..55).map(player).collect();
        mm.queue_leveled(&squad, &[at(0); 5], true, start);
        assert!(placed(&mm, 1, 0, start));
        let seats = mm.formed_match(FIRST_MATCH_ID).unwrap().seats;
        assert_eq!(seats.len(), 20);
        assert!(seats.iter().all(|seat| seat.player.session_id < 50));
        for squad in [1, 2] {
            let teams: HashSet<u8> = seats
                .iter()
                .filter(|seat| seat.squad == squad)
                .map(|seat| seat.team)
                .collect();
            assert_eq!(teams.len(), 1, "squad {squad} was split");
        }
    }

    #[test]
    fn the_gathering_window_is_read_from_the_environment() {
        assert_eq!(gather_window(None), Ok(DEFAULT_GATHER_WINDOW));
        assert_eq!(gather_window(Some(" 0 ")), Ok(Duration::ZERO));
        assert_eq!(gather_window(Some("20")), Ok(Duration::from_secs(20)));
        let error = gather_window(Some("soon")).unwrap_err();
        assert!(error.contains("SRP_MATCH_GATHER_SECONDS"), "{error}");
    }
}
