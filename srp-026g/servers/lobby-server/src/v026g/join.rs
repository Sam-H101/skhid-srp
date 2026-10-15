//! JOIN: the button of a match in the "matches now" window of srp's changed
//! lobby movie (`docs/notes/lobby.md` §2.16.1, list version M2).
//!
//! The owner's rule (2026-10-14): the button is on only for a player and a
//! match where that player would be put in at once, with no wait, by the
//! balance rule; joins are only to fix matches that are uneven by value
//! (what the balance rule counts: a player's value, its strength, the
//! lead), with the asker's own value counted, so a 3 v 4 that is even by
//! value takes nobody; and several players pressing it at once are handled.
//! So whether asker A may join match X now ([`Core::join_fit`]):
//!
//! 1. X admits late joiners now ([`Core::late_admission`]: growing rosters,
//!    teams not fixed, the match server admits, more than
//!    `policy::JOIN_CUTOFF` left, no earlier joiner on its way in);
//! 2. A is in the lobby (signed in, not in the queue, not in or sent to a
//!    match, no place in one), in no squad with others, its game plays in
//!    growing rosters, its Play would not be refused for an old game, and
//!    it never had a place in X (a place is never given twice);
//! 3. no other player's place is held in X (below);
//! 4. the late-join placement ([`policy::place_late`]) of A alone, with what
//!    A really counts for ([`Core::strength_of`], as the queue counts it)
//!    and no patience waited out, puts A on a side, and the sides then
//!    count strictly more alike than without A.
//!
//! Near a planned maintenance (the owner's of 2026-10-15), once the clients
//! turn Play off ([`MAINTENANCE_WINDOW`] minutes or fewer to it), point 4
//! is the maintenance rule instead: a match whose time limit ends
//! (`admission_remaining`) at least [`MAINTENANCE_MARGIN`] before the
//! maintenance starts takes A on its weaker side by value
//! ([`policy::place_weaker`]: no rule of sizes, of patience or of the sides
//! counting more alike, only a side's capacity and the match's room); a
//! match that ends later takes nobody. Everything else stays: the asker's
//! checks, one hold per match, the hold, Play from the hold (checked again
//! by the same rule) and never the queue.
//!
//! The window's rows say per asker whether that holds, and if not why
//! ([`JoinCode`], the row's `join`). Of the reasons that apply, the one
//! said is the first of this order (the owner's of 2026-10-15): the
//! asker's own that block every match, which the player must act on first
//! (it waits in the queue 13, it is in a match 8, in a squad 9, an old game
//! 10), then the match's own that hold for everyone (it takes no late
//! joiners 7, it is not in play yet 5, it ends soon 6, another player is
//! joining 4), then the asker's place of old in this match (11), then the
//! maintenance (12), then the placement (no side takes it 3, the sides
//! would count no more alike 2). A JOIN
//! request is decided again at the click: `ok` holds a place for the asker
//! for [`HOLD`], and its Play then puts it into X through the same steps
//! the queue's late joins take (`Matchmaking` and the match server's
//! reservation, all or nothing), checked again; a Play the match no longer
//! takes is refused ([`JOIN_REFUSED`]) and the player is **not** queued.
//!
//! Several clicks at once: the lobby is one thread that reads one message
//! at a time (`Lobby::run`: `recv_from`, then `Core::datagram` handles each
//! message in full before the next), so the clicks are decided one after
//! the other, each against the holds of the ones before it. One hold per
//! match: the match server takes one batch of joiners at a time and admits
//! no other until that one is in the match (`MatchRegistry::
//! join_running_group` closes admission on a reservation; the game reopens
//! it only once every admission has entered), which takes far longer than
//! a hold lives. A second asker's Play into the same match would always be
//! refused, so its JOIN is `full` while the first hold lives. For the same
//! reason the queue's late joins pass a match with a hold in it by.
//!
//! The JOIN answer carries the code too (`J1;<result>;<key>;<code>`); its
//! result word, all that the first M2 movie reads, is the code's
//! ([`JoinCode::result`]).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use session::matchmaking_policy::{self as policy, Joiner, LatePlacement, Worth};
use session::v026g::quests;
use session::{LockExt, QueuePool};
use survarium::v026g::lobby::requests::id;
use survarium::v026g::lobby::{self, JoinCode};

use super::core::{Core, Who};
use super::play::{MatchPlayer, Running, match_profile};

/// How long a place is held for a JOIN that was granted: the movie closes
/// its window and presses Play for the player at once.
pub(super) const HOLD: Duration = Duration::from_secs(10);

/// The game's text for a Play the held place no longer takes: "Error
/// changing readiness status." in the player's language, what the game
/// says of a Play-like press it could not carry out (the clan team's ready,
/// `clans`). The game has no text of a match that stopped taking players.
pub(super) const JOIN_REFUSED: &str = "st_cant_change_ready_state";

/// The maintenance rule is in force from this many minutes before a
/// planned maintenance on: the minutes the clients are told (`9F`), at
/// which they turn Play off (`update_play_button_state`, `lobby.md`
/// §2.16.1).
pub(super) const MAINTENANCE_WINDOW: u32 = 30;

/// Under the maintenance rule a match takes a joiner only when its time
/// limit ends at least this long before the maintenance starts.
pub(super) const MAINTENANCE_MARGIN: Duration = Duration::from_secs(60);

/// A place held in a match for a player whose JOIN was granted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Hold {
    /// The match, by the id its players are sent to it with
    /// ([`super::MatchTicket::match_id`]).
    pub match_id: u32,
    /// Its listing key, for the log.
    pub key: u32,
    pub until: Instant,
}

/// The lobby's listing keys of the matches: what the list's rows carry and
/// a JOIN request names a match by. Each match keeps its key for its life;
/// a key is not reused while a match has it.
pub(super) struct Listing {
    /// By the match's id ([`super::MatchTicket::match_id`]).
    keys: HashMap<u32, u32>,
    next: u32,
}

impl Listing {
    /// Keys from `first` on (brought into 1..=`JOIN_KEY_MAX`), and round
    /// again after the last.
    pub(super) fn new(first: u32) -> Self {
        Self {
            keys: HashMap::new(),
            next: first % lobby::JOIN_KEY_MAX + 1,
        }
    }

    /// The match's key: the one it has, or the next free one.
    pub(super) fn key(&mut self, match_id: u32) -> u32 {
        if let Some(&key) = self.keys.get(&match_id) {
            return key;
        }
        while self.keys.values().any(|&key| key == self.next) {
            self.next = self.next % lobby::JOIN_KEY_MAX + 1;
        }
        let key = self.next;
        self.next = self.next % lobby::JOIN_KEY_MAX + 1;
        self.keys.insert(match_id, key);
        key
    }

    /// The match that has `key`.
    pub(super) fn match_of(&self, key: u32) -> Option<u32> {
        self.keys
            .iter()
            .find(|(_, held)| **held == key)
            .map(|(match_id, _)| *match_id)
    }

    /// The match is over: its key goes.
    pub(super) fn forget(&mut self, match_id: u32) {
        self.keys.remove(&match_id);
    }

    /// Only the matches `open` says are open keep their keys.
    pub(super) fn retain(&mut self, open: impl Fn(u32) -> bool) {
        self.keys.retain(|match_id, _| open(*match_id));
    }
}

/// Why an asker may not join a match now, as the rows and the JOIN answer
/// say it, and in words for the log.
type Refusal = (JoinCode, &'static str);

/// An asker a match would take now ([`Core::join_fit`]).
pub(super) struct Fit {
    /// Where it goes, and the sides with it.
    pub placement: LatePlacement,
    pub team: u8,
    /// What it counts for besides being there.
    pub strength: i32,
    pub lead: [i64; 2],
    pub dramatic: bool,
    /// Placed by the maintenance rule, not the balance rule.
    pub maintenance: bool,
}

impl Fit {
    /// What the log's lines say of the rule that placed it.
    fn by(&self) -> &'static str {
        if self.maintenance {
            ", maintenance rule"
        } else {
            ""
        }
    }
}

impl Core<'_> {
    /// Whether a place in the match (by its ticket's id) is held for a
    /// player other than `asker` (for anyone with `None`) now.
    pub(super) fn is_held(&self, match_id: u32, asker: Option<u32>, now: Instant) -> bool {
        self.holds.iter().any(|(&account_id, hold)| {
            Some(account_id) != asker && hold.match_id == match_id && hold.until > now
        })
    }

    /// Why `who` may not join any match now, whatever the match: the
    /// rule's second point but the place it may have had in the match. The
    /// first reasons of all ([`Core::join_fit`]).
    fn cannot_join(&self, who: &Who) -> Option<Refusal> {
        let Some(player) = self.player(who.addr) else {
            return Some((JoinCode::Unknown, "not in the lobby"));
        };
        if player.queued_since.is_some() {
            return Some((JoinCode::InQueue, "it waits in the queue"));
        }
        if player.in_match.is_some() || player.connect.is_some() {
            return Some((JoinCode::InMatch, "it is in a match"));
        }
        if self
            .store
            .matchmaking()
            .pool_of_account(who.account_id)
            .is_some()
        {
            return Some((JoinCode::InMatch, "it has a place in a match"));
        }
        if self
            .store
            .squads()
            .of(who.account_id)
            .is_some_and(|squad| squad.members.len() > 1)
        {
            return Some((JoinCode::InSquad, "it is in a squad"));
        }
        if self.pool_for_sessions([who.session_id]) != Some(QueuePool::GrowingRoster) {
            return Some((
                JoinCode::Outdated,
                "its game does not join matches being played",
            ));
        }
        None
    }

    /// Why a running match takes no player now, when
    /// [`Core::late_admission`] says it does not: the rule's first point.
    /// A match of rosters that do not grow, or of fixed teams, never does;
    /// otherwise the match server's word on it tells a match too near its
    /// end, one not in play yet and one in play with time left, which
    /// admits nobody while a joiner is on its way in.
    fn not_admitting(&self, running: &Running, now: Instant) -> Refusal {
        if running.pool != QueuePool::GrowingRoster || running.fixed_teams {
            return (JoinCode::NoLateJoiners, "it takes no late joiners");
        }
        let ticket = running.ticket.match_id;
        if self.host.admission_remaining(ticket, now).is_some() {
            return (JoinCode::EndsSoon, "it ends too soon");
        }
        match self.host.live(ticket, now) {
            None => (JoinCode::Unknown, "it does not admit players now"),
            Some(live)
                if live
                    .time_left
                    .is_some_and(|left| left <= policy::JOIN_CUTOFF) =>
            {
                (JoinCode::EndsSoon, "it ends too soon")
            }
            Some(live) if live.played.is_none() => (JoinCode::NotStarted, "it is not in play yet"),
            Some(_) => (
                JoinCode::AnotherJoining,
                "it admits nobody now (a joiner on its way in)",
            ),
        }
    }

    /// When the maintenance starts (Unix seconds), while the maintenance
    /// rule is in force: one is planned and it is [`MAINTENANCE_WINDOW`]
    /// minutes away or less (0 once its time has come).
    fn maintenance_near(&self) -> Option<u64> {
        let minutes = self.store.minutes_to_maintenance()?;
        (minutes <= MAINTENANCE_WINDOW)
            .then(|| self.store.maintenance_at())
            .flatten()
    }

    /// Whether `who` may join the running match `id` now, and where it
    /// would go: the module's rule, its reasons checked in the module's
    /// order. A place held for `who` itself is no other player's.
    pub(super) fn join_fit(
        &self,
        who: &Who,
        id: u32,
        running: &Running,
        now: Instant,
    ) -> Result<Fit, Refusal> {
        if let Some(refusal) = self.cannot_join(who) {
            return Err(refusal);
        }
        let ticket = running.ticket.match_id;
        let Some(remaining) = self.late_admission(running, now) else {
            return Err(self.not_admitting(running, now));
        };
        if self.is_held(ticket, Some(who.account_id), now) {
            return Err((
                JoinCode::AnotherJoining,
                "a place is held in it for another player",
            ));
        }
        let Some(formed) = self.store.matchmaking().formed_match(id) else {
            return Err((JoinCode::Unknown, "the queue has no such match"));
        };
        if formed
            .seats
            .iter()
            .any(|seat| seat.player.account_id == who.account_id)
        {
            return Err((JoinCode::HadPlace, "it had a place in this match"));
        }
        let level = self.level_of(who.account_id, quests::now());
        let strength = self.strength_of(who.account_id, level);
        let live = self.host.live(ticket, now);
        let (sides, rule) = self.late_rule(running, live.as_ref());
        let worth = Worth {
            players: 1,
            strength: i64::from(strength),
        };
        let fit = |placement: LatePlacement, maintenance: bool| {
            let team = placement.assignments.first().map_or(0, |(_, team)| *team);
            Fit {
                placement,
                team,
                strength,
                lead: rule.lead,
                dramatic: rule.dramatic,
                maintenance,
            }
        };
        if let Some(maintenance_at) = self.maintenance_near() {
            // When its time limit ends it, in whole seconds rounded up.
            let ends =
                quests::now() + remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0);
            if ends + MAINTENANCE_MARGIN.as_secs() > maintenance_at {
                return Err((
                    JoinCode::AfterMaintenance,
                    "it ends after the maintenance starts, maintenance rule",
                ));
            }
            let Some(placement) = policy::place_weaker(sides, worth, &rule) else {
                return Err((
                    JoinCode::NoPlace,
                    "its weaker side is full or it has no room, maintenance rule",
                ));
            };
            return Ok(fit(placement, true));
        }
        let joiner = Joiner {
            worth,
            waited_out: false,
        };
        let Some(placement) = policy::place_late(sides, &[joiner], &rule) else {
            // A joiner who had waited out its patience would go in: it is
            // the rule that keeps an even match even that says no.
            let waited = Joiner {
                waited_out: true,
                ..joiner
            };
            if policy::place_late(sides, &[waited], &rule).is_some() {
                return Err((
                    JoinCode::Balanced,
                    "the sides would count less alike with it",
                ));
            }
            return Err((JoinCode::NoPlace, "no side takes it now"));
        };
        if placement.apart_after.abs() >= placement.apart_before.abs() {
            return Err((
                JoinCode::Balanced,
                "the sides would count no more alike with it",
            ));
        }
        Ok(fit(placement, false))
    }

    /// The match the lobby made whose players are sent to it with
    /// `match_id`, by the queue's id.
    fn running_of(&self, match_id: u32) -> Option<u32> {
        self.matches
            .iter()
            .find(|(_, running)| running.ticket.match_id == match_id)
            .map(|(id, _)| *id)
    }

    /// `"<players> v <players>, counting <a> v <b>"` of the sides of the
    /// match now, and with the asker when `after` says how they would be:
    /// what the log's JOIN lines end with.
    fn join_sides(&self, id: u32, after: Option<&Fit>, now: Instant) -> String {
        let Some(running) = self.matches.get(&id) else {
            return String::new();
        };
        let live = self.host.live(running.ticket.match_id, now);
        let (sides, rule) = self.late_rule(running, live.as_ref());
        let said = |sides: [Worth; 2]| {
            let counted = self.counted(sides, rule.lead);
            format!(
                "{} v {}, counting {:.1} v {:.1}",
                counted[0].1, counted[1].1, counted[0].0, counted[1].0
            )
        };
        match after {
            Some(fit) => format!(
                "; sides {}; with it {}",
                said(sides),
                said(fit.placement.sides)
            ),
            None => format!("; sides {}", said(sides)),
        }
    }

    /// `3B 00 <JOIN_START + key> <count>`: the JOIN button of the match
    /// with that listing key. Answered with one `91` row
    /// ([`lobby::join_answer`]); `ok` holds a place for the asker. A JOIN
    /// replaces the asker's place held before, granted or not: it chose
    /// another match.
    pub(super) fn join_request(&mut self, who: &Who, key: u32, now: Instant) {
        self.holds.retain(|_, hold| hold.until > now);
        self.holds.remove(&who.account_id);
        let target = self
            .listing
            .match_of(key)
            .and_then(|match_id| Some((match_id, self.running_of(match_id)?)));
        let (code, said) = match target {
            None => (
                JoinCode::Unknown,
                format!(
                    "{}: JOIN key {key}: gone (no match this lobby made and runs has it)",
                    who.name
                ),
            ),
            Some((match_id, id)) => {
                let fit = self.join_fit(who, id, &self.matches[&id], now);
                let (code, why) = match &fit {
                    Ok(fit) => (JoinCode::Yes, format!(" (team {}{})", fit.team, fit.by())),
                    Err((code, why)) => (*code, format!(" ({why}; code {})", *code as u8)),
                };
                let sides = self.join_sides(id, fit.as_ref().ok(), now);
                if code == JoinCode::Yes {
                    self.holds.insert(
                        who.account_id,
                        Hold {
                            match_id,
                            key,
                            until: now + HOLD,
                        },
                    );
                }
                (
                    code,
                    format!(
                        "{}: JOIN match {match_id:#x} (key {key}): {}{why}{sides}",
                        who.name,
                        code.result().word()
                    ),
                )
            }
        };
        log::info!("{said}");
        self.send_made(who.addr, lobby::join_answer(code, key));
    }

    /// Play from a player with a place held for its JOIN (`hold`, live):
    /// the rule is checked again, and if the match still takes the player
    /// it goes in at once, through the queue's own late-join steps: the
    /// seat (`Matchmaking::append_player`, as `append_batch`) only once the
    /// match server reserved the place (`MatchHost::join_running`), then
    /// "match found" and where the match is, as for any Play. Never another
    /// match and never a new one: otherwise the Play is refused with
    /// [`JOIN_REFUSED`] and the player stays in the lobby, not queued.
    pub(super) fn play_held(&mut self, who: &Who, kit_id: u32, hold: Hold, now: Instant) {
        let match_id = hold.match_id;
        let key = hold.key;
        let Some(id) = self.running_of(match_id) else {
            log::info!(
                "{}: Play with a place held in match {match_id:#x} (key {key}): refused (the \
                 match is over)",
                who.name
            );
            self.deny(who.addr, id::PLAY, JOIN_REFUSED);
            return;
        };
        let fit = match self.join_fit(who, id, &self.matches[&id], now) {
            Ok(fit) => fit,
            Err((code, why)) => {
                let why = format!("{why}; code {}", code as u8);
                let sides = self.join_sides(id, None, now);
                log::info!(
                    "{}: Play with a place held in match {match_id:#x} (key {key}): refused \
                     ({why}){sides}; not queued",
                    who.name
                );
                self.deny(who.addr, id::PLAY, JOIN_REFUSED);
                return;
            }
        };
        let sides = self.join_sides(id, Some(&fit), now);
        let unix = quests::now();
        let player = session::Player {
            session_id: who.session_id,
            account_id: who.account_id,
            profile_id: kit_id,
        };
        let profile = {
            let mut account = who.account.locked();
            account.settle_active_kit(unix);
            MatchPlayer {
                session_id: who.session_id,
                account_id: who.account_id,
                profile: match_profile(&account, fit.team, 0, unix),
                bot: self.accounts.is_bot_name(&account.name),
            }
        };
        let first = self.matches[&id].seats.len();
        let host = self.host;
        let admitted = self
            .store
            .matchmaking()
            .append_player(id, player, fit.team, |actual, _| {
                actual == first && host.join_running(match_id, vec![profile])
            });
        if !admitted {
            log::info!(
                "{}: Play with a place held in match {match_id:#x} (key {key}): refused (the \
                 match server did not take it{}){sides}; not queued",
                who.name,
                fit.by()
            );
            self.deny(who.addr, id::PLAY, JOIN_REFUSED);
            return;
        }
        log::info!(
            "{}: Play with a place held in match {match_id:#x} (key {key}): in, team {}{}{sides}",
            who.name,
            fit.team,
            fit.by()
        );
        if let Some(account) = self.store.account(who.account_id) {
            account.locked().play = Some(player);
        }
        let order_id = self.next_order_id;
        // 0xFFFFFFFF is the client's "not queued".
        self.next_order_id = (self.next_order_id + 1) % 0x7FFF_FFFF + 1;
        if let Some(state) = self.player_mut(who.addr) {
            state.queued_since = Some(now);
            state.order_id = order_id;
        }
        self.seated_late(
            id,
            &[(player, fit.team, fit.strength)],
            &fit.placement,
            fit.lead,
            fit.dramatic,
        );
        self.squads_dirty = true;
        // "Match found" and where the match server is (0x82).
        self.queue_status(who.addr, now);
    }
}
