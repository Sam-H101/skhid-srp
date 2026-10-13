//! Play: the queue, making a match, and coming back from it.
//!
//! When a match starts is 0.21d's rule (`session::Matchmaking`; the
//! owner's standing rule): a match needs two players; the gather window
//! (`SRP_MATCH_GATHER_SECONDS`) starts when the second one queues, and a
//! player alone waits; with `SRP_MATCH_FILL_PLAYERS` waiting and nobody
//! joining for `SRP_MATCH_QUIET_SECONDS` it starts sooner, and at once when
//! full. A squad queues as one group once every member pressed Play and
//! plays on one team. A seat is kept for the whole match: Play sends a
//! player who left back to it.
//!
//! Who plays whom is 0.26g's (`resources/notes/v026g/matchmaking.md`,
//! `session::v026g::matchmaking`):
//! - a match has two sides of at most [`Settings::team_size`] (8), as even
//!   as the squads allow, in players and then in levels;
//! - a player has a level, the client's own "equipment level": the highest
//!   of its kits, moved up by its rating. A match is formed around a
//!   waiting player from those whose level is in its range, which grows by
//!   one level at each end every [`Settings::search_step`] of its wait
//!   (the client draws that range on its search screen), so a level delays
//!   a match and never forbids it;
//! - the match's level (the reward multipliers' row) is the mean of its
//!   players' levels;
//! - the map is the one its players have played least, the mode and the
//!   weather drawn among the playable `maps_dict` ids of
//!   [`Settings::maps`] for it. 0.26g's Play (`2F <kit id>`) does not say
//!   where to play: there is one queue.
//!
//! The match itself is the match server's: the lobby hands it the roster
//! through [`MatchHost`] and sends each player where the ticket says
//! (0x82).
//!
//! 0x82 is not ordered against the client states (0x85) sent before it
//! (transport.md §4.1), and a 0x85 "in the queue" read after it would take
//! the client's match id away again (`lobby.md` §2.4). So a 0x82 goes only
//! once the client state sent just before it ("match found") is acked: the
//! ordered channel delivers in order, so every earlier state is then read.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use session::v026g::{Account, matchmaking, quests, rules, wire};
use session::{Level, LevelRules, LockExt, QueueState};
use survarium::v026g::lobby::requests::id;
use survarium::v026g::lobby::{self, ClientState, limits};
use survarium::v026g::maps;
use survarium::v026g::match_msgs::static_info::{
    KIT_NOT_LOADED, MatchItem, MatchKit, MatchProfile, Upgrade,
};

use super::core::{Core, Who};

/// A match the queue formed is made at the match server by the first of
/// its players the lobby looks at; one not made after this long has none.
const UNMADE_MATCH: Duration = Duration::from_secs(10);

/// The game's text for a Play with a kit that is none of the account's, or
/// the premium account's without one.
const PLAY_KIT: &str = "st_profile_check_failed";

/// The match server, as the lobby needs it. `servers/srp` gives the lobby
/// an adapter over the match server's registry.
pub trait MatchHost: Send + Sync {
    /// Make a match for this roster. The error is for the log; the players
    /// go back to the lobby.
    fn create(&self, setup: MatchSetup) -> Result<MatchTicket, String>;
    /// Whether the match is made and not over. The lobby asks about its
    /// matches every second and frees the seats of one that is over, also
    /// when nobody called [`super::Inbox::match_finished`].
    fn is_open(&self, match_id: u32) -> bool;
    /// The account signed in again while its match goes on: its place in
    /// the match is now its new session's (the client signs its match
    /// connection with the session, 0xD8). `false` when the match is over
    /// or the account has no place in it.
    fn reseat(&self, match_id: u32, account_id: u32, session_id: u32) -> bool;
}

/// One roster place: the index in [`MatchSetup::players`] is the player's
/// place in the match.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchPlayer {
    /// The session the client signs its match connection with (0xD8).
    pub session_id: u32,
    pub account_id: u32,
    /// The player as the match shows it: its kits, items and the skills'
    /// modifiers; `team` (0 or 1) and `squad` (0 for none) are set.
    pub profile: MatchProfile,
    /// **Testing only**: the account is a test bot
    /// (`session::v026g::bots`, `SRP_BOT_ACCOUNTS`; false for everyone
    /// unless the operator named bots). The match server then takes the
    /// place as a player and never as a witness of the world.
    pub bot: bool,
}

/// A match the lobby formed.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchSetup {
    /// A playable `maps_dict` id: the map and the mode.
    pub map_id: u8,
    /// The row of `match_levels_dict` the match's rewards are multiplied
    /// by: the mean of its players' levels
    /// (`session::v026g::matchmaking::match_level`).
    pub match_level: u16,
    pub players: Vec<MatchPlayer>,
}

/// Where the players of a made match connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchTicket {
    /// What the clients are told (0x82) and the match is known by from
    /// then on ([`MatchHost::is_open`], [`super::MatchFinished`]).
    pub match_id: u32,
    /// The match server's public host; a name is looked up, as the client
    /// takes only an IPv4 address.
    pub host: String,
    pub port: u16,
}

/// A match made and not over.
pub(super) struct Running {
    pub ticket: MatchTicket,
    pub map_id: u8,
    /// Its roster's sessions and teams, by place.
    pub seats: Vec<(u32, u8)>,
}

/// The account as a match shows it: every kit with what it wears, the
/// items once, and the modifiers its skills give
/// (`survarium::v026g::skills`), on `team` and in `squad`.
///
/// Only a kit the player could press Play with ([`rules::check_play`], the
/// client's own check of a kit: a weapon, and its ammunition) is loaded in
/// the match; any other goes as [`KIT_NOT_LOADED`]. In a match a dead
/// player picks the kit of its next spawn among the loaded ones, with no
/// check at all, and a spawn with nothing in the weapon slots 7 and 10
/// ends every game of the match: `base_player::respawn` leaves the item in
/// the hands empty (0x2DADC6..0x2DAE0C), `player::respawn` then takes the
/// player's own at +0xF47C, which is null, and calls through it
/// (0x1F25AD..0x1F25BF). Every client plays that spawn, so every client
/// dies at that tick. PROVEN: a player's crash report of match 0x1f793b8
/// (2026-10-12, access violation reading 0 at 0x1F25BF, under the respawn
/// rule's tick 0x316820), and match 0x5cc8cc52, whose last agreed world
/// has a dead player with an empty kit as its next (three matches lost
/// that day). Until then every kit of an account went as usable, the
/// empty ones too.
pub fn match_profile(account: &Account, team: u8, squad: u8, now: u64) -> MatchProfile {
    let body = wire::account_body(account, now);
    MatchProfile {
        team,
        name: body.name,
        clan_tag: body.clan_tag,
        protection: body.protection,
        kits: body
            .kits
            .into_iter()
            .map(|kit| MatchKit {
                state: if rules::check_play(account, kit.id, now).is_ok() {
                    kit.state
                } else {
                    KIT_NOT_LOADED
                },
                id: kit.id,
                name: kit.name,
                slots: kit
                    .slots
                    .map(|slot| slot.map(|slot| (slot.item_index, slot.count))),
                modifiers: kit.modifiers,
            })
            .collect(),
        items: body
            .items
            .into_iter()
            .map(|item| MatchItem {
                instance_id: item.item_id,
                dict_id: item.dict_id,
                condition_or_stack: item.condition_or_stack,
                repair_time: item.repair_time,
                reroll_time: item.reroll_time,
                modifications: item
                    .modifications
                    .iter()
                    .map(|modification| (modification.id, modification.state))
                    .collect(),
                upgrades: item
                    .upgrades
                    .iter()
                    .map(|upgrade| Upgrade {
                        id: upgrade.id,
                        unknown_4: upgrade.a,
                        unknown_8: upgrade.value,
                        param: upgrade.param,
                    })
                    .collect(),
            })
            .collect(),
        squad,
    }
}

/// Whether `ip` is this machine or its local network
/// (`vostok::config`'s rule).
fn is_local(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local(),
    }
}

/// `host` as a numeric IPv4 address, looked up if it is a name.
fn ipv4_host(host: &str, port: u16) -> String {
    if host.parse::<Ipv4Addr>().is_ok() {
        return host.to_owned();
    }
    let found = (host, port).to_socket_addrs().ok().and_then(|mut addrs| {
        addrs.find_map(|addr| match addr {
            SocketAddr::V4(addr) => Some(addr.ip().to_string()),
            SocketAddr::V6(_) => None,
        })
    });
    found.unwrap_or_else(|| {
        log::warn!("The match server host `{host}` has no IPv4 address; the client can't use it");
        host.to_owned()
    })
}

impl Core<'_> {
    /// How the queue forms matches ([`session::Matchmaking::poll_with`]).
    fn level_rules(&self) -> LevelRules {
        LevelRules {
            team_size: self.settings.team_size,
            step: Duration::from_secs(self.settings.search_step.get().into()),
        }
    }

    /// The level the account is looked for a match at; the lowest for an
    /// account the lobby does not have.
    fn level_of(&self, account_id: u32, unix: u64) -> Level {
        match self.accounts.get(account_id) {
            Some(account) => matchmaking::level(&account.locked(), unix),
            None => Level::default(),
        }
    }

    /// `2F <kit id>`: Play. In a squad, Play makes the player ready, and
    /// the squad queues once all are.
    pub(super) fn play(&mut self, who: &Who, kit_id: u32, now: Instant) {
        let unix = quests::now();
        let (checked, level) = {
            let mut account = who.account.locked();
            account.settle_active_kit(unix);
            (
                rules::check_play(&account, kit_id, unix),
                matchmaking::level(&account, unix),
            )
        };
        if let Err(error) = checked {
            log::info!("{}: Play refused: {error}", who.name);
            let key = match error {
                rules::ActionError::NoWeapon | rules::ActionError::NoAmmo => error.text_key(),
                _ => PLAY_KIT,
            };
            self.deny(who.addr, id::PLAY, key);
            return;
        }
        // A match that ended without a word frees its seats first: Play
        // would send the player back to it.
        self.check_matches(now);

        let player = session::Player {
            session_id: who.session_id,
            account_id: who.account_id,
            profile_id: kit_id,
        };
        if let Some(account) = self.store.account(who.account_id) {
            account.locked().play = Some(player);
        }
        let order_id = self.next_order_id;
        // 0xFFFFFFFF is the client's "not queued".
        self.next_order_id = (self.next_order_id + 1) % 0x7FFF_FFFF + 1;
        if let Some(state) = self.player_mut(who.addr)
            && state.queued_since.is_none()
        {
            state.queued_since = Some(now);
            state.order_id = order_id;
        }
        let waiting_before = self.store.matchmaking().queued_players();
        let mut mates = Vec::new();
        match self.store.squads().set_ready(who.account_id, true) {
            None => {
                log::info!(
                    "{} queues at level {} (looked for at {}..={})",
                    who.name,
                    level.min,
                    level.min,
                    level.max
                );
                self.store
                    .matchmaking()
                    .queue_leveled(&[player], &[level], false, now);
            }
            Some(squad) if squad.all_ready() => {
                let players: Option<Vec<session::Player>> = squad
                    .members
                    .iter()
                    .map(|member| self.store.account(*member)?.locked().play)
                    .collect();
                match players {
                    Some(players) => {
                        let levels: Vec<Level> = players
                            .iter()
                            .map(|player| self.level_of(player.account_id, unix))
                            .collect();
                        log::info!(
                            "Squad {} is ready: all {} in the queue, at levels {:?}",
                            squad.id,
                            players.len(),
                            levels.iter().map(|level| level.min).collect::<Vec<_>>()
                        );
                        self.store
                            .matchmaking()
                            .queue_leveled(&players, &levels, true, now);
                        mates = squad.members;
                    }
                    None => log::warn!("Squad {}: a ready member has no Play", squad.id),
                }
            }
            Some(_) => {}
        }
        self.squads_dirty = true;
        log::info!("{} pressed Play with kit {kit_id}", who.name);
        if self.store.matchmaking().queued_players() > waiting_before {
            // A squad in its own order; a player alone.
            let joined = if mates.is_empty() {
                vec![who.account_id]
            } else {
                mates.clone()
            };
            self.announce_joined(&who, &joined, now);
        }
        self.queue_status(who.addr, now);
        for mate in mates {
            if mate != who.account_id
                && let Some(addr) = self.by_account.get(&mate).copied()
            {
                self.queue_status(addr, now);
            }
        }
    }

    /// `31 <order> <left match>`: the player left the queue, or is back
    /// from a match it closed. Either way its client is in the lobby.
    pub(super) fn leave(&mut self, who: &Who, left_match: bool) {
        if !left_match {
            if let Some(account) = self.store.account(who.account_id) {
                account.locked().play = None;
            }
            // Its squad's group leaves the queue with it.
            self.leave_queue_announced(who.session_id, who.account_id, &who.name);
            self.store.squads().set_ready(who.account_id, false);
        }
        if let Some(player) = self.player_mut(who.addr) {
            player.queued_since = None;
            player.in_match = None;
            player.connect = None;
        }
        if !left_match {
            // A clan team it waited with does not wait any more.
            self.clan_left_queue(who.account_id);
        }
        log::info!(
            "{} {}",
            who.name,
            if left_match {
                "is back from its match"
            } else {
                "left the queue"
            }
        );
        self.send(who.addr, lobby::client_state(&ClientState::Lobby, None));
        if left_match {
            // Its match may be over already (the client closed it at the
            // end): the results.
            self.show_results(who.addr, false);
        }
        self.squads_dirty = true;
    }

    /// Equipment can change while searching. Refresh every waiting player
    /// before any poll can form a match, including polls for other players.
    /// A changed level restarts the whole party's search and range timer.
    pub(super) fn refresh_search_levels(&mut self, now: Instant) {
        let unix = quests::now();
        let levels: Vec<_> = self
            .players()
            .into_iter()
            .filter_map(|addr| {
                let player = self.player(addr)?;
                player.queued_since?;
                Some((player.account_id, self.level_of(player.account_id, unix)))
            })
            .collect();
        if levels.is_empty() {
            return;
        }
        for player in self.store.matchmaking().refresh_levels(&levels, now) {
            if let Some(addr) = self.by_account.get(&player.account_id).copied()
                && let Some(state) = self.player_mut(addr)
            {
                state.queued_since = Some(now);
            }
            log::info!(
                "Account {} restarts search after its party's equipment level changed",
                player.account_id
            );
        }
    }

    /// What the client is told of its place in the queue:
    /// - while it waits, its place and how long it has waited (the client
    ///   shows the wait the server last sent);
    /// - a squad member whose squad is not all ready waits on its own, and
    ///   its client shows "squad not ready";
    /// - once its match is made, "match found" and where the match server
    ///   is (0x82), once.
    pub(super) fn queue_status(&mut self, addr: SocketAddr, now: Instant) {
        self.refresh_search_levels(now);
        let Some(player) = self.player(addr) else {
            return;
        };
        let (session_id, queued_since, order_id) =
            (player.session_id, player.queued_since, player.order_id);
        let in_match = player.in_match;
        let search_step = self.settings.search_step;
        // [GUESS] The client draws entry `i` as a column over level `i` of
        // its search screen (`LobbyGameSearch.setSearchProperties`), the
        // highest of them full height (of 30 at least): the players
        // searching at each level.
        let search_properties: [u16; limits::SEARCH_PROPERTIES] =
            self.store.matchmaking().waiting_by_level();
        let rules = self.level_rules();
        let waiting = move |place: u32, total: u32, since: Instant| {
            lobby::client_state(
                &ClientState::InQueue {
                    order_id,
                    place,
                    queue_total: total,
                    current_wait_seconds: now.saturating_duration_since(since).as_secs() as u32,
                    average_wait_seconds: 0,
                    search_step,
                    search_properties,
                },
                None,
            )
        };
        match self
            .store
            .matchmaking()
            .poll_with(session_id, now, Some(&rules))
        {
            Some(QueueState::Placed {
                match_id,
                team,
                index,
            }) => {
                if queued_since.is_none() && self.store.matchmaking().is_told(session_id) {
                    return;
                }
                if !self.matches.contains_key(&match_id) && !self.launch(match_id) {
                    return;
                }
                let ticket = self.matches[&match_id].ticket.clone();
                // The queue keeps a seat by account; the match server finds
                // a connecting client's place by its session. A player who
                // signed in again and pressed Play comes with a new one.
                let seated_as = self.matches[&match_id]
                    .seats
                    .get(index)
                    .map(|(session, _)| *session);
                if seated_as.is_some_and(|session| session != session_id) {
                    let account_id = self.player(addr).map_or(0, |player| player.account_id);
                    if self.host.reseat(ticket.match_id, account_id, session_id) {
                        log::info!(
                            "Match {:#x}: place {index} is now session {session_id:#x}'s (its \
                             account signed in again)",
                            ticket.match_id
                        );
                    } else {
                        log::warn!(
                            "Match {:#x}: place {index} could not be given to session \
                             {session_id:#x}; its client will be turned away",
                            ticket.match_id
                        );
                    }
                    if let Some(seat) = self
                        .matches
                        .get_mut(&match_id)
                        .and_then(|running| running.seats.get_mut(index))
                    {
                        seat.0 = session_id;
                    }
                }
                self.store.matchmaking().mark_told(session_id);
                let host = self.match_host_for(addr, &ticket);
                let found = waiting(0, 0, now);
                let found = self.send(addr, found);
                // Its game loads a match now: from here on it can show a
                // results screen (`show_results`). Sessions that are over
                // are forgotten here.
                let store = self.store;
                self.sent_to_match.retain(|id| !store.has_ended(*id));
                self.sent_to_match.insert(session_id);
                if let Some(player) = self.player_mut(addr) {
                    player.queued_since = None;
                    player.in_match = Some(match_id);
                    player.connect = Some((
                        found,
                        lobby::connect_to_match(
                            &host,
                            ticket.port,
                            ticket.match_id,
                            team,
                            // The session the client gives the match server
                            // in 0xD8 and 0xD9: the one its seat is kept by.
                            session_id,
                        ),
                    ));
                    log::info!(
                        "{} goes to match {:#x} on team {team} at {host}:{}",
                        player.name,
                        ticket.match_id,
                        ticket.port
                    );
                }
            }
            Some(QueueState::Waiting {
                place,
                total,
                since,
            }) => {
                let status = waiting(place as u32, total as u32, since);
                self.send(addr, status);
            }
            // Its match is over.
            None if in_match.is_some() => {
                if let Some(player) = self.player_mut(addr) {
                    player.in_match = None;
                    player.connect = None;
                }
                self.send(addr, lobby::client_state(&ClientState::Lobby, None));
                self.show_results(addr, false);
            }
            None => {
                // Ready, in a squad that is not.
                if let Some(since) = queued_since {
                    let status = waiting(1, 1, since);
                    self.send(addr, status);
                }
            }
        }
    }

    /// The match server's host for the client at `addr`: one on this
    /// machine or its network is told the address it reached srp at, when
    /// the match server is on this machine too.
    fn match_host_for(&self, addr: SocketAddr, ticket: &MatchTicket) -> String {
        let reached = self
            .player(addr)
            .and_then(|player| player.reached)
            .map(|reached| reached.to_canonical());
        match reached {
            Some(IpAddr::V4(reached))
                if self.settings.local_peers
                    && is_local(addr.ip())
                    && ticket.host == self.settings.public_host =>
            {
                reached.to_string()
            }
            _ => ipv4_host(&ticket.host, ticket.port),
        }
    }

    /// Make the match the queue formed at the match server. `false` when
    /// that failed: its players are back in the lobby.
    fn launch(&mut self, match_id: u32) -> bool {
        let Some(formed) = self.store.matchmaking().formed_match(match_id) else {
            return false;
        };
        let unix = quests::now();
        // The matches its players were put in on each map, and their
        // levels.
        let mut plays = [0_u64; 256];
        let mut levels = Vec::with_capacity(formed.seats.len());
        let players: Option<Vec<MatchPlayer>> = formed
            .seats
            .iter()
            .map(|seat| {
                let account = self.accounts.get(seat.player.account_id)?;
                let mut account = account.locked();
                account.settle_active_kit(unix);
                levels.push(matchmaking::level(&account, unix).min);
                for &(map, count) in &account.map_plays {
                    plays[usize::from(map)] += u64::from(count);
                }
                Some(MatchPlayer {
                    session_id: seat.player.session_id,
                    account_id: seat.player.account_id,
                    profile: match_profile(&account, seat.team, seat.squad, unix),
                    bot: self.accounts.is_bot_name(&account.name),
                })
            })
            .collect();
        let rng = &mut self.rng;
        let map_id = matchmaking::choose_map(
            &self.settings.maps,
            |map| plays[usize::from(map)],
            |bound| rng.below(bound),
        )
        .or_else(|| maps::playable().next().map(|map| map.id))
        .unwrap_or(0);
        let match_level = matchmaking::match_level(&levels);
        let made = match players {
            Some(players) => self.host.create(MatchSetup {
                map_id,
                match_level,
                players,
            }),
            None => Err("a seated player has no 0.26g account".to_owned()),
        };
        match made {
            Ok(ticket) => {
                let map = maps::map_mode(map_id).map_or(0, |map| map.level);
                log::info!(
                    "Match {:#x} made for {} players (levels {levels:?}: match level \
                     {match_level}) on map {map_id} (level_{map:02}, played {} times by them), \
                     at {}:{}",
                    ticket.match_id,
                    formed.seats.len(),
                    plays[usize::from(map)],
                    ticket.host,
                    ticket.port
                );
                // The map is one more that each of them has played.
                for seat in &formed.seats {
                    let account_id = seat.player.account_id;
                    if let Some(account) = self.accounts.get(account_id) {
                        matchmaking::count_map_play(&mut account.locked(), map);
                        self.accounts.mark_dirty(account_id);
                    }
                }
                let mut accounts = Vec::new();
                let mut teams: [Vec<String>; 2] = [Vec::new(), Vec::new()];
                for seat in &formed.seats {
                    accounts.push(seat.player.account_id);
                    if let Some(account) = self.accounts.get(seat.player.account_id) {
                        teams[usize::from(seat.team.min(1))].push(account.locked().name.clone());
                    }
                }
                self.announce_match(&accounts, teams);
                self.matches.insert(
                    match_id,
                    Running {
                        ticket,
                        map_id,
                        seats: formed
                            .seats
                            .iter()
                            .map(|seat| (seat.player.session_id, seat.team))
                            .collect(),
                    },
                );
                true
            }
            Err(error) => {
                log::error!(
                    "A match for {} players could not be made: {error}; they are back in the lobby",
                    formed.seats.len()
                );
                self.store.matchmaking().end_match(match_id);
                for seat in &formed.seats {
                    let account_id = seat.player.account_id;
                    self.store.squads().set_ready(account_id, false);
                    if let Some(account) = self.store.account(account_id) {
                        account.locked().play = None;
                    }
                    let Some(addr) = self.by_account.get(&account_id).copied() else {
                        continue;
                    };
                    if let Some(player) = self.player_mut(addr) {
                        player.queued_since = None;
                        player.in_match = None;
                        player.connect = None;
                    }
                    self.send(addr, lobby::client_state(&ClientState::Lobby, None));
                }
                self.squads_dirty = true;
                false
            }
        }
    }

    /// Free the seats of the matches the match server no longer has, and
    /// of a match the queue formed that never reached the match server
    /// (its players' links all went as it formed).
    pub(super) fn check_matches(&mut self, now: Instant) {
        let over: Vec<u32> = self
            .matches
            .iter()
            .filter(|(_, running)| !self.host.is_open(running.ticket.match_id))
            .map(|(id, _)| *id)
            .collect();
        for id in over {
            self.finish(id);
        }
        for (id, formed_at) in self.store.matchmaking().matches() {
            if !self.matches.contains_key(&id)
                && now.saturating_duration_since(formed_at) >= UNMADE_MATCH
            {
                log::warn!("Match {id:#x} of the queue was never made; its seats are freed");
                self.store.matchmaking().end_match(id);
            }
        }
    }

    /// [`super::Inbox::match_finished`], by the id the clients were told.
    pub(super) fn match_finished(&mut self, ticket_match_id: u32, _now: Instant) {
        let id = self
            .matches
            .iter()
            .find(|(_, running)| running.ticket.match_id == ticket_match_id)
            .map(|(id, _)| *id);
        match id {
            Some(id) => self.finish(id),
            None => log::debug!("Match {ticket_match_id:#x} is over; the lobby had freed it"),
        }
    }

    /// The match is over: its seats are free, and its players who are
    /// still marked in it are told they are in the lobby (`85 00`; the
    /// client's Play is off in any other state).
    fn finish(&mut self, match_id: u32) {
        let Some(running) = self.matches.remove(&match_id) else {
            return;
        };
        self.store.matchmaking().end_match(match_id);
        log::info!(
            "Match {:#x} (map {}) is over: {} seats freed",
            running.ticket.match_id,
            running.map_id,
            running.seats.len()
        );
        for (session_id, _) in &running.seats {
            // A session that ended during the match was kept for it.
            self.store.seen_in_match(*session_id);
        }
        for addr in self.players() {
            let Some(player) = self.player_mut(addr) else {
                continue;
            };
            if player.in_match != Some(match_id) {
                continue;
            }
            player.in_match = None;
            player.connect = None;
            self.send(addr, lobby::client_state(&ClientState::Lobby, None));
            self.show_results(addr, false);
        }
    }

    /// The results of the matches the account's client has not been shown
    /// (`session::v026g::rewards`), once: the screen of the last one (`97`)
    /// and the notices of all of them. A client that was signed in through
    /// the match's end gets what the match changed of its account first
    /// (`MatchReport::lobby_push`); one whose link is new (`after_sign_in`:
    /// it signed in, or connected again) has that from the sign-in's pushes.
    /// Call it when the client is in the lobby (`85 00`).
    ///
    /// A client whose session the lobby has sent into no match
    /// (`sent_to_match`) gets the notices and **no screen**: the game would
    /// die of it. The screen's code translates the map's name it kept from
    /// the loading screen of its last match, and a game that has loaded no
    /// match since it was started has none (a null pointer: `results.md` §6
    /// hazard 10, seen in a crash dump on 2026-10-13). That is the player
    /// whose game ended in a match and who starts it again, at the sign-in
    /// when the match is over by then, and in the lobby when it ends later.
    /// What the match changed of the account, such a client has from its
    /// sign-in or gets with the account's next push.
    pub(super) fn show_results(&mut self, addr: SocketAddr, after_sign_in: bool) {
        let Some(who) = self.who(addr) else {
            return;
        };
        let Some(unseen) = self.results.take_unseen(who.account_id) else {
            return;
        };
        // What it earned is its clan's too.
        if let Some(player) = unseen.report.player(who.account_id) {
            self.clan_experience(who.account_id, player.reward.experience);
        }
        if !self.sent_to_match.contains(&who.session_id) {
            log::info!(
                "{} gets the notices of match {:#x} and no results screen: its game has loaded no match since it signed in, and would die of one",
                who.name,
                unseen.report.match_id
            );
            self.notify(&who, &unseen.notices);
            return;
        }
        let push = if after_sign_in {
            // The inventory once more if autobuy or auto-repair spent
            // anything: the screen lists it from there.
            let account = who.account.locked();
            unseen.report.screen_for(who.account_id).map(|screen| {
                let mut push = Vec::new();
                if let Some(spent) = unseen.report.spent_inventory(who.account_id, &account) {
                    push.push(spent?);
                }
                push.push(screen?);
                Ok(push)
            })
        } else {
            // With no account locked: the ranking locks each in turn.
            let place = self
                .accounts
                .ranking()
                .iter()
                .position(|row| row.0 == who.account_id)
                .map(|index| index as u32 + 1);
            let account = who.account.locked();
            unseen
                .report
                .lobby_push(who.account_id, &account, quests::now(), place)
        };
        match push {
            Some(Ok(push)) => {
                log::info!(
                    "{} is shown the results of match {:#x}",
                    who.name,
                    unseen.report.match_id
                );
                self.send_all(addr, push);
                let balance = {
                    let account = who.account.locked();
                    (account.generic_money, account.premium_money)
                };
                if let Some(player) = self.player_mut(addr) {
                    player.money_sent = balance;
                }
            }
            Some(Err(error)) => log::error!(
                "The results of match {:#x} could not be sent to {}: {error}",
                unseen.report.match_id,
                who.name
            ),
            None => {}
        }
        self.notify(&who, &unseen.notices);
        // The daily bonus as the match left it, and a premium offer
        // (`shop`).
        self.after_results(&who, &unseen.report);
    }

    /// The team of a player in the match it was sent to.
    pub(super) fn team_in_match(&self, match_id: u32, session_id: u32) -> Option<u8> {
        self.matches
            .get(&match_id)?
            .seats
            .iter()
            .find(|(session, _)| *session == session_id)
            .map(|(_, team)| *team)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_is_given_an_ipv4_address() {
        assert_eq!(ipv4_host("203.0.113.7", 1236), "203.0.113.7");
        // A name is looked up: the client takes no name.
        assert_eq!(ipv4_host("localhost", 1236), "127.0.0.1");
    }

    #[test]
    fn this_machine_and_its_network_are_local() {
        let local = |ip: &str| is_local(ip.parse().unwrap());
        assert!(local("127.0.0.1") && local("192.168.1.20") && local("10.0.0.5"));
        assert!(local("::1") && local("::ffff:192.168.1.20"));
        assert!(!local("203.0.113.7") && !local("100.64.0.1"));
    }
}
