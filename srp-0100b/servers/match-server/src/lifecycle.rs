// Match lifecycle: the authoritative phase clock of one match.
//
// The stock client runs no match timers of its own; it only displays what the
// server pushes (`game_status_changed`, `match_wait_time_changed`,
// `match_time_changed`) and leaves the match on `match_finished`. The phases are
// the client's `game_status` values. The timings mirror the server-side fields
// of the client's `match_options` struct, which are never sent on the wire:
// `wait2_time` (waiting for players), `wait_player_percent` (how full the match
// must be to stop waiting), `countdown_time`, `match_time` and `respawn_time`.
// `wait1_time` (waiting for the first player) has no visible effect: a match
// only exists here once its first player has joined.

use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use crate::message::ServerGameMessageKind;
use crate::message::server_message::raw::game_status;

/// Timings and thresholds for every match, read once at startup.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MatchRules {
    /// `wait2_time`: longest wait for more players before the countdown.
    pub wait_secs: u8,
    /// `wait_player_percent`: stop waiting once this share of the match's
    /// player slots has joined. `0` never waits, so a lone tester can play.
    pub wait_player_percent: u8,
    /// `countdown_time`: the final countdown before the match starts.
    pub countdown_secs: u8,
    /// `match_time`: also advertised to the client in `MatchOptions`.
    pub match_time_secs: u16,
    /// `respawn_time`: also advertised to the client in `MatchOptions`.
    pub respawn_secs: u8,
    /// How long the client shows its "time is up" text before `match_finished`
    /// takes everyone back to the lobby.
    pub time_up_secs: u8,
    /// How long finished-match peers may linger before they are dropped (the
    /// client disconnects by itself on `match_finished`).
    pub finished_grace_secs: u8,
    /// `victory_items_count`: batteries in play, also advertised to the client
    /// in `MatchOptions` (it shows this many as required). A team that stores
    /// them all ends the match early.
    pub battery_count: u8,
    /// How far (metres) from a player's eyes a battery or base may be used.
    /// The client's own `usable_objects_detection_distance` is 1 m.
    pub use_reach: f32,
}

impl Default for MatchRules {
    fn default() -> Self {
        Self {
            wait_secs: 60,
            wait_player_percent: 0,
            countdown_secs: 10,
            match_time_secs: 15 * 60,
            respawn_secs: 10,
            time_up_secs: 5,
            finished_grace_secs: 15,
            battery_count: 10,
            use_reach: 1.0,
        }
    }
}

impl MatchRules {
    /// Defaults, overridden by `SRP_MATCH_*` / `SRP_RESPAWN_SECS` /
    /// `SRP_BATTERIES` / `SRP_USE_REACH` environment variables (handy for short
    /// test matches). Invalid values are logged and ignored.
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            wait_secs: env_number("SRP_MATCH_WAIT_SECS", defaults.wait_secs, 0..=u8::MAX),
            wait_player_percent: env_number(
                "SRP_MATCH_WAIT_PLAYER_PERCENT",
                defaults.wait_player_percent,
                0..=100,
            ),
            countdown_secs: env_number(
                "SRP_MATCH_COUNTDOWN_SECS",
                defaults.countdown_secs,
                0..=u8::MAX,
            ),
            match_time_secs: env_number(
                "SRP_MATCH_TIME_SECS",
                defaults.match_time_secs,
                1..=u16::MAX,
            ),
            respawn_secs: env_number("SRP_RESPAWN_SECS", defaults.respawn_secs, 1..=u8::MAX),
            time_up_secs: defaults.time_up_secs,
            finished_grace_secs: defaults.finished_grace_secs,
            battery_count: env_number(
                "SRP_BATTERIES",
                defaults.battery_count,
                1..=crate::map::BATTERY_SPAWNS.len() as u8,
            ),
            use_reach: env_number("SRP_USE_REACH", defaults.use_reach, 0.1..=5.0),
        }
    }

    pub fn wait(&self) -> Duration {
        Duration::from_secs(self.wait_secs.into())
    }

    pub fn countdown(&self) -> Duration {
        Duration::from_secs(self.countdown_secs.into())
    }

    pub fn match_time(&self) -> Duration {
        Duration::from_secs(self.match_time_secs.into())
    }

    pub fn respawn_time(&self) -> Duration {
        Duration::from_secs(self.respawn_secs.into())
    }

    pub fn time_up(&self) -> Duration {
        Duration::from_secs(self.time_up_secs.into())
    }

    pub fn finished_grace(&self) -> Duration {
        Duration::from_secs(self.finished_grace_secs.into())
    }

    /// Whether `joined` of `capacity` players is enough to stop waiting.
    fn enough_players(&self, joined: usize, capacity: usize) -> bool {
        joined > 0 && joined * 100 >= usize::from(self.wait_player_percent) * capacity
    }
}

fn env_number<T>(name: &str, default: T, range: RangeInclusive<T>) -> T
where
    T: Copy + PartialOrd + std::fmt::Display + std::str::FromStr,
{
    let Ok(value) = std::env::var(name) else {
        return default;
    };
    match value.trim().parse::<T>() {
        Ok(number) if range.contains(&number) => number,
        _ => {
            log::warn!(
                "ignoring {name}={value:?}: expected a number in {}..={}; using {default}",
                range.start(),
                range.end()
            );
            default
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Phase {
    WaitingForPlayers {
        ends_at: Instant,
    },
    FinalCountdown {
        ends_at: Instant,
    },
    InProcess {
        ends_at: Instant,
    },
    /// The clock ran out: the client shows its "time is up" text; scores and
    /// respawns are frozen.
    TimeUp {
        ends_at: Instant,
    },
    /// `match_finished` was sent; peers are expected to disconnect.
    Finished {
        at: Instant,
    },
}

impl Phase {
    pub fn name(&self) -> &'static str {
        match self {
            Self::WaitingForPlayers { .. } => "waiting for players",
            Self::FinalCountdown { .. } => "final countdown",
            Self::InProcess { .. } => "in process",
            Self::TimeUp { .. } => "time up",
            Self::Finished { .. } => "finished",
        }
    }

    pub fn is_in_process(&self) -> bool {
        matches!(self, Self::InProcess { .. })
    }

    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Finished { .. })
    }

    /// No more damage, kills or respawns.
    pub fn is_over(&self) -> bool {
        matches!(self, Self::TimeUp { .. } | Self::Finished { .. })
    }

    fn ends_at(&self) -> Option<Instant> {
        match *self {
            Self::WaitingForPlayers { ends_at }
            | Self::FinalCountdown { ends_at }
            | Self::InProcess { ends_at }
            | Self::TimeUp { ends_at } => Some(ends_at),
            Self::Finished { .. } => None,
        }
    }

    fn status(&self) -> Option<game_status> {
        match self {
            Self::WaitingForPlayers { .. } => Some(game_status::waiting_for_players),
            Self::FinalCountdown { .. } => Some(game_status::final_countdown),
            Self::InProcess { .. } => Some(game_status::inprocess),
            Self::TimeUp { .. } | Self::Finished { .. } => None,
        }
    }
}

/// Whole seconds left until `ends_at`, rounded up like a countdown clock.
pub fn seconds_left(ends_at: Instant, now: Instant) -> u32 {
    let millis = ends_at.saturating_duration_since(now).as_millis();
    millis.div_ceil(1000).min(u128::from(u32::MAX)) as u32
}

pub struct MatchState {
    phase: Phase,
    /// The timer value (seconds) last pushed to the clients.
    announced_seconds: Option<u32>,
}

impl MatchState {
    pub fn new(now: Instant, rules: &MatchRules) -> Self {
        Self {
            phase: Phase::WaitingForPlayers {
                ends_at: now + rules.wait(),
            },
            announced_seconds: None,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Jump straight to `phase` (debug console). The caller announces it.
    pub fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
        self.announced_seconds = None;
    }

    /// Move the current phase's deadline (debug console).
    pub fn set_time_left(&mut self, now: Instant, time_left: Duration) -> bool {
        let ends_at = now + time_left;
        match &mut self.phase {
            Phase::WaitingForPlayers { ends_at: deadline }
            | Phase::FinalCountdown { ends_at: deadline }
            | Phase::InProcess { ends_at: deadline }
            | Phase::TimeUp { ends_at: deadline } => *deadline = ends_at,
            Phase::Finished { .. } => return false,
        }
        self.announced_seconds = None;
        true
    }

    /// Advance the clock. Returns every phase entered, in order (several when
    /// a phase has zero length, e.g. no countdown).
    pub fn advance(
        &mut self,
        now: Instant,
        rules: &MatchRules,
        joined: usize,
        capacity: usize,
    ) -> Vec<Phase> {
        let mut entered = Vec::new();
        loop {
            let next = match self.phase {
                Phase::WaitingForPlayers { ends_at }
                    if joined > 0 && (now >= ends_at || rules.enough_players(joined, capacity)) =>
                {
                    Phase::FinalCountdown {
                        ends_at: now + rules.countdown(),
                    }
                }
                Phase::FinalCountdown { ends_at } if now >= ends_at => Phase::InProcess {
                    ends_at: now + rules.match_time(),
                },
                Phase::InProcess { ends_at } if now >= ends_at => Phase::TimeUp {
                    ends_at: now + rules.time_up(),
                },
                Phase::TimeUp { ends_at } if now >= ends_at => Phase::Finished { at: now },
                _ => break,
            };
            self.set_phase(next);
            entered.push(next);
        }
        entered
    }

    /// Everything a client needs to show the current phase: the status, then
    /// its timer (the client picks the wait label from the status it already
    /// has, so the order matters). Then the "time is up" text, and finally
    /// `match_finished`.
    pub fn announcement(&self, now: Instant) -> Vec<ServerGameMessageKind> {
        match self.phase {
            Phase::TimeUp { .. } => {
                vec![ServerGameMessageKind::MatchTimeChanged { time_left_ms: 0 }]
            }
            Phase::Finished { .. } => vec![ServerGameMessageKind::MatchFinished],
            phase => phase
                .status()
                .map(|status| ServerGameMessageKind::GameStatusChanged { status })
                .into_iter()
                .chain(self.timer_message(now))
                .collect(),
        }
    }

    /// Record that the current timer value has been pushed to the clients.
    pub fn mark_announced(&mut self, now: Instant) {
        self.announced_seconds = self
            .phase
            .ends_at()
            .map(|ends_at| seconds_left(ends_at, now));
    }

    /// The timer message, once per displayed second.
    pub fn timer_tick(&mut self, now: Instant) -> Option<ServerGameMessageKind> {
        let seconds = seconds_left(self.phase.ends_at()?, now);
        if self.announced_seconds == Some(seconds) {
            return None;
        }
        self.announced_seconds = Some(seconds);
        self.timer_message(now)
    }

    fn timer_message(&self, now: Instant) -> Option<ServerGameMessageKind> {
        let seconds = seconds_left(self.phase.ends_at()?, now);
        Some(match self.phase {
            // The client shows these as MM:SS of whole seconds.
            Phase::WaitingForPlayers { .. } | Phase::FinalCountdown { .. } => {
                ServerGameMessageKind::MatchWaitTimeChanged { time_left: seconds }
            }
            // Milliseconds on the wire; whole seconds keep the display steady.
            Phase::InProcess { .. } => ServerGameMessageKind::MatchTimeChanged {
                time_left_ms: seconds.saturating_mul(1000),
            },
            Phase::TimeUp { .. } | Phase::Finished { .. } => return None,
        })
    }

    /// Seconds left in the current phase (for logs and the debug console).
    pub fn seconds_left(&self, now: Instant) -> Option<u32> {
        self.phase
            .ends_at()
            .map(|ends_at| seconds_left(ends_at, now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPACITY: usize = 20;

    fn rules() -> MatchRules {
        MatchRules::default()
    }

    #[test]
    fn default_rules_do_not_wait_for_more_players() {
        let rules = rules();
        let now = Instant::now();
        let mut state = MatchState::new(now, &rules);

        assert_eq!(state.advance(now, &rules, 0, CAPACITY), vec![]);
        let entered = state.advance(now, &rules, 1, CAPACITY);
        assert_eq!(
            entered,
            vec![Phase::FinalCountdown {
                ends_at: now + rules.countdown()
            }]
        );
    }

    #[test]
    fn waits_for_the_required_share_of_players_or_the_wait_time() {
        let rules = MatchRules {
            wait_player_percent: 50,
            ..rules()
        };
        let now = Instant::now();
        let mut state = MatchState::new(now, &rules);

        assert!(state.advance(now, &rules, 9, CAPACITY).is_empty());
        assert_eq!(state.advance(now, &rules, 10, CAPACITY).len(), 1);

        let mut lonely = MatchState::new(now, &rules);
        assert!(lonely.advance(now, &rules, 1, CAPACITY).is_empty());
        let at_deadline = now + rules.wait();
        assert_eq!(
            lonely.advance(at_deadline, &rules, 1, CAPACITY),
            vec![Phase::FinalCountdown {
                ends_at: at_deadline + rules.countdown()
            }],
            "the wait time starts the match with whoever is there"
        );
    }

    #[test]
    fn runs_countdown_match_and_finish_in_order() {
        let rules = rules();
        let start = Instant::now();
        let mut state = MatchState::new(start, &rules);
        state.advance(start, &rules, 1, CAPACITY);

        let countdown_end = start + rules.countdown();
        assert!(
            state
                .advance(
                    countdown_end - Duration::from_millis(1),
                    &rules,
                    1,
                    CAPACITY
                )
                .is_empty()
        );
        assert_eq!(
            state.advance(countdown_end, &rules, 1, CAPACITY),
            vec![Phase::InProcess {
                ends_at: countdown_end + rules.match_time()
            }]
        );

        let match_end = countdown_end + rules.match_time();
        assert_eq!(
            state.advance(match_end, &rules, 1, CAPACITY),
            vec![Phase::TimeUp {
                ends_at: match_end + rules.time_up()
            }]
        );
        let time_up_end = match_end + rules.time_up();
        assert_eq!(
            state.advance(time_up_end, &rules, 1, CAPACITY),
            vec![Phase::Finished { at: time_up_end }]
        );
        assert!(state.advance(time_up_end, &rules, 1, CAPACITY).is_empty());
    }

    #[test]
    fn zero_length_countdown_starts_the_match_at_once() {
        let rules = MatchRules {
            countdown_secs: 0,
            ..rules()
        };
        let now = Instant::now();
        let mut state = MatchState::new(now, &rules);
        let entered = state.advance(now, &rules, 1, CAPACITY);
        assert_eq!(entered.len(), 2);
        assert!(entered[1].is_in_process());
    }

    #[test]
    fn announces_status_before_its_timer() {
        let rules = rules();
        let now = Instant::now();
        let mut state = MatchState::new(now, &rules);
        state.advance(now, &rules, 1, CAPACITY);

        assert_eq!(
            state.announcement(now),
            vec![
                ServerGameMessageKind::GameStatusChanged {
                    status: game_status::final_countdown
                },
                ServerGameMessageKind::MatchWaitTimeChanged { time_left: 10 },
            ]
        );

        state.advance(now + rules.countdown(), &rules, 1, CAPACITY);
        assert_eq!(
            state.announcement(now + rules.countdown()),
            vec![
                ServerGameMessageKind::GameStatusChanged {
                    status: game_status::inprocess
                },
                ServerGameMessageKind::MatchTimeChanged {
                    time_left_ms: 15 * 60 * 1000
                },
            ]
        );
    }

    #[test]
    fn shows_time_up_then_ends_the_match() {
        let rules = rules();
        let now = Instant::now();
        let mut state = MatchState::new(now, &rules);
        state.set_phase(Phase::TimeUp {
            ends_at: now + rules.time_up(),
        });
        assert_eq!(
            state.announcement(now),
            vec![ServerGameMessageKind::MatchTimeChanged { time_left_ms: 0 }]
        );
        assert_eq!(state.timer_tick(now), None, "no timer while time is up");

        state.set_phase(Phase::Finished { at: now });
        assert_eq!(
            state.announcement(now),
            vec![ServerGameMessageKind::MatchFinished]
        );
    }

    #[test]
    fn pushes_the_timer_once_per_displayed_second() {
        let rules = rules();
        let start = Instant::now();
        let mut state = MatchState::new(start, &rules);
        state.set_phase(Phase::InProcess {
            ends_at: start + Duration::from_secs(60),
        });
        state.mark_announced(start);

        assert_eq!(state.timer_tick(start + Duration::from_millis(500)), None);
        assert_eq!(
            state.timer_tick(start + Duration::from_millis(1000)),
            Some(ServerGameMessageKind::MatchTimeChanged {
                time_left_ms: 59_000
            })
        );
        assert_eq!(state.timer_tick(start + Duration::from_millis(1500)), None);
    }

    #[test]
    fn rounds_time_left_up_to_whole_seconds() {
        let now = Instant::now();
        assert_eq!(seconds_left(now + Duration::from_millis(9_001), now), 10);
        assert_eq!(seconds_left(now + Duration::from_millis(9_000), now), 9);
        assert_eq!(seconds_left(now, now + Duration::from_secs(1)), 0);
    }

    #[test]
    fn debug_time_change_moves_the_deadline_and_re_announces() {
        let rules = rules();
        let now = Instant::now();
        let mut state = MatchState::new(now, &rules);
        state.set_phase(Phase::InProcess {
            ends_at: now + rules.match_time(),
        });
        state.mark_announced(now);

        assert!(state.set_time_left(now, Duration::from_secs(30)));
        assert_eq!(
            state.timer_tick(now),
            Some(ServerGameMessageKind::MatchTimeChanged {
                time_left_ms: 30_000
            })
        );

        state.set_phase(Phase::Finished { at: now });
        assert!(!state.set_time_left(now, Duration::from_secs(30)));
    }
}
