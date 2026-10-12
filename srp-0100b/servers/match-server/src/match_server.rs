// Multi-client match server: a dumb relay.
//
// SRP is a mock — this does NOT simulate the game. It owns one UDP socket,
// demultiplexes datagrams to a per-peer `Transport` (reliable-UDP), and:
//   * runs each client through the connect/startup/join handshake, reflecting the
//     per-match live roster (one `PlayerProfile`/`SpawnPlayer` per connected
//     peer), and
//   * relays each client's `ClientPlayerUpdate` to every OTHER peer as a
//     `ServerPlayerInput`, so players see each other move.
//   * replays every player's weapons from those updates and flies the bullets
//     with the game's own data and rules (see `combat/`), sending the stock
//     `HitPlayer`/`AffectDamageModel`/`KillPlayer` messages. Client hit
//     reports are ignored.
//   * runs each match's lifecycle (see `lifecycle.rs`): waiting/countdown, the
//     match clock and `match_finished`, kill/death counts and respawn timers.
//   * runs battery collection (see `batteries.rs`): picking batteries up from
//     the client's use key, storing, stealing, dropping, and the early win.
//
// Movement remains client-reported (there is no movement reconciliation).
// The stock client has a fixed 20-player array. Late join/profile replacement
// remains best-effort and may require a client patch (see WORK.md).

use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use session::{LockExt, MAX_PLAYERS_PER_MATCH, MatchResult, SessionStore};
use survarium::player_input::{player_stamina, weapon_state};
use survarium::player_profile::raw::{
    game_team_id, player_profile as raw_player_profile, profile_slot_enum,
};

use vostok::math::float3;

use crate::batteries::{self, Batteries, USE_ACTION, UseTarget};
use crate::combat::math::Vec3;
use crate::combat::system::{Combat, CombatAssets, CombatEvent, Seeds, is_announced};
use crate::combat::traps::TrapChange;
use crate::console::{self, Command};
use crate::game::{self, SpawnState, WeaponLoad};
use crate::lifecycle::{MatchRules, MatchState, Phase, seconds_left};
use crate::map;
use crate::message::{
    BodyPartState, ClientGameMessageKind, PlayerHit, PlayerKill, ServerGameMessageKind,
    VictoryItemAction,
};
use crate::supplies::{Life, Supplies};
use crate::transport::Transport;

const FRAMES_PER_SECOND: u64 = 120;
const FRAME_DURATION: Duration = Duration::from_millis(1000 / FRAMES_PER_SECOND);
/// Standalone clients without a lobby assignment share the historic match.
const DEFAULT_MATCH_ID: u32 = 0x123;
/// Drop a peer we haven't heard from for this long (the client sends keepalives
/// and updates far more often), so dead clients don't linger in the roster.
/// Generous on purpose: a reaped peer cannot resume its connection, and the
/// stock client goes quiet while its window is stalled (alt-tab, loading
/// hitches). A clean client disconnect is still reaped immediately.
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a client's list of players waits for the others the lobby placed
/// in its match at the same time (see `answer_startup_requests`).
const ROSTER_GRACE: Duration = Duration::from_secs(10);
/// Unsent messages past which a client in a match is taken to have stopped
/// reading (its game froze, e.g. while alt-tabbed). The whole backlog would
/// overflow its pool of 8192 packets when it wakes up (see
/// `transport::MAX_UNREPORTED`); the moves and timers in it go instead
/// (`catch_up_stalled_peers`). A few seconds of a full match.
const STALL_BACKLOG: usize = 2048;
/// The client drops the use bit for an update or a few (33-100 ms) while the
/// key is held, so the key only counts as released after this long without it.
const USE_RELEASE_DELAY: Duration = Duration::from_millis(200);
/// A standing player's height, for the respawn zones: the enemy zones start
/// above the base floor, so a player's feet alone are never in them.
const BODY_HEIGHT: f32 = 1.8;
/// Cap on total peer entries (registered + still-connecting), to bound the work
/// a flood of unknown source addresses can create.
const PEER_LIMIT: usize = 64;
/// Test switch: hold every incoming datagram this long, adding it to every
/// client's round trip, to try lag compensation on one machine. At most
/// `MAX_SIMULATED_LATENCY`, with at most `MAX_HELD_DATAGRAMS` held.
const SIMULATED_LATENCY_VARIABLE: &str = "SRP_SIM_LATENCY_MS";
const MAX_SIMULATED_LATENCY: Duration = Duration::from_secs(5);
const MAX_HELD_DATAGRAMS: usize = 16_384;
/// Test switch: drop this percentage (at most `MAX_SIMULATED_LOSS`) of the
/// datagrams sent to every client, to try the retransmission on one machine.
const SIMULATED_LOSS_VARIABLE: &str = "SRP_SIM_LOSS_PERCENT";
const MAX_SIMULATED_LOSS: u64 = 90;
/// The client reads body part names into a 16-byte buffer.
const MAX_BODY_PART_NAME: usize = 15;
/// `affect_event_type_enum`.
const AFFECT_APPLYING: u32 = 0;
const AFFECT_RECALLING: u32 = 1;

struct Peer {
    transport: Transport,
    /// `false` until the peer's `ConnectionRequest` assigns it a player id.
    registered: bool,
    session_id: Option<u32>,
    match_id: u32,
    player_id: u8,
    team: game_team_id,
    name: String,
    /// Exact lobby-selected equipment for this match connection.
    profile: raw_player_profile,
    /// `true` once the peer has sent `JoinMatch` (is in the match).
    joined: bool,
    /// The player ids whose profiles this client got in its startup roster,
    /// in that order. The client numbers its players by their place here, and
    /// has no others (see `ServerGameMessageKind::for_client`).
    roster: Vec<u8>,
    alive: bool,
    respawn_at: Option<Instant>,
    /// Respawn countdown (seconds) last shown to this (dead) player.
    respawn_announced: Option<u32>,
    /// Scoreboard, counted only while the match is in process.
    kills: u32,
    deaths: u32,
    /// Batteries taken from the enemy base, counted like the scoreboard.
    steals: u32,
    /// Whether its match result went to the store (once per match).
    result_recorded: bool,
    /// What its stacks have left for the rest of the match.
    supplies: Supplies,
    /// What its current life was given (its spawn message says so).
    life: Option<Life>,
    /// Index into `map::RESPAWN_POINTS`, assigned at every (re)spawn.
    spawn_index: usize,
    /// The weapons' random seeds, new at every (re)spawn: the spawn message
    /// gives them to the clients and combat replays the spread from them.
    weapon_seeds: Seeds,
    /// Last position (feet) the client reported since its (re)spawn.
    position: Option<float3>,
    /// The yaw and look pitch of that report.
    facing: Option<(f32, f32)>,
    /// When its client asked for its startup info, until it is answered (see
    /// `answer_startup_requests`).
    startup_asked: Option<Instant>,
    /// The client's action mask in its last update (logged when it changes).
    action_mask: u32,
    /// Whether the use key is held (see `track_use_key`).
    use_down: bool,
    /// Since when the use bit has been missing while the key counts as held.
    use_missing_since: Option<Instant>,
    /// What the held use key last acted on (a held key uses each new target
    /// once, like the client's `detect_usable_objects`).
    use_target: Option<UseTarget>,
    /// `true` once `match_finished` is queued for this client: it unloads the
    /// world then, and handles what still comes against freed state, so
    /// nothing more is sent to it.
    match_finished: bool,
    /// Whether this client has been given the match's battery state.
    batteries_sent: bool,
    /// When we last received a datagram from this peer (for the idle timeout).
    last_seen: Instant,
    /// When the answer to its last time sync request was queued, until the
    /// client confirms it.
    sync_answered: Option<Instant>,
    /// The players whose damage models follow their spawns to this client
    /// once its clock is known (see `send_damage_model`).
    owed_damage_models: Vec<SocketAddr>,
    /// Since when its client has been asked to clear its world
    /// (`world_synchronization_request`), until it confirms; meanwhile
    /// nothing about the world is sent to it (see `request_world_sync`).
    world_sync_requested: Option<Instant>,
}

impl Peer {
    fn new() -> Self {
        Self {
            transport: Transport::new(),
            registered: false,
            session_id: None,
            match_id: DEFAULT_MATCH_ID,
            player_id: 0,
            team: game_team_id::team_1,
            name: String::new(),
            profile: raw_player_profile::new_dummy(0, 0, "demo_player"),
            joined: false,
            roster: Vec::new(),
            alive: true,
            respawn_at: None,
            respawn_announced: None,
            kills: 0,
            deaths: 0,
            steals: 0,
            result_recorded: false,
            supplies: Supplies::default(),
            life: None,
            spawn_index: 0,
            weapon_seeds: [(0, 1); 2],
            position: None,
            facing: None,
            startup_asked: None,
            action_mask: 0,
            use_down: false,
            use_missing_since: None,
            use_target: None,
            match_finished: false,
            batteries_sent: false,
            last_seen: Instant::now(),
            sync_answered: None,
            owed_damage_models: Vec::new(),
            world_sync_requested: None,
        }
    }

    /// Track the use key from one update's use bit, ignoring the client's
    /// brief drops of the bit while the key is held. Returns `Some(is_new_press)`
    /// when the bit is set, `None` when it is not.
    fn track_use_key(&mut self, bit_set: bool, now: Instant) -> Option<bool> {
        if bit_set {
            let is_new_press = !self.use_down;
            self.use_down = true;
            self.use_missing_since = None;
            return Some(is_new_press);
        }
        if self.use_down {
            let missing_since = *self.use_missing_since.get_or_insert(now);
            if now.duration_since(missing_since) >= USE_RELEASE_DELAY {
                self.use_down = false;
                self.use_missing_since = None;
                self.use_target = None;
            }
        }
        None
    }

    /// Where the player stands: its last report, else its spawn point.
    fn position(&self) -> float3 {
        self.position
            .unwrap_or(map::RESPAWN_POINTS[self.spawn_index % map::RESPAWN_POINTS.len()].position)
    }
}

pub struct MatchServer {
    socket: UdpSocket,
    store: Arc<SessionStore>,
    peers: HashMap<SocketAddr, Peer>,
    spawn_rng: SpawnRng,
    rules: MatchRules,
    /// Lifecycle of every match with at least one joined player.
    matches: HashMap<u32, MatchState>,
    /// Batteries of every match in `matches`.
    batteries: HashMap<u32, Batteries>,
    /// Weapons, bullets and damage of every joined player; `None` without the
    /// game data (nobody can be hurt then).
    combat: Option<Combat>,
    /// `SRP_SIM_LATENCY_MS`, and the datagrams it holds back (in arrival order).
    simulated_latency: Duration,
    held_datagrams: VecDeque<(Instant, SocketAddr, Vec<u8>)>,
    /// `SRP_SIM_LOSS_PERCENT`.
    simulated_loss: u64,
    /// The server clock that stamps relayed player updates.
    started: Instant,
    /// What `report_held_player_ids` last told the matchmaker, per match.
    held_ids_reported: HashMap<u32, u8>,
    /// Test hook: when set, every game message sent is also recorded here.
    recorded: Option<Vec<(SocketAddr, ServerGameMessageKind)>>,
}

fn record(
    recorded: &mut Option<Vec<(SocketAddr, ServerGameMessageKind)>>,
    addr: SocketAddr,
    message: &ServerGameMessageKind,
) {
    if let Some(recorded) = recorded {
        recorded.push((addr, message.clone()));
    }
}

/// Queue `message` for the client at `addr`, numbering the players as that
/// client does. A message about a player it doesn't have is not sent.
fn deliver(
    recorded: &mut Option<Vec<(SocketAddr, ServerGameMessageKind)>>,
    addr: SocketAddr,
    peer: &mut Peer,
    message: ServerGameMessageKind,
) {
    let message_type = message.message_type();
    if peer.match_finished {
        log::trace!(
            "not sending {message_type:?} to player {}: its match has finished",
            peer.player_id
        );
        return;
    }
    if peer.world_sync_requested.is_some() && message.is_world_state() {
        log::trace!(
            "not sending {message_type:?} to player {}: its world is being resynchronized",
            peer.player_id
        );
        return;
    }
    let roster = &peer.roster;
    let Some(message) = message.for_client(|player_id| {
        roster
            .iter()
            .position(|listed| *listed == player_id)
            .map(|client_id| client_id as u8)
    }) else {
        log::trace!(
            "not sending {message_type:?} to player {}: its client doesn't have that player",
            peer.player_id
        );
        return;
    };
    peer.match_finished = message == ServerGameMessageKind::MatchFinished;
    record(recorded, addr, &message);
    peer.transport.send_game_message(message);
}

/// What a client that stopped reading can do without: a player's update
/// carries its whole state (position, look, input), so the next one makes up
/// for any before it, and a timer is sent again each second.
fn made_up_for_later(message: &ServerGameMessageKind) -> bool {
    matches!(
        message,
        ServerGameMessageKind::ServerPlayerInput { .. }
            | ServerGameMessageKind::MatchWaitTimeChanged { .. }
            | ServerGameMessageKind::MatchTimeChanged { .. }
            | ServerGameMessageKind::RespawnTimeChanged { .. }
    )
}

/// The most a `damage_model_state` body can take: a message is at most 250
/// bytes, its type and order id included.
const MAX_DAMAGE_MODEL_SIZE: usize = 247;

/// Damage model parts with only the affects the other clients hear of.
fn state_without_unannounced(mut parts: Vec<BodyPartState>) -> Vec<BodyPartState> {
    for part in &mut parts {
        part.affects.retain(|(affect, _)| is_announced(*affect));
    }
    parts
}

/// A time on a client's clock, in ms, for the wire. A time from before the
/// client started its clock is its start (1, as 0 means "never").
fn on_client_clock(ms: i64) -> u32 {
    ms.clamp(1, i64::from(u32::MAX)) as u32
}

/// Tiny dependency-free PRNG used only to vary demo spawn selection. This is
/// deliberately not suitable for security or authoritative game simulation.
struct SpawnRng(u64);

impl SpawnRng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn next_index(&mut self, upper_bound: usize) -> usize {
        debug_assert!(upper_bound > 0);
        self.next_u64() as usize % upper_bound
    }
}

impl MatchServer {
    pub fn bind(addr: &str, store: Arc<SessionStore>) -> io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
            ^ u64::from(socket.local_addr()?.port());
        let rules = MatchRules::from_env();
        log::info!("match rules: {rules:?}");
        // Unit tests load the game data only where they need it.
        let combat = if cfg!(test) {
            None
        } else {
            CombatAssets::shared().map(Combat::new)
        };
        let simulated_latency = Duration::from_millis(
            std::env::var(SIMULATED_LATENCY_VARIABLE)
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
        )
        .min(MAX_SIMULATED_LATENCY);
        if !simulated_latency.is_zero() {
            log::warn!(
                "{SIMULATED_LATENCY_VARIABLE}: holding every incoming datagram for \
                 {simulated_latency:?}"
            );
        }
        let simulated_loss = std::env::var(SIMULATED_LOSS_VARIABLE)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0_u64)
            .min(MAX_SIMULATED_LOSS);
        if simulated_loss > 0 {
            log::warn!(
                "{SIMULATED_LOSS_VARIABLE}: dropping {simulated_loss}% of the datagrams sent \
                 to every client"
            );
        }
        Ok(Self {
            socket,
            store,
            peers: HashMap::new(),
            spawn_rng: SpawnRng::new(seed),
            rules,
            matches: HashMap::new(),
            batteries: HashMap::new(),
            combat,
            simulated_latency,
            held_datagrams: VecDeque::new(),
            simulated_loss,
            started: Instant::now(),
            held_ids_reported: HashMap::new(),
            recorded: None,
        })
    }

    /// The address the relay is bound to (used by tests to find the ephemeral port).
    #[cfg(test)]
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Run the relay loop forever (never returns; panics are contained by the
    /// caller's supervisor).
    pub fn run(mut self) -> ! {
        let mut buf = [0u8; 2048];
        loop {
            let frame_start = Instant::now();

            self.receive_pending(&mut buf);

            self.reap_peers();
            self.report_held_player_ids();
            let now = Instant::now();
            self.answer_startup_requests(now);
            self.run_console_commands(now);
            self.tick_matches_at(now);
            self.tick_combat_at(now);
            self.respawn_players_at(now);

            // This frame's messages, then keepalives to the idle.
            let now = Instant::now();
            self.catch_up_stalled_peers();
            let socket = &self.socket;
            for (addr, peer) in self.peers.iter_mut() {
                peer.transport.flush(socket, *addr, now);
                peer.transport.maintain(socket, *addr);
            }

            if let Some(remaining) = FRAME_DURATION.checked_sub(frame_start.elapsed()) {
                std::thread::sleep(remaining);
            }
        }
    }

    /// Handle every datagram already waiting on the socket.
    fn receive_pending(&mut self, buf: &mut [u8]) {
        loop {
            match self.socket.recv_from(buf) {
                Ok((len, addr)) if !self.simulated_latency.is_zero() => {
                    if self.held_datagrams.len() >= MAX_HELD_DATAGRAMS {
                        log::warn!("{SIMULATED_LATENCY_VARIABLE}: queue full; dropping a datagram");
                        continue;
                    }
                    let due = Instant::now() + self.simulated_latency;
                    self.held_datagrams
                        .push_back((due, addr, buf[..len].to_vec()));
                }
                Ok((len, addr)) => self.handle_datagram(addr, buf[..len].to_vec()),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                // Windows reports an ICMP "port unreachable" for an earlier send
                // (keepalives to a client that has gone away, for up to
                // IDLE_TIMEOUT) as a reset on a later receive. It names no peer
                // and must not hold back the datagrams queued behind it.
                Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {
                    log::debug!("ignoring a port-unreachable report: {error}");
                }
                Err(error) => {
                    log::error!("recv_from failed: {error}");
                    break;
                }
            }
        }
        let now = Instant::now();
        while let Some((_, addr, datagram)) =
            self.held_datagrams.pop_front_if(|(due, ..)| *due <= now)
        {
            self.handle_datagram(addr, datagram);
        }
    }

    fn handle_datagram(&mut self, addr: SocketAddr, datagram: Vec<u8>) {
        if !self.peers.contains_key(&addr) && self.peers.len() >= PEER_LIMIT {
            log::warn!("peer limit ({PEER_LIMIT}) reached; dropping datagram from {addr}");
            return;
        }

        let now = Instant::now();
        if !self.peers.contains_key(&addr) {
            let mut peer = Peer::new();
            peer.transport
                .simulate_loss(self.simulated_loss, self.spawn_rng.next_u64());
            self.peers.insert(addr, peer);
        }
        let peer = self.peers.get_mut(&addr).expect("inserted above");
        peer.last_seen = now;
        let messages = peer
            .transport
            .handle_datagram(&datagram, &self.socket, addr);
        for message in messages {
            self.handle_game_message_at(addr, message, now);
        }
    }

    #[cfg(test)]
    fn handle_game_message(&mut self, from: SocketAddr, message: ClientGameMessageKind) {
        self.handle_game_message_at(from, message, Instant::now());
    }

    /// One client message, which arrived at `now`.
    fn handle_game_message_at(
        &mut self,
        from: SocketAddr,
        message: ClientGameMessageKind,
        now: Instant,
    ) {
        let is_connection_request =
            matches!(&message, ClientGameMessageKind::ConnectionRequest { .. });
        if !is_connection_request && !self.peers.get(&from).is_some_and(|peer| peer.registered) {
            log::warn!("ignoring pre-registration match message from {from}");
            return;
        }

        match message {
            ClientGameMessageKind::ConnectionRequest { session_id } => {
                self.register_peer(from, session_id);
            }

            ClientGameMessageKind::GetStartupInfo => {
                let match_id = self.peers[&from].match_id;
                if let Some(peer) = self.peers.get_mut(&from)
                    && peer.startup_asked.is_none()
                {
                    peer.startup_asked = Some(now);
                }
                let awaited = self.awaited_sessions(match_id, now);
                if !awaited.is_empty() {
                    log::info!(
                        "match {match_id:#x}: player {}'s list of players waits for sessions \
                         {awaited:#x?} to connect",
                        self.peers[&from].player_id
                    );
                }
                self.answer_startup_requests(now);
            }

            ClientGameMessageKind::JoinMatch => {
                let (match_id, team, joined) = {
                    let peer = &self.peers[&from];
                    (peer.match_id, peer.team, peer.joined)
                };
                if joined {
                    // A resent startup message: joining again would revive and
                    // heal the player.
                    log::debug!("ignoring a repeated JoinMatch from {from}");
                    return;
                }
                let spawn_index = self.select_spawn_index(match_id, team, from);
                let weapon_seeds = self.new_weapon_seeds();
                let Some(peer) = self.peers.get_mut(&from) else {
                    return;
                };
                peer.joined = true;
                peer.alive = true;
                peer.respawn_at = None;
                peer.spawn_index = spawn_index;
                peer.weapon_seeds = weapon_seeds;
                peer.position = None;
                peer.facing = None;
                let player_id = peer.player_id;

                self.spawn_fighter(from, now);
                self.send_to(from, self.spawn_message(match_id, player_id));

                // Let everyone already in the match see the newcomer.
                self.broadcast_to_others(from, self.spawn_message(match_id, player_id));
                self.broadcast_to_others(
                    from,
                    ServerGameMessageKind::PlayerVisibilityChanged {
                        player_id,
                        is_visible: true,
                    },
                );

                // `JoinMatch` is the client's last startup message, so its own
                // player now exists: the match status can safely switch its
                // camera and controls.
                self.join_lifecycle(from, match_id, now);
                self.send_battery_state(from);
            }

            ClientGameMessageKind::TeamBasesInitializeInfo => {
                // Show this client every other player already in the match, as
                // it is now (the client sends this just before `JoinMatch`,
                // once it has loaded). Who is connected it learns from the
                // answer to its first time sync request, sent once its own
                // player is in the world.
                let match_id = self.peers[&from].match_id;
                let mut others: Vec<(u8, SocketAddr)> = self
                    .peers
                    .iter()
                    .filter(|(addr, peer)| {
                        **addr != from && peer.joined && peer.match_id == match_id
                    })
                    .map(|(addr, peer)| (peer.player_id, *addr))
                    .collect();
                others.sort();
                let mut shown = Vec::new();
                for (player_id, addr) in others {
                    // A dead player stays out of its world: `player::insert`
                    // would stand it up, and its spawn comes when it respawns.
                    let Some(spawn) = self.live_spawn_message(from, addr) else {
                        log::debug!("not showing dead player {player_id} to a newcomer");
                        continue;
                    };
                    self.send_to(from, spawn);
                    self.send_damage_model(from, addr);
                    shown.push(player_id);
                }
                // Then their traps, now that their players stand in this
                // client's world (a spawn takes a player's traps away). A dead
                // player's traps stay unseen until its respawn takes them away.
                let traps = self
                    .combat
                    .as_ref()
                    .map(|combat| combat.traps_out(match_id))
                    .unwrap_or_default();
                for trap in traps.into_iter().filter(|t| shown.contains(&t.player_id)) {
                    self.send_to(
                        from,
                        ServerGameMessageKind::GameWorldObjectState {
                            player_id: trap.player_id,
                            slot: trap.slot,
                            index: trap.index,
                            state: trap.state,
                            position: trap.position.into(),
                            angles: trap.angles.into(),
                        },
                    );
                }
            }

            ClientGameMessageKind::ClientPlayerUpdate {
                player_input,
                player_state,
                time_in_ms,
            } => {
                let Some(peer) = self.peers.get_mut(&from) else {
                    return;
                };
                if peer.alive {
                    peer.position = Some(player_state.translation_w);
                    peer.facing = Some((player_state.euler_angle_y, player_state.look_pitch));
                }
                let player_id = peer.player_id;
                let round_trip = peer.transport.round_trip();
                let mut active_slot = game::active_weapon_slot(&peer.profile)
                    .unwrap_or(profile_slot_enum::weapon1_slot);
                if let Some(combat) = self.combat.as_mut() {
                    combat.update(
                        from,
                        &player_input,
                        &player_state,
                        time_in_ms,
                        round_trip,
                        now,
                    );
                    if let Some(slot) = combat.weapon_slot(from) {
                        active_slot = [
                            profile_slot_enum::weapon1_slot,
                            profile_slot_enum::weapon2_slot,
                        ][slot];
                    }
                }
                // Relay: forward this player's movement to everyone else. The
                // client message carries no weapon_state. Receivers switch
                // their copy of a remote player by its input bits and ignore
                // `slot_id`; it still names the slot the player's weapons are on.
                let stamp = now.saturating_duration_since(self.started).as_millis() as u32;
                self.broadcast_to_others(
                    from,
                    ServerGameMessageKind::ServerPlayerInput {
                        // Receivers only order a player's updates by this,
                        // dropping older ones (`player::time_warp`), and a
                        // player id outlives its player there: the server's
                        // clock never goes back for a reused id.
                        time_in_ms: stamp,
                        player_id,
                        player_input,
                        player_state,
                        weapon_state: weapon_state {
                            slot_id: active_slot as u8,
                            ammo_slot_id: 0,
                            state: 0,
                        },
                    },
                );
                self.handle_use_at(from, player_input.action_mask, now);
                self.pay_damage_models(from);
            }

            ClientGameMessageKind::TimeSynchronizationRequest { client_time_ms } => {
                // `network_client::process_sync_response`: the answer is who is
                // connected; the client confirms it and, the first time, starts
                // running its own player.
                let Some(peer) = self.peers.get_mut(&from) else {
                    return;
                };
                peer.sync_answered = Some(now);
                let (match_id, player_id) = (peer.match_id, peer.player_id);
                if let Some(combat) = self.combat.as_mut() {
                    combat.note_sync_request(from, client_time_ms, now);
                }
                self.pay_damage_models(from);
                log::debug!(
                    "player {player_id}: time sync request at client clock {client_time_ms} ms"
                );
                self.send_to(
                    from,
                    ServerGameMessageKind::SyncResponse {
                        is_connected_bitmask: self.connected_bitmask(match_id),
                    },
                );
            }

            ClientGameMessageKind::TimeSynchronizationConfirmation => {
                let Some(peer) = self.peers.get_mut(&from) else {
                    return;
                };
                // The client also confirms the answers it did not ask for.
                if let Some(answered) = peer.sync_answered.take() {
                    log::debug!(
                        "player {}: time sync confirmed {:?} after the answer (ack round trip {:?})",
                        peer.player_id,
                        now.duration_since(answered),
                        peer.transport.round_trip()
                    );
                }
            }

            ClientGameMessageKind::CommitSuicide => self.commit_suicide(from, Instant::now()),

            ClientGameMessageKind::WorldSynchronizationConfirmation => {
                self.resend_world(from, now);
            }

            ClientGameMessageKind::ClientPlayerHit(hit) => {
                // Only a patched client sends these; the server decides hits.
                log::debug!("ignoring a client hit report from {from}: {hit:?}");
            }

            // Kept only for its place in the message order.
            ClientGameMessageKind::Unhandled(_) => {}
        }
    }

    /// New weapon seeds for a (re)spawn: (`random32`, `normal_random`) per
    /// weapon slot.
    fn new_weapon_seeds(&mut self) -> Seeds {
        let mut next = || self.spawn_rng.next_u64();
        [
            (next() as u32, (next() as u32 & 0x7FFF_FFFF) as i32),
            (next() as u32, (next() as u32 & 0x7FFF_FFFF) as i32),
        ]
    }

    /// A (re)spawn's supplies and combat side, at the peer's spawn point and
    /// with its new seeds. What the last life left goes back to the stacks,
    /// and the new one takes from them (`supplies`), as on the client.
    fn spawn_fighter(&mut self, addr: SocketAddr, now: Instant) {
        let leftovers = self
            .combat
            .as_ref()
            .map(|combat| combat.leftovers(addr))
            .unwrap_or_default();
        let combat = self.combat.as_ref();
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        if combat.is_none() {
            // Nothing tracks what a life used: every life gets its loadout.
            peer.supplies = Supplies::default();
        }
        peer.supplies.give_back(&leftovers);
        let life = peer.supplies.next_life(&peer.profile, |dict_id| {
            combat
                .and_then(|combat| combat.spawn_load(dict_id))
                .unwrap_or_else(|| game::fallback_spawn_load(dict_id))
        });
        peer.life = Some(life);

        let (Some(combat), Some(peer)) = (self.combat.as_mut(), self.peers.get(&addr)) else {
            return;
        };
        let point = &map::RESPAWN_POINTS[peer.spawn_index % map::RESPAWN_POINTS.len()];
        let active_slot =
            game::active_weapon_slot(&peer.profile).unwrap_or(profile_slot_enum::weapon1_slot);
        combat.spawn_loaded(
            addr,
            peer.match_id,
            peer.player_id,
            peer.team as u8,
            &life.profile,
            active_slot,
            peer.weapon_seeds,
            Vec3::from(point.position),
            point.yaw,
            now,
            life.loads,
        );
    }

    /// Fly the bullets and send what they did. Once time is up nobody is hurt.
    fn tick_combat_at(&mut self, now: Instant) {
        let Some(combat) = self.combat.as_mut() else {
            return;
        };
        let matches = &self.matches;
        let events = combat.tick(now, |match_id| {
            !matches
                .get(&match_id)
                .is_some_and(|state| state.phase().is_over())
        });
        for event in events {
            self.apply_combat_event(event, now);
        }
    }

    fn apply_combat_event(&mut self, event: CombatEvent, now: Instant) {
        match event {
            CombatEvent::Hit {
                match_id,
                shooter,
                victim,
                body_part,
                hit_type,
                amount,
                armor_piercing,
            } => {
                if body_part.len() > MAX_BODY_PART_NAME || hit_type.len() > MAX_BODY_PART_NAME {
                    log::warn!("{hit_type} hit on `{body_part}`: name too long for the client");
                    return;
                }
                log::debug!(
                    "match {match_id:#x}: player {shooter} hit player {victim} in the \
                     {body_part}: {amount} damage, {armor_piercing} armour piercing"
                );
                // The raw bullet: each client runs its own copy of the victim's
                // armour over it, as the server did.
                self.broadcast_to_match(
                    match_id,
                    ServerGameMessageKind::HitPlayer(PlayerHit {
                        hit_initiator: shooter,
                        being_hit: victim,
                        body_part,
                        damage_type: hit_type,
                        amount,
                        armor_piercing,
                    }),
                );
            }
            CombatEvent::LimbAffect {
                match_id,
                player_id,
                body_part,
                affect,
                applying,
            } => {
                if body_part.len() > MAX_BODY_PART_NAME {
                    log::warn!("affect on `{body_part}`: name too long for the client");
                    return;
                }
                log::debug!(
                    "match {match_id:#x}: player {player_id} {} affect {affect} on the \
                     {body_part}",
                    if applying { "gets" } else { "loses" }
                );
                self.broadcast_to_match(
                    match_id,
                    ServerGameMessageKind::AffectDamageModel {
                        player_id,
                        body_part,
                        affect: u32::from(affect),
                        event: if applying {
                            AFFECT_APPLYING
                        } else {
                            AFFECT_RECALLING
                        },
                    },
                );
            }
            CombatEvent::Kill {
                victim,
                victim_id,
                killer_id,
                is_headshot,
                weapon_dict_id,
                ..
            } => self.kill_player_at(
                victim,
                PlayerKill {
                    victim_id,
                    killer_id,
                    is_headshot,
                    item_dict_id: u32::from(weapon_dict_id),
                },
                now,
            ),
            // Everyone, the owner too: each client inserts the trap and takes
            // one off the owner's set.
            CombatEvent::TrapPlaced {
                match_id,
                player_id,
                slot,
                index,
                position,
                angles,
            } => self.broadcast_to_match(
                match_id,
                ServerGameMessageKind::TrapPlaced {
                    player_id,
                    slot,
                    index,
                    position: position.into(),
                    angles: angles.into(),
                },
            ),
            CombatEvent::TrapChanged {
                match_id,
                player_id,
                slot,
                index,
                change,
            } => {
                log::debug!(
                    "match {match_id:#x}: trap {index} of player {player_id}'s slot {slot:#x}: \
                     {change:?}"
                );
                let message = match change {
                    TrapChange::Fired => ServerGameMessageKind::TrapFired {
                        player_id,
                        slot,
                        index,
                    },
                    TrapChange::Disarmed => ServerGameMessageKind::TrapDisarmed {
                        player_id,
                        slot,
                        index,
                    },
                    TrapChange::Removed => ServerGameMessageKind::TrapRemoved {
                        player_id,
                        slot,
                        index,
                    },
                };
                self.broadcast_to_match(match_id, message);
            }
        }
    }

    /// The player at `addr` killed itself (the console's `suicide`): a death,
    /// with itself as the killer and the weapon it held in the kill feed.
    fn commit_suicide(&mut self, addr: SocketAddr, now: Instant) {
        let Some(peer) = self.peers.get(&addr) else {
            return;
        };
        if !peer.joined || !peer.alive {
            return;
        }
        let (victim_id, profile) = (peer.player_id, peer.profile);
        let held = match self.combat.as_mut() {
            Some(combat) => match combat.commit_suicide(addr) {
                Some(held) => held,
                None => return,
            },
            None => None,
        };
        // Empty hands: a weapon it carries, so the kill feed has an icon.
        let item_dict_id = held.unwrap_or_else(|| {
            [
                profile_slot_enum::weapon1_slot,
                profile_slot_enum::weapon2_slot,
            ]
            .into_iter()
            .map(|slot| profile.slots[slot].dict_id)
            .find(|dict_id| *dict_id != 0)
            .unwrap_or_default()
        });
        log::info!("player {victim_id} ({addr}) committed suicide");
        self.kill_player_at(
            addr,
            PlayerKill {
                victim_id,
                killer_id: victim_id,
                is_headshot: false,
                item_dict_id: u32::from(item_dict_id),
            },
            now,
        );
    }

    /// The player at `addr` died: tell the match, count it, start its respawn
    /// countdown and drop what it carried.
    fn kill_player_at(&mut self, addr: SocketAddr, kill: PlayerKill, now: Instant) {
        let respawn_time = self.rules.respawn_time();
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        if !peer.joined || !peer.alive {
            return;
        }
        peer.alive = false;
        peer.respawn_at = Some(now + respawn_time);
        let match_id = peer.match_id;
        self.broadcast_to_match(match_id, ServerGameMessageKind::KillPlayer(kill.clone()));
        self.record_kill(match_id, &kill, now);
        self.drop_battery(match_id, kill.victim_id);
    }

    fn register_peer(&mut self, addr: SocketAddr, session_id: u32) {
        if self.peers.get(&addr).is_some_and(|peer| peer.registered) {
            log::debug!("ignoring duplicate registration from {addr}");
            return;
        }

        let account = self.store.account_for_session(session_id);
        let mut assignment = self.store.match_assignment(session_id).or_else(|| {
            // A known session may reconnect directly after its previous peer was
            // reaped. Re-enter matchmaking so it can claim any live vacancy.
            account.is_some().then(|| self.store.join_match(session_id))
        });
        let mut match_id = assignment
            .map(|assignment| assignment.match_id)
            .unwrap_or(DEFAULT_MATCH_ID);
        let mut player_id = self.free_player_id(match_id);
        if player_id.is_none()
            && let Some(placed) = assignment
        {
            // Placed before the matchmaker heard which ids this match's
            // clients still have.
            let moved = self
                .store
                .move_to_another_match(session_id, placed.match_id);
            log::info!(
                "match {:#x} has no free player id; session {session_id:#x} moves to match {:#x}",
                placed.match_id,
                moved.match_id
            );
            assignment = Some(moved);
            match_id = moved.match_id;
            player_id = self.free_player_id(match_id);
        }
        let team = match assignment {
            Some(a) if a.team_id == 2 => game_team_id::team_2,
            _ => game_team_id::team_1,
        };
        let Some(player_id) = player_id else {
            log::warn!(
                "match {match_id:#x} has no free player id ({MAX_PLAYERS_PER_MATCH} players, \
                 or ids its clients still have); rejecting session {session_id:#x}"
            );
            self.store.leave_match(session_id);
            // The transport has already acknowledged the connection request.
            // Remove it now so it cannot continue with the default player ID.
            self.peers.remove(&addr);
            return;
        };

        let name = account
            .as_ref()
            .map(|account| account.locked().name.clone())
            .unwrap_or_else(|| format!("player_{player_id}"));
        let profile = self
            .store
            .selected_profile(session_id)
            .or_else(|| {
                account
                    .as_ref()
                    .and_then(|account| account.locked().profile_contents.first().copied())
            })
            .unwrap_or_else(|| {
                raw_player_profile::new_dummy(u32::from(player_id), 0, "demo_player")
            });

        if let Some(peer) = self.peers.get_mut(&addr) {
            peer.registered = true;
            peer.session_id = Some(session_id);
            peer.match_id = match_id;
            peer.player_id = player_id;
            peer.team = team;
            peer.name = name;
            peer.profile = profile;
        }

        log::info!(
            "session {session_id:#x} is player {player_id} in match {match_id:#x} \
             (team {team:?}) at {addr}",
        );
    }

    /// The lowest player id free in a match. An id stays taken while a client
    /// still has the player who left it: a newcomer with it would be read
    /// there as that player, against the old player's equipment.
    fn free_player_id(&self, match_id: u32) -> Option<u8> {
        (0..MAX_PLAYERS_PER_MATCH).find(|candidate| {
            !self.peers.values().any(|peer| {
                peer.registered
                    && peer.match_id == match_id
                    && (peer.player_id == *candidate || peer.roster.contains(candidate))
            })
        })
    }

    /// Tell the matchmaker, for every match whose count changed, how many
    /// player ids its clients still have for players who left, so it places
    /// no one in a match without a free id.
    fn report_held_player_ids(&mut self) {
        let mut held: HashMap<u32, Vec<u8>> = HashMap::new();
        for peer in self.peers.values().filter(|peer| peer.registered) {
            held.entry(peer.match_id)
                .or_default()
                .extend(peer.roster.iter().copied());
        }
        for peer in self.peers.values().filter(|peer| peer.registered) {
            if let Some(ids) = held.get_mut(&peer.match_id) {
                ids.retain(|id| *id != peer.player_id);
            }
        }
        let counts: HashMap<u32, u8> = held
            .into_iter()
            .map(|(match_id, mut ids)| {
                ids.sort_unstable();
                ids.dedup();
                (match_id, ids.len() as u8)
            })
            .collect();
        for (match_id, count) in &counts {
            if self.held_ids_reported.get(match_id) != Some(count) {
                self.store.set_held_player_ids(*match_id, *count);
            }
        }
        self.held_ids_reported = counts;
    }

    /// A respawn point for the `team` player at `spawning` in this match, by
    /// the rule the map's data spells out (the original code is not
    /// available): a living teammate in a point's ally zones, or a living
    /// enemy in its enemy zones, closes it. The open points of the highest
    /// priority are picked from at random, so a team respawns at its base
    /// unless an enemy is in it, then at the "safe" points behind it. With
    /// every point closed, any of the highest priority.
    /// Indexes into `map::RESPAWN_POINTS`.
    fn select_spawn_index(
        &mut self,
        match_id: u32,
        team: game_team_id,
        spawning: SocketAddr,
    ) -> usize {
        let mut points = game::team_respawn_points(team);
        if points.is_empty() {
            // Not a playing team (never assigned): use the first team's points.
            points = game::team_respawn_points(game_team_id::team_1);
        }
        let others: Vec<(game_team_id, float3)> = self
            .peers
            .iter()
            .filter(|(addr, peer)| {
                **addr != spawning && peer.joined && peer.alive && peer.match_id == match_id
            })
            .map(|(_, peer)| (peer.team, peer.position()))
            .collect();
        let is_open = |index: &usize| {
            let point = &map::RESPAWN_POINTS[*index];
            !others.iter().any(|(other_team, feet)| {
                let zones = if *other_team == team {
                    point.ally_zones
                } else {
                    point.enemy_zones
                };
                zones
                    .iter()
                    .any(|zone| zone.touches_body(*feet, BODY_HEIGHT))
            })
        };
        let open: Vec<usize> = points.iter().copied().filter(is_open).collect();
        let pool = if open.is_empty() { &points } else { &open };
        let priority = |index: &usize| map::RESPAWN_POINTS[*index].priority;
        let Some(best) = pool.iter().map(priority).max() else {
            return 0;
        };
        let candidates: Vec<usize> = pool
            .iter()
            .copied()
            .filter(|index| priority(index) == best)
            .collect();
        if points.iter().map(priority).max() != Some(best) {
            log::info!(
                "{spawning} respawns behind the base: every base point has an enemy in the \
                 base or a teammate on it"
            );
        }
        candidates[self.spawn_rng.next_index(candidates.len())]
    }

    /// A `SpawnPlayer` for `player_id` at its assigned respawn point.
    fn spawn_message(&self, match_id: u32, player_id: u8) -> ServerGameMessageKind {
        let peer = self.peers.values().find(|peer| {
            peer.registered && peer.match_id == match_id && peer.player_id == player_id
        });
        let spawn_index = peer.map_or(player_id as usize, |peer| peer.spawn_index);
        let life = peer.and_then(|peer| peer.life);
        let profile = match (peer, life) {
            (_, Some(life)) => life.profile,
            (Some(peer), None) => peer.profile,
            (None, None) => raw_player_profile::new_dummy(u32::from(player_id), 0, "demo_player"),
        };
        let seeds = peer.map_or([(0, 1); 2], |peer| peer.weapon_seeds);
        let combat = self.combat.as_ref();
        ServerGameMessageKind::SpawnPlayer {
            player_id,
            player: game::spawn_content(spawn_index, &profile, seeds, |dict_id| {
                life.and_then(|life| life.load_of(dict_id))
                    .or_else(|| combat.and_then(|combat| combat.spawn_load(dict_id)))
            }),
        }
    }

    /// A `SpawnPlayer` showing the living player at `subject` as it is now to
    /// the client at `to`, which doesn't have it in its world yet: where it
    /// last stood and looked, the weapon in its hands, what its weapons and
    /// stacks hold and its stamina. `None` for a dead player.
    ///
    /// `player::deserialize` reads it as any spawn: it puts the player in the
    /// world (`insert`, which also resets its damage model and stamina), then
    /// sets the active weapon, the stamina and every item's state from it.
    fn live_spawn_message(
        &self,
        to: SocketAddr,
        subject: SocketAddr,
    ) -> Option<ServerGameMessageKind> {
        let peer = self
            .peers
            .get(&subject)
            .filter(|peer| peer.joined && peer.alive)?;
        let point = &map::RESPAWN_POINTS[peer.spawn_index % map::RESPAWN_POINTS.len()];
        let profile = peer.life.map_or(peer.profile, |life| life.profile);
        let combat = self.combat.as_ref();
        let kit = combat.and_then(|combat| combat.kit(subject));
        let (yaw, look_pitch) = peer.facing.unwrap_or((point.yaw, game::SPAWN_LOOK_PITCH));
        let weapon_slots = [
            profile_slot_enum::weapon1_slot,
            profile_slot_enum::weapon2_slot,
        ];
        let active_slot =
            game::active_weapon_slot(&profile).unwrap_or(profile_slot_enum::weapon1_slot);
        let in_hand = kit
            .as_ref()
            .map(|kit| weapon_slots[kit.weapon_slot])
            .filter(|slot| profile.slots[*slot].id != 0)
            .unwrap_or(active_slot);
        // A weapon combat doesn't run is shown as its life gave it.
        let weapons = std::array::from_fn(|i| {
            kit.as_ref().and_then(|kit| kit.weapons[i]).or_else(|| {
                let item = profile.slots[weapon_slots[i]];
                (item.id != 0).then(|| WeaponLoad {
                    seeds: peer.weapon_seeds[i],
                    load: peer
                        .life
                        .and_then(|life| life.load_of(item.dict_id))
                        .or_else(|| combat.and_then(|combat| combat.spawn_load(item.dict_id)))
                        .unwrap_or_else(|| game::fallback_spawn_load(item.dict_id)),
                    ammo_slot: None,
                    fire_queue_type: 0,
                })
            })
        });
        let stamina = kit.as_ref().map_or(game::SPAWN_STAMINA, |kit| {
            self.stamina_for(subject, to, kit.stamina)
        });
        let state = SpawnState {
            position: peer.position(),
            yaw,
            look_pitch,
            active_slot,
            target_slot: in_hand,
            stamina,
            weapons,
            amounts: kit.map(|kit| kit.amounts).unwrap_or_default(),
        };
        Some(ServerGameMessageKind::SpawnPlayer {
            player_id: peer.player_id,
            player: game::player_content(&profile, &state),
        })
    }

    /// The damage model of the living player at `subject` for the client at
    /// `to`, which has its spawn: each part's health, last hit and affects,
    /// the times on that client's clock (a remote player's model regenerates
    /// on the receiver's clock, `player::tick`). The player's own client gets
    /// every affect; the others only those they hear come and go
    /// (`is_announced`). `None` without combat, for a dead player, or while
    /// the clock of `to` is unknown.
    fn damage_model_message(
        &self,
        to: SocketAddr,
        subject: SocketAddr,
    ) -> Option<ServerGameMessageKind> {
        let combat = self.combat.as_ref()?;
        let peer = self
            .peers
            .get(&subject)
            .filter(|peer| peer.joined && peer.alive)?;
        let state = combat.damage_state(subject)?;
        let offset = combat.client_offset(to)?;
        let on_clock = |server_ms: u32| on_client_clock(i64::from(server_ms) - offset);
        let mut parts: Vec<BodyPartState> = state
            .into_iter()
            .map(|part| BodyPartState {
                health: part.health,
                last_hit_time_ms: match part.last_hit_ms {
                    0 => 0,
                    last_hit_ms => on_clock(last_hit_ms),
                },
                affects: part
                    .affects
                    .into_iter()
                    .filter(|(affect, _)| to == subject || is_announced(*affect))
                    .map(|(affect, started_ms)| (affect, on_clock(started_ms)))
                    .collect(),
            })
            .collect();
        // Player id, then 9 bytes a part and 5 an affect.
        let size = 1 + parts
            .iter()
            .map(|part| 9 + 5 * part.affects.len())
            .sum::<usize>();
        if size > MAX_DAMAGE_MODEL_SIZE {
            log::warn!(
                "player {}'s damage model takes {size} bytes; sending only its limb affects",
                peer.player_id
            );
            parts = state_without_unannounced(parts);
        }
        Some(ServerGameMessageKind::DamageModelState {
            player_id: peer.player_id,
            parts,
        })
    }

    /// Give the client at `to` the damage model of the living player at
    /// `subject` (`damage_model_state`), right after that player's spawn,
    /// which resets it (`insert_alive`). A newcomer's clock is unknown until
    /// its first time sync request, which it sends once its own player is in
    /// the world: until then the model is owed, and goes out (as it is then)
    /// with that request.
    fn send_damage_model(&mut self, to: SocketAddr, subject: SocketAddr) {
        let Some(combat) = self.combat.as_ref() else {
            return;
        };
        if combat.client_offset(to).is_none() {
            if let Some(peer) = self.peers.get_mut(&to)
                && !peer.owed_damage_models.contains(&subject)
            {
                peer.owed_damage_models.push(subject);
            }
            return;
        }
        if let Some(message) = self.damage_model_message(to, subject) {
            self.send_to(to, message);
        }
    }

    /// The damage models owed to the client at `addr`, once its clock is
    /// known. A player who died or left since is skipped.
    fn pay_damage_models(&mut self, addr: SocketAddr) {
        let known = self
            .combat
            .as_ref()
            .is_some_and(|combat| combat.client_offset(addr).is_some());
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        if !known || peer.owed_damage_models.is_empty() {
            return;
        }
        for subject in std::mem::take(&mut peer.owed_damage_models) {
            self.send_damage_model(addr, subject);
        }
    }

    /// `stamina` of the player at `subject` (times on its client's clock) as
    /// the client at `to` reads it. The client ticks every player's stamina
    /// on its own clock (`player::tick`): the first tick after a spawn
    /// regenerates or spends for the time since the last tick sent. With
    /// either clock unknown, the times are the spawn's (long past).
    fn stamina_for(
        &self,
        subject: SocketAddr,
        to: SocketAddr,
        stamina: player_stamina,
    ) -> player_stamina {
        let times = self
            .time_for(subject, to, stamina.last_spending_time_in_ms)
            .zip(self.time_for(subject, to, stamina.last_tick_time_in_ms));
        let Some((last_spending_time_in_ms, last_tick_time_in_ms)) = times else {
            return player_stamina {
                last_spending_time_in_ms: game::SPAWN_STAMINA.last_spending_time_in_ms,
                last_tick_time_in_ms: game::SPAWN_STAMINA.last_tick_time_in_ms,
                ..stamina
            };
        };
        player_stamina {
            last_spending_time_in_ms,
            last_tick_time_in_ms,
            ..stamina
        }
    }

    /// `time` on the clock of the client at `from` as the client at `to`
    /// reads its own (0, "never", stays 0); `None` while either clock is
    /// unknown.
    fn time_for(&self, from: SocketAddr, to: SocketAddr, time: u32) -> Option<u32> {
        if from == to || time == 0 {
            return Some(time);
        }
        let combat = self.combat.as_ref()?;
        let server_ms = i64::from(time) + combat.client_offset(from)?;
        Some(on_client_clock(server_ms - combat.client_offset(to)?))
    }

    /// Answer each startup request (`GetStartupInfo`) that can be answered.
    /// The client builds its roster of the match's players from the answer,
    /// once: nothing adds a player to it later, and messages about a player
    /// it lacks are dropped (see `ServerGameMessageKind::for_client`). So a
    /// request waits while a player the lobby placed in the same match in the
    /// last `ROSTER_GRACE` has not connected yet: the queue sends its players
    /// to a new match at once, and each hears of it at its next status
    /// request, up to a second apart. It waits at most `ROSTER_GRACE`.
    fn answer_startup_requests(&mut self, now: Instant) {
        let asking: Vec<(SocketAddr, u32, Instant)> = self
            .peers
            .iter()
            .filter_map(|(addr, peer)| Some((*addr, peer.match_id, peer.startup_asked?)))
            .collect();
        for (addr, match_id, asked) in asking {
            let waited = now.saturating_duration_since(asked);
            if waited < ROSTER_GRACE && !self.awaited_sessions(match_id, now).is_empty() {
                continue;
            }
            if let Some(peer) = self.peers.get_mut(&addr) {
                peer.startup_asked = None;
            }
            if !waited.is_zero() {
                log::info!(
                    "match {match_id:#x}: sending {addr} its list of players after {waited:?}"
                );
            }
            self.send_startup_info(addr);
        }
    }

    /// The sessions placed in `match_id` in the last `ROSTER_GRACE` that have
    /// not connected to it yet.
    fn awaited_sessions(&self, match_id: u32, now: Instant) -> Vec<u32> {
        self.store
            .placements(match_id)
            .into_iter()
            .filter(|(_, placed)| now.saturating_duration_since(*placed) < ROSTER_GRACE)
            .map(|(session_id, _)| session_id)
            .filter(|session_id| {
                !self
                    .peers
                    .values()
                    .any(|peer| peer.registered && peer.session_id == Some(*session_id))
            })
            .collect()
    }

    /// A client's startup info: the match's options, then the profile of
    /// each player registered in it, its own marked local. This is its roster.
    fn send_startup_info(&mut self, to: SocketAddr) {
        // Tell this client about the whole current roster: how many players
        // to expect, then a profile per player (its own marked is_local).
        let match_id = self.peers[&to].match_id;
        let mut roster: Vec<(u8, game_team_id, String, raw_player_profile, bool)> = self
            .peers
            .iter()
            .filter(|(_, peer)| peer.registered && peer.match_id == match_id)
            .map(|(addr, peer)| {
                (
                    peer.player_id,
                    peer.team,
                    peer.name.clone(),
                    peer.profile,
                    *addr == to,
                )
            })
            .collect();
        roster.sort_by_key(|(player_id, ..)| *player_id);
        if let Some(peer) = self.peers.get_mut(&to) {
            peer.roster = roster.iter().map(|(id, ..)| *id).collect();
        }

        self.send_to(to, game::match_options(roster.len() as u8, &self.rules));
        for (client_id, (_, team, name, profile, is_local)) in roster.into_iter().enumerate() {
            self.send_to(
                to,
                ServerGameMessageKind::PlayerProfile {
                    player_profile: Box::new(game::player_profile(
                        profile,
                        client_id as u8,
                        team,
                        &name,
                        is_local,
                    )),
                },
            );
        }
    }

    /// Bitmask of registered (connected) players, one bit per player id.
    fn connected_bitmask(&self, match_id: u32) -> u32 {
        self.peers
            .values()
            .filter(|peer| peer.registered && peer.match_id == match_id)
            .fold(0, |mask, peer| mask | (1 << peer.player_id))
    }

    /// Show each dead player its respawn countdown, once per displayed second,
    /// and revive every due player at a newly selected team respawn point.
    fn respawn_players_at(&mut self, now: Instant) {
        let mut countdowns = Vec::new();
        for (addr, peer) in &mut self.peers {
            if let (true, false, Some(respawn_at)) = (peer.joined, peer.alive, peer.respawn_at) {
                let time_left = seconds_left(respawn_at, now);
                if time_left > 0 && peer.respawn_announced != Some(time_left) {
                    peer.respawn_announced = Some(time_left);
                    countdowns.push((*addr, time_left));
                }
            }
        }
        for (addr, time_left) in countdowns {
            self.send_to(
                addr,
                ServerGameMessageKind::RespawnTimeChanged { time_left },
            );
        }

        let due: Vec<(SocketAddr, u32, u8, game_team_id)> = self
            .peers
            .iter()
            .filter(|(_, peer)| {
                peer.registered
                    && peer.joined
                    && !peer.alive
                    && peer.respawn_at.is_some_and(|deadline| deadline <= now)
            })
            .map(|(addr, peer)| (*addr, peer.match_id, peer.player_id, peer.team))
            .collect();

        for (addr, match_id, player_id, team) in due {
            let spawn_index = self.select_spawn_index(match_id, team, addr);
            let weapon_seeds = self.new_weapon_seeds();
            let Some(peer) = self.peers.get_mut(&addr) else {
                continue;
            };
            peer.alive = true;
            peer.respawn_at = None;
            peer.respawn_announced = None;
            peer.spawn_index = spawn_index;
            peer.weapon_seeds = weapon_seeds;
            peer.position = None;
            peer.facing = None;
            self.spawn_fighter(addr, now);

            self.send_to(
                addr,
                ServerGameMessageKind::RespawnTimeChanged { time_left: 0 },
            );
            let spawn = self.spawn_message(match_id, player_id);
            self.broadcast_to_match(match_id, spawn);
        }
    }

    fn send_to(&mut self, addr: SocketAddr, message: ServerGameMessageKind) {
        if let Some(peer) = self.peers.get_mut(&addr) {
            deliver(&mut self.recorded, addr, peer, message);
        }
    }

    fn broadcast_to_others(&mut self, except: SocketAddr, message: ServerGameMessageKind) {
        let Some(match_id) = self.peers.get(&except).map(|peer| peer.match_id) else {
            return;
        };
        for (addr, peer) in self.peers.iter_mut() {
            if *addr != except && peer.joined && peer.match_id == match_id {
                deliver(&mut self.recorded, *addr, peer, message.clone());
            }
        }
    }

    fn broadcast_to_match(&mut self, match_id: u32, message: ServerGameMessageKind) {
        for (addr, peer) in self.peers.iter_mut() {
            if peer.joined && peer.match_id == match_id {
                deliver(&mut self.recorded, *addr, peer, message.clone());
            }
        }
    }

    /// Drop peers that have disconnected or gone silent, and tell the rest who is
    /// left.
    fn reap_peers(&mut self) {
        let now = Instant::now();
        let gone: Vec<SocketAddr> = self
            .peers
            .iter()
            .filter(|(_, peer)| {
                peer.transport.is_disconnected()
                    || now.duration_since(peer.last_seen) > IDLE_TIMEOUT
            })
            .map(|(addr, _)| *addr)
            .collect();

        if gone.is_empty() {
            return;
        }

        let mut departures = Vec::new();
        for addr in gone {
            self.record_result(addr, false);
            if let Some(combat) = self.combat.as_mut() {
                combat.remove(addr);
            }
            if let Some(peer) = self.peers.remove(&addr) {
                log::info!("player {} ({addr}) disconnected", peer.player_id);
                departures.push((
                    peer.match_id,
                    peer.player_id,
                    peer.joined,
                    peer.session_id,
                    peer.position(),
                ));
            }
        }

        for (match_id, player_id, was_joined, session_id, position) in departures {
            if let Some(session_id) = session_id
                && !self
                    .peers
                    .values()
                    .any(|peer| peer.registered && peer.session_id == Some(session_id))
            {
                self.store.leave_match(session_id);
            }
            let gone = ServerGameMessageKind::PlayerVisibilityChanged {
                player_id,
                is_visible: false,
            };
            if was_joined {
                // Before the player disappears: the client drops the battery
                // at its copy of the player.
                self.drop_battery_at(match_id, player_id, position);
            }
            // Only the clients whose roster listed it have it (a player who
            // left before joining has a row on their scoreboard).
            self.broadcast_to_match(match_id, gone);
        }

        // Refresh each match's connected bitmask after releasing departed peers.
        let bitmasks: HashMap<u32, u32> = self.peers.values().filter(|peer| peer.registered).fold(
            HashMap::new(),
            |mut bitmasks, peer| {
                *bitmasks.entry(peer.match_id).or_default() |= 1 << peer.player_id;
                bitmasks
            },
        );
        for (addr, peer) in self.peers.iter_mut() {
            if peer.joined {
                let message = ServerGameMessageKind::SyncResponse {
                    is_connected_bitmask: bitmasks.get(&peer.match_id).copied().unwrap_or_default(),
                };
                deliver(&mut self.recorded, *addr, peer, message);
            }
        }
    }

    /// Open (or reopen, after it finished) this match's lifecycle for a newly
    /// joined peer, then bring the newcomer up to date with the current phase.
    fn join_lifecycle(&mut self, from: SocketAddr, match_id: u32, now: Instant) {
        if self
            .matches
            .get(&match_id)
            .is_none_or(|state| state.phase().is_finished())
        {
            // Opening is a phase change too: announce the first phase (or the
            // one it immediately advances to) before any timer update.
            let joined = self
                .peers
                .values()
                .filter(|peer| peer.registered && peer.joined && peer.match_id == match_id)
                .count();
            let mut state = MatchState::new(now, &self.rules);
            let mut entered = vec![state.phase()];
            entered.extend(state.advance(
                now,
                &self.rules,
                joined,
                usize::from(MAX_PLAYERS_PER_MATCH),
            ));
            let spawn_rng = &mut self.spawn_rng;
            let batteries = Batteries::spawn(self.rules.battery_count, |n| spawn_rng.next_index(n));
            log::info!(
                "match {match_id:#x}: opened; batteries {}",
                batteries.describe()
            );
            self.batteries.insert(match_id, batteries);
            self.matches.insert(match_id, state);
            self.enter_phases(match_id, &entered, now);
            return;
        }

        // A phase change is announced to every joined peer, newcomer included.
        if self.tick_match_at(match_id, now) {
            return;
        }
        self.send_match_status(from, match_id, now);
    }

    /// The match's phase and timer, and while it is in process its
    /// scoreboard, for one client.
    fn send_match_status(&mut self, to: SocketAddr, match_id: u32, now: Instant) {
        let Some(state) = self.matches.get(&match_id) else {
            return;
        };
        let in_process = state.phase().is_in_process();
        for message in state.announcement(now) {
            self.send_to(to, message);
        }
        if in_process {
            for message in self.kd_messages(match_id) {
                self.send_to(to, message);
            }
        }
    }

    /// Clients in a match that stopped reading (`STALL_BACKLOG`): drop the
    /// queued messages that later ones make up for (`made_up_for_later`),
    /// most of the backlog. Everything that changes the world stays, in
    /// order. A world resync would also do, but the stock client crashes on
    /// it (see `request_world_sync`).
    fn catch_up_stalled_peers(&mut self) {
        for (addr, peer) in self.peers.iter_mut() {
            if !peer.joined || peer.match_finished || peer.transport.queued_len() <= STALL_BACKLOG {
                continue;
            }
            let waiting = peer.transport.queued_len();
            let dropped = peer.transport.drop_queued(made_up_for_later);
            if dropped > 0 {
                log::info!(
                    "match {:#x}: player {} ({addr}) stopped reading; dropping the {dropped} moves and timers of the {waiting} messages waiting for it",
                    peer.match_id,
                    peer.player_id
                );
            }
        }
    }

    fn tick_matches_at(&mut self, now: Instant) {
        let match_ids: Vec<u32> = self.matches.keys().copied().collect();
        for match_id in match_ids {
            self.tick_match_at(match_id, now);
        }
    }

    /// Advance one match's clock: announce phase changes and each displayed
    /// timer second, and close the match once nobody is left. Returns whether a
    /// phase change was announced.
    fn tick_match_at(&mut self, match_id: u32, now: Instant) -> bool {
        let in_match = |peer: &&Peer| peer.registered && peer.match_id == match_id;
        let registered = self.peers.values().filter(in_match).count();
        let joined = self
            .peers
            .values()
            .filter(in_match)
            .filter(|peer| peer.joined)
            .count();
        if registered == 0 {
            self.batteries.remove(&match_id);
            if self.matches.remove(&match_id).is_some() {
                log::info!("match {match_id:#x}: closed (no players left)");
            }
            return false;
        }

        let rules = self.rules;
        let Some(state) = self.matches.get_mut(&match_id) else {
            return false;
        };
        if let Phase::Finished { at } = state.phase() {
            if now >= at + rules.finished_grace() {
                self.close_finished_match(match_id);
            }
            return false;
        }

        let entered = state.advance(now, &rules, joined, usize::from(MAX_PLAYERS_PER_MATCH));
        if entered.is_empty() {
            if let Some(timer) = state.timer_tick(now) {
                log::debug!("match {match_id:#x}: {timer:?}");
                self.broadcast_to_match(match_id, timer);
            }
            return false;
        }
        self.enter_phases(match_id, &entered, now);
        true
    }

    /// Announce the newest of `entered` (the match state is already in it) and
    /// apply every entered phase's side effects in order.
    fn enter_phases(&mut self, match_id: u32, entered: &[Phase], now: Instant) {
        let Some(state) = self.matches.get_mut(&match_id) else {
            return;
        };
        let time_left = state
            .seconds_left(now)
            .map(|seconds| format!(" ({}:{:02} left)", seconds / 60, seconds % 60))
            .unwrap_or_default();
        let names: Vec<&str> = entered.iter().map(Phase::name).collect();
        log::info!("match {match_id:#x}: {}{time_left}", names.join(" -> "));

        let announcement = state.announcement(now);
        state.mark_announced(now);
        for message in announcement {
            self.broadcast_to_match(match_id, message);
        }
        for phase in entered {
            match phase {
                Phase::InProcess { .. } => self.start_match(match_id, now),
                Phase::TimeUp { .. } => self.finish_match(match_id),
                Phase::WaitingForPlayers { .. }
                | Phase::FinalCountdown { .. }
                | Phase::Finished { .. } => {}
            }
        }
    }

    /// The match clock started: clear the scoreboard and revive the dead (the
    /// respawn runs after the `inprocess` status, so it gives them control).
    fn start_match(&mut self, match_id: u32, now: Instant) {
        let mut players = 0;
        for (addr, peer) in self
            .peers
            .iter_mut()
            .filter(|(_, peer)| peer.registered && peer.match_id == match_id)
        {
            peer.kills = 0;
            peer.deaths = 0;
            peer.steals = 0;
            // What was used before the start isn't the match's.
            if let Some(combat) = self.combat.as_mut() {
                combat.take_used(*addr);
            }
            if peer.joined {
                players += 1;
                if !peer.alive {
                    peer.respawn_at = Some(now);
                }
            }
        }
        log::info!("match {match_id:#x}: started with {players} player(s)");
        for message in self.kd_messages(match_id) {
            self.broadcast_to_match(match_id, message);
        }
    }

    /// Time is up: freeze respawns and free every player's matchmaking slot,
    /// so the next "ready" gets a new match. `match_finished` follows when the
    /// time-up phase ends; the clients then disconnect by themselves and are
    /// reaped as usual.
    fn finish_match(&mut self, match_id: u32) {
        let finishers: Vec<SocketAddr> = self
            .peers
            .iter()
            .filter(|(_, peer)| peer.registered && peer.match_id == match_id)
            .map(|(addr, _)| *addr)
            .collect();
        for addr in finishers {
            self.record_result(addr, true);
        }
        let mut sessions = Vec::new();
        let mut scores = Vec::new();
        for peer in self
            .peers
            .values_mut()
            .filter(|peer| peer.registered && peer.match_id == match_id)
        {
            peer.respawn_at = None;
            peer.respawn_announced = None;
            sessions.extend(peer.session_id);
            if peer.joined {
                scores.push(format!("{} {}/{}", peer.name, peer.kills, peer.deaths));
            }
        }
        for session_id in sessions {
            self.store.leave_match(session_id);
        }
        log::info!(
            "match {match_id:#x}: time is up; kills/deaths: {}",
            if scores.is_empty() {
                "none".to_owned()
            } else {
                scores.join(", ")
            }
        );
        if let Some(batteries) = self.batteries.get(&match_id) {
            log::info!("match {match_id:#x}: batteries {}", batteries.describe());
        }
    }

    /// Hand what a joined player did in its match to the store, once: when
    /// the match finishes, or when the player leaves before.
    fn record_result(&mut self, addr: SocketAddr, finished: bool) {
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        if !peer.joined || peer.result_recorded {
            return;
        }
        let Some(session_id) = peer.session_id else {
            return;
        };
        peer.result_recorded = true;
        let result = MatchResult {
            kills: peer.kills,
            steals: peer.steals,
            finished,
            used: self
                .combat
                .as_mut()
                .map(|combat| combat.take_used(addr))
                .unwrap_or_default(),
        };
        log::info!(
            "match {:#x}: player {} {result:?}",
            peer.match_id,
            peer.player_id
        );
        self.store.record_match_result(session_id, result);
    }

    /// Drop the peers still in a finished match after the grace period.
    fn close_finished_match(&mut self, match_id: u32) {
        let lingering: Vec<SocketAddr> = self
            .peers
            .iter()
            .filter(|(_, peer)| peer.registered && peer.match_id == match_id)
            .map(|(addr, _)| *addr)
            .collect();
        for addr in lingering {
            if let Some(combat) = self.combat.as_mut() {
                combat.remove(addr);
            }
            if let Some(peer) = self.peers.remove(&addr) {
                log::warn!(
                    "match {match_id:#x}: dropping player {} ({addr}), still connected after \
                     the match finished",
                    peer.player_id
                );
            }
        }
        self.matches.remove(&match_id);
        self.batteries.remove(&match_id);
        log::info!("match {match_id:#x}: closed");
    }

    /// Update the scoreboard for a kill (only while the match is in process),
    /// and start the victim's respawn countdown.
    fn record_kill(&mut self, match_id: u32, kill: &PlayerKill, now: Instant) {
        let victim = self.peers.iter().find_map(|(addr, peer)| {
            (peer.registered && peer.match_id == match_id && peer.player_id == kill.victim_id)
                .then_some((*addr, peer.respawn_at))
        });
        if let Some((addr, Some(respawn_at))) = victim {
            let time_left = seconds_left(respawn_at, now);
            if let Some(peer) = self.peers.get_mut(&addr) {
                peer.respawn_announced = Some(time_left);
            }
            self.send_to(
                addr,
                ServerGameMessageKind::RespawnTimeChanged { time_left },
            );
        }

        if !self
            .matches
            .get(&match_id)
            .is_some_and(|state| state.phase().is_in_process())
        {
            log::info!(
                "match {match_id:#x}: player {} killed player {} before the match started \
                 (not counted)",
                kill.killer_id,
                kill.victim_id
            );
            return;
        }

        let mut changed = Vec::new();
        for peer in self
            .peers
            .values_mut()
            .filter(|peer| peer.registered && peer.match_id == match_id)
        {
            // A suicide counts only as a death.
            if peer.player_id == kill.victim_id {
                peer.deaths += 1;
            } else if peer.player_id == kill.killer_id {
                peer.kills += 1;
            } else {
                continue;
            }
            changed.push(ServerGameMessageKind::PlayerKdStatsChanged {
                player_id: peer.player_id,
                kills: peer.kills,
                deaths: peer.deaths,
            });
        }
        changed.sort_by_key(|message| match message {
            ServerGameMessageKind::PlayerKdStatsChanged { player_id, .. } => *player_id,
            _ => u8::MAX,
        });
        log::info!(
            "match {match_id:#x}: player {} killed player {}; {changed:?}",
            kill.killer_id,
            kill.victim_id
        );
        for message in changed {
            self.broadcast_to_match(match_id, message);
        }
    }

    /// Give a newly joined client the match's battery state, once.
    fn send_battery_state(&mut self, addr: SocketAddr) {
        if self.peers.get(&addr).is_none_or(|peer| peer.batteries_sent) {
            return;
        }
        self.resend_battery_state(addr);
    }

    /// Give the client at `addr` the match's whole battery state, which it
    /// places without clearing first: once per match, or after it cleared
    /// its world.
    fn resend_battery_state(&mut self, addr: SocketAddr) {
        let Some(match_id) = self.peers.get(&addr).map(|peer| peer.match_id) else {
            return;
        };
        let Some(batteries) = self.batteries.get(&match_id) else {
            return;
        };
        let message = batteries.initialize_message(|player_id| {
            self.peers
                .values()
                .find(|peer| {
                    peer.registered && peer.match_id == match_id && peer.player_id == player_id
                })
                .map_or(map::RESPAWN_POINTS[0].position, Peer::position)
        });
        if let Some(peer) = self.peers.get_mut(&addr) {
            peer.batteries_sent = true;
        }
        self.send_to(addr, message);
    }

    /// Ask the client at `addr` to clear its world
    /// (`world_synchronization_request`) and rebuild it from the server's
    /// once it confirms (`resend_world`). The original server's reason to ask
    /// is unknown; srp asks only from the console (`resync`). The stock client
    /// crashes on the spawns that follow if a player it took out had a weapon
    /// in hand: `player::remove` leaves the weapon's logic state, so
    /// `weapon_core::deserialize` reads a state srp does not send (0x5A4627)
    /// and starts a null one (0x5A46C4). Messages queued
    /// before still reach the client first (they are ordered). From then
    /// until its confirmation nothing about the world goes to it: it would
    /// handle them against players it took out (a move, a hit, a trap of a
    /// removed player), and the world it gets back supersedes them.
    fn request_world_sync(&mut self, addr: SocketAddr, now: Instant) {
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        if !peer.joined || peer.match_finished || peer.world_sync_requested.is_some() {
            return;
        }
        log::info!(
            "match {:#x}: asking player {} ({addr}) to resynchronize its world",
            peer.match_id,
            peer.player_id
        );
        deliver(
            &mut self.recorded,
            addr,
            peer,
            ServerGameMessageKind::WorldSynchronizationRequest,
        );
        peer.world_sync_requested = Some(now);
    }

    /// The client at `addr` cleared its world and confirmed
    /// (`world_synchronization_confirmation`): put the world back as it is
    /// now. First every living player its roster has, itself included, then
    /// the batteries (which name their carriers), the traps of the players
    /// put back, and their damage models (each spawn reset one).
    fn resend_world(&mut self, addr: SocketAddr, now: Instant) {
        let Some(peer) = self.peers.get_mut(&addr) else {
            return;
        };
        let Some(asked) = peer.world_sync_requested.take() else {
            log::warn!("ignoring a world synchronization confirmation nobody asked {addr} for");
            return;
        };
        let match_id = peer.match_id;
        log::info!(
            "match {match_id:#x}: player {} ({addr}) cleared its world in {:?}; sending it back",
            peer.player_id,
            now.saturating_duration_since(asked)
        );
        peer.owed_damage_models.clear();

        let mut players: Vec<(u8, SocketAddr)> = self
            .peers
            .iter()
            .filter(|(_, peer)| peer.joined && peer.match_id == match_id)
            .map(|(addr, peer)| (peer.player_id, *addr))
            .collect();
        players.sort();
        let mut shown = Vec::new();
        for (player_id, subject) in players {
            let Some(spawn) = self.live_spawn_message(addr, subject) else {
                continue;
            };
            if subject == addr {
                // Its own player: the client restarts its weapons from this.
                if let Some(combat) = self.combat.as_mut() {
                    combat.respawn_weapons(addr);
                }
            }
            self.send_to(addr, spawn);
            shown.push((player_id, subject));
        }
        self.resend_battery_state(addr);
        let traps = self
            .combat
            .as_ref()
            .map(|combat| combat.traps_out(match_id))
            .unwrap_or_default();
        for trap in traps
            .into_iter()
            .filter(|t| shown.iter().any(|(player_id, _)| *player_id == t.player_id))
        {
            self.send_to(
                addr,
                ServerGameMessageKind::GameWorldObjectState {
                    player_id: trap.player_id,
                    slot: trap.slot,
                    index: trap.index,
                    state: trap.state,
                    position: trap.position.into(),
                    angles: trap.angles.into(),
                },
            );
        }
        for (_, subject) in shown {
            self.send_damage_model(addr, subject);
        }
    }

    /// Act on the use key of a client update: while it is held, use each new
    /// thing in reach once. Only a live player in a running match can use.
    fn handle_use_at(&mut self, from: SocketAddr, action_mask: u32, now: Instant) {
        let rules = self.rules;
        let Some(peer) = self.peers.get_mut(&from) else {
            return;
        };
        if action_mask != peer.action_mask {
            // Which bit is which key (use is USE_ACTION) shows up here.
            log::debug!(
                "player {} action mask {:#010x} -> {action_mask:#010x}",
                peer.player_id,
                peer.action_mask
            );
            peer.action_mask = action_mask;
        }
        let Some(just_pressed) = peer.track_use_key(action_mask & USE_ACTION != 0, now) else {
            return;
        };
        let (match_id, player_id, team) = (peer.match_id, peer.player_id, peer.team);
        let eye = batteries::eye_at(peer.position());
        let can_use = peer.joined && peer.alive;

        let in_process = self
            .matches
            .get(&match_id)
            .is_some_and(|state| state.phase().is_in_process());
        let Some(batteries) = self.batteries.get_mut(&match_id) else {
            return;
        };
        let target = batteries.target(player_id, team, eye, rules.use_reach);
        if just_pressed {
            log::info!(
                "match {match_id:#x}: player {player_id} pressed use at ({:.1}, {:.1}, {:.1}): \
                 {}; reach {} m -> {}",
                eye.x,
                eye.y - batteries::EYE_HEIGHT,
                eye.z,
                batteries.describe_reach(eye),
                rules.use_reach,
                match (target, can_use, in_process) {
                    (None, ..) => "nothing to use".to_owned(),
                    (Some(_), false, _) => "ignored (not alive in the match)".to_owned(),
                    (Some(_), true, false) => "ignored (the match is not running)".to_owned(),
                    (Some(target), true, true) => format!("{target:?}"),
                }
            );
        }
        if !can_use || !in_process {
            return;
        }

        let peer = self.peers.get_mut(&from).unwrap();
        if peer.use_target == target {
            return;
        }
        peer.use_target = target;
        let Some(target) = target else {
            return;
        };
        let Some((item_id, action)) = batteries.use_target(player_id, team, target) else {
            return;
        };
        if matches!(action, VictoryItemAction::TakeFromBase { .. }) {
            peer.steals += 1;
        }
        log::info!(
            "match {match_id:#x}: player {player_id} ({team:?}) {action:?} battery {item_id}; {}",
            batteries.describe()
        );
        let winner = batteries.winner();
        self.broadcast_to_match(
            match_id,
            ServerGameMessageKind::VictoryItemTakeOrPut {
                player_id,
                item_id,
                action,
            },
        );
        if let Some(winner) = winner {
            self.win_match(match_id, winner, now);
        }
    }

    /// A dying or leaving carrier drops its battery where it was.
    fn drop_battery(&mut self, match_id: u32, player_id: u8) {
        let position = self
            .peers
            .values()
            .find(|peer| {
                peer.registered && peer.match_id == match_id && peer.player_id == player_id
            })
            .map(Peer::position);
        if let Some(position) = position {
            self.drop_battery_at(match_id, player_id, position);
        }
    }

    fn drop_battery_at(&mut self, match_id: u32, player_id: u8, position: float3) {
        let Some(batteries) = self.batteries.get_mut(&match_id) else {
            return;
        };
        let Some(item_id) = batteries.drop_carried(player_id, position) else {
            return;
        };
        log::info!(
            "match {match_id:#x}: player {player_id} dropped battery {item_id}; {}",
            batteries.describe()
        );
        self.broadcast_to_match(
            match_id,
            ServerGameMessageKind::VictoryItemTakeOrPut {
                player_id,
                item_id,
                action: VictoryItemAction::Drop { position },
            },
        );
    }

    /// A team stored every battery: end the match the way time running out
    /// does (the client has no win screen).
    fn win_match(&mut self, match_id: u32, winner: game_team_id, now: Instant) {
        let time_up = self.rules.time_up();
        let Some(state) = self.matches.get_mut(&match_id) else {
            return;
        };
        if !state.phase().is_in_process() {
            return;
        }
        log::info!("match {match_id:#x}: {winner:?} stored every battery and wins");
        let phase = Phase::TimeUp {
            ends_at: now + time_up,
        };
        state.set_phase(phase);
        self.enter_phases(match_id, &[phase], now);
    }

    /// The scoreboard of every joined player, by player id.
    fn kd_messages(&self, match_id: u32) -> Vec<ServerGameMessageKind> {
        let mut players: Vec<&Peer> = self
            .peers
            .values()
            .filter(|peer| peer.registered && peer.joined && peer.match_id == match_id)
            .collect();
        players.sort_by_key(|peer| peer.player_id);
        players
            .into_iter()
            .map(|peer| ServerGameMessageKind::PlayerKdStatsChanged {
                player_id: peer.player_id,
                kills: peer.kills,
                deaths: peer.deaths,
            })
            .collect()
    }

    fn run_console_commands(&mut self, now: Instant) {
        for command in console::take_all() {
            self.run_console_command(command, now);
        }
    }

    fn run_console_command(&mut self, command: Command, now: Instant) {
        log::info!("console: {command:?}");
        match command {
            Command::Status => self.log_status(now),
            Command::Start { match_id } => {
                let match_time = self.rules.match_time();
                for match_id in self.console_targets(match_id) {
                    let state = self.matches.get_mut(&match_id).unwrap();
                    if !matches!(
                        state.phase(),
                        Phase::WaitingForPlayers { .. } | Phase::FinalCountdown { .. }
                    ) {
                        log::info!(
                            "console: match {match_id:#x} is already {}",
                            state.phase().name()
                        );
                        continue;
                    }
                    let phase = Phase::InProcess {
                        ends_at: now + match_time,
                    };
                    state.set_phase(phase);
                    self.enter_phases(match_id, &[phase], now);
                }
            }
            Command::End { match_id } => {
                let time_up = self.rules.time_up();
                for match_id in self.console_targets(match_id) {
                    let state = self.matches.get_mut(&match_id).unwrap();
                    // The same path as a natural end: time-up text first.
                    let phase = match state.phase() {
                        Phase::Finished { .. } => {
                            log::info!("console: match {match_id:#x} is already finished");
                            continue;
                        }
                        Phase::TimeUp { .. } => Phase::Finished { at: now },
                        _ => Phase::TimeUp {
                            ends_at: now + time_up,
                        },
                    };
                    state.set_phase(phase);
                    self.enter_phases(match_id, &[phase], now);
                }
            }
            Command::Resync { match_id } => {
                for match_id in self.console_targets(match_id) {
                    let joined: Vec<SocketAddr> = self
                        .peers
                        .iter()
                        .filter(|(_, peer)| peer.joined && peer.match_id == match_id)
                        .map(|(addr, _)| *addr)
                        .collect();
                    for addr in joined {
                        self.request_world_sync(addr, now);
                    }
                }
            }
            Command::Batteries { match_id } => {
                for match_id in self.console_targets(match_id) {
                    if let Some(batteries) = self.batteries.get(&match_id) {
                        log::info!(
                            "console: match {match_id:#x} batteries {}",
                            batteries.describe()
                        );
                    }
                }
            }
            Command::Time { seconds, match_id } => {
                for match_id in self.console_targets(match_id) {
                    let state = self.matches.get_mut(&match_id).unwrap();
                    if state.set_time_left(now, Duration::from_secs(seconds.into())) {
                        log::info!(
                            "console: match {match_id:#x} {} now has {seconds} s left",
                            state.phase().name()
                        );
                    } else {
                        log::info!("console: match {match_id:#x} is already finished");
                    }
                }
            }
        }
    }

    /// The matches a console command applies to: the given one, or all.
    fn console_targets(&self, match_id: Option<u32>) -> Vec<u32> {
        let mut match_ids: Vec<u32> = match match_id {
            Some(match_id) => self
                .matches
                .contains_key(&match_id)
                .then_some(match_id)
                .into_iter()
                .collect(),
            None => self.matches.keys().copied().collect(),
        };
        match_ids.sort();
        if match_ids.is_empty() {
            log::info!("console: no matching active match");
        }
        match_ids
    }

    fn log_status(&self, now: Instant) {
        log::info!("status: rules {:?}", self.rules);
        match &self.combat {
            Some(combat) => log::info!(
                "status: combat on, {} bullet(s) in flight",
                combat.bullets_in_flight()
            ),
            None => log::info!("status: combat off (no game data)"),
        }
        let mut match_ids: Vec<u32> = self
            .peers
            .values()
            .filter(|peer| peer.registered)
            .map(|peer| peer.match_id)
            .chain(self.matches.keys().copied())
            .collect();
        match_ids.sort();
        match_ids.dedup();
        if match_ids.is_empty() {
            log::info!("status: no players connected");
        }

        for match_id in match_ids {
            let phase = self.matches.get(&match_id).map_or_else(
                || "no lifecycle yet (nobody has joined)".to_owned(),
                |state| {
                    let time_left = state
                        .seconds_left(now)
                        .map(|seconds| format!(", {}:{:02} left", seconds / 60, seconds % 60))
                        .unwrap_or_default();
                    format!("{}{time_left}", state.phase().name())
                },
            );
            log::info!("status: match {match_id:#x}: {phase}");
            if let Some(batteries) = self.batteries.get(&match_id) {
                log::info!("status:   batteries {}", batteries.describe());
            }

            let mut players: Vec<(&SocketAddr, &Peer)> = self
                .peers
                .iter()
                .filter(|(_, peer)| peer.registered && peer.match_id == match_id)
                .collect();
            players.sort_by_key(|(_, peer)| peer.player_id);
            for (addr, peer) in players {
                let health = self
                    .combat
                    .as_ref()
                    .and_then(|combat| combat.vital_fraction(*addr))
                    .map(|fraction| format!(" ({:.0}% health)", fraction * 100.0))
                    .unwrap_or_default();
                let state = if !peer.joined {
                    "loading".to_owned()
                } else if peer.alive {
                    format!("alive{health}")
                } else {
                    "dead".to_owned()
                };
                log::info!(
                    "status:   player {} {} ({:?}, {addr}): {state}, {} kills / {} deaths, \
                     last heard {} ms ago",
                    peer.player_id,
                    peer.name,
                    peer.team,
                    peer.kills,
                    peer.deaths,
                    now.saturating_duration_since(peer.last_seen).as_millis()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map;
    use crate::message::server_message::raw::game_status;
    use std::time::Duration;

    fn peer_addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    /// The startup roster of a client in a full match: its numbers are the
    /// server's.
    fn full_roster() -> Vec<u8> {
        (0..MAX_PLAYERS_PER_MATCH).collect()
    }

    fn joined_peer(match_id: u32, player_id: u8) -> Peer {
        Peer {
            registered: true,
            match_id,
            player_id,
            joined: true,
            roster: full_roster(),
            ..Peer::new()
        }
    }

    fn player_hit(hit_initiator: u8, being_hit: u8) -> PlayerHit {
        PlayerHit {
            hit_initiator,
            being_hit,
            body_part: "head".to_owned(),
            damage_type: "injury".to_owned(),
            amount: 50.0,
            armor_piercing: 0.25,
        }
    }

    /// A minimal, valid client "connection" datagram: local_seq 0, remote_seq
    /// 0xFFFF, ack word 0 (→ remote_ack_bits 0x8000, single packet), then a
    /// `connection_request` (0x40) with order_id 0 and the session id.
    fn connection_packet(session_id: u32) -> Vec<u8> {
        let mut packet = vec![0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0x40, 0x00, 0x00];
        packet.extend(session_id.to_le_bytes());
        packet
    }

    #[test]
    fn responds_to_a_connection_request() {
        let store = Arc::new(SessionStore::new());
        let server = MatchServer::bind("127.0.0.1:0", store).unwrap();
        let server_addr = server.local_addr().unwrap();
        std::thread::spawn(move || server.run());

        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .send_to(&connection_packet(0xDD00), server_addr)
            .unwrap();

        let mut buf = [0u8; 2048];
        let (len, _) = client
            .recv_from(&mut buf)
            .expect("relay should answer a connection request");

        // Server packet: 6-byte header, then the first message's type byte.
        // 0x80 = match_server_connection_successful.
        assert!(len >= 7, "response too short: {len} bytes");
        assert_eq!(
            buf[6], 0x80,
            "expected ConnectionSuccessful, got {:#x}",
            buf[6]
        );
    }

    #[test]
    fn keeps_receiving_after_sending_to_a_closed_port() {
        let mut server = MatchServer::bind("127.0.0.1:0", Arc::new(SessionStore::new())).unwrap();
        // A client that has gone away: nothing listens on this port any more.
        let closed = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        server.socket.send_to(&[0], closed).unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .send_to(&connection_packet(0xDD00), server.local_addr().unwrap())
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));

        server.receive_pending(&mut [0u8; 2048]);

        assert!(
            server.peers.values().any(|peer| peer.registered),
            "a live client's datagram must not be held back by the closed port"
        );
    }

    #[test]
    fn assigns_player_ids_per_match() {
        let store = Arc::new(SessionStore::new());
        for session_id in 0..=20 {
            store.join_match(session_id);
        }
        let mut server = MatchServer::bind("127.0.0.1:0", store).unwrap();

        for session_id in 0..=20 {
            let addr = peer_addr(10_000 + session_id as u16);
            server.peers.insert(addr, Peer::new());
            server.register_peer(addr, session_id);
        }

        let first = &server.peers[&peer_addr(10_000)];
        let twenty_first = &server.peers[&peer_addr(10_020)];
        assert_ne!(first.match_id, twenty_first.match_id);
        assert_eq!(first.player_id, 0);
        assert_eq!(twenty_first.player_id, 0);
    }

    #[test]
    fn reuses_a_vacated_player_id() {
        let store = Arc::new(SessionStore::new());
        for session_id in 1..=3 {
            store.join_match(session_id);
        }
        let mut server = MatchServer::bind("127.0.0.1:0", store).unwrap();

        for session_id in 1..=2 {
            let addr = peer_addr(11_000 + session_id as u16);
            server.peers.insert(addr, Peer::new());
            server.register_peer(addr, session_id);
        }
        server.peers.remove(&peer_addr(11_001));

        let replacement = peer_addr(11_003);
        server.peers.insert(replacement, Peer::new());
        server.register_peer(replacement, 3);
        assert_eq!(server.peers[&replacement].player_id, 0);
    }

    #[test]
    fn reaping_a_departure_releases_its_match_and_player_slots() {
        let store = Arc::new(SessionStore::new());
        let departed_assignment = store.join_match(1);
        store.join_match(2);
        let mut server = MatchServer::bind("127.0.0.1:0", Arc::clone(&store)).unwrap();

        let departed = peer_addr(11_100);
        let remaining = peer_addr(11_101);
        let mut departed_peer = joined_peer(departed_assignment.match_id, 0);
        departed_peer.session_id = Some(1);
        departed_peer.last_seen = Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);
        let mut remaining_peer = joined_peer(departed_assignment.match_id, 1);
        remaining_peer.session_id = Some(2);
        // Its client never had player 0.
        remaining_peer.roster = vec![1];
        server.peers.insert(departed, departed_peer);
        server.peers.insert(remaining, remaining_peer);

        server.reap_peers();

        assert!(!server.peers.contains_key(&departed));
        assert_eq!(store.match_assignment(1), None);
        let replacement_assignment = store.join_match(3);
        assert_eq!(
            replacement_assignment.match_id,
            departed_assignment.match_id
        );

        let replacement = peer_addr(11_102);
        server.peers.insert(replacement, Peer::new());
        server.register_peer(replacement, 3);
        assert_eq!(server.peers[&replacement].player_id, 0);
    }

    #[test]
    fn a_player_id_stays_taken_while_a_client_still_has_its_player() {
        let store = Arc::new(SessionStore::new());
        for session_id in 1..=3 {
            store.join_match(session_id);
        }
        let mut server = MatchServer::bind("127.0.0.1:0", store).unwrap();
        let (first, second) = (peer_addr(11_010), peer_addr(11_011));
        for (session_id, addr) in [(1, first), (2, second)] {
            server.peers.insert(addr, Peer::new());
            server.register_peer(addr, session_id);
        }
        // Its list of players, without waiting for session 3.
        server.send_startup_info(second);
        server.peers.remove(&first);

        let newcomer = peer_addr(11_012);
        server.peers.insert(newcomer, Peer::new());
        server.register_peer(newcomer, 3);
        assert_eq!(server.peers[&newcomer].player_id, 2);

        // Once no client has the old player, its id is free again.
        server.peers.remove(&second);
        server.peers.remove(&newcomer);
        server.peers.insert(newcomer, Peer::new());
        server.register_peer(newcomer, 3);
        assert_eq!(server.peers[&newcomer].player_id, 0);
    }

    #[test]
    fn a_client_finding_no_free_id_plays_in_another_match() {
        let store = Arc::new(SessionStore::new());
        let full = store.join_match(1).match_id;
        let mut server = MatchServer::bind("127.0.0.1:0", store.clone()).unwrap();
        // Every id is held: a registered player whose roster lists all.
        server
            .peers
            .insert(peer_addr(11_020), loading_peer(full, 0));

        let newcomer = peer_addr(11_021);
        store.join_match(2);
        server.peers.insert(newcomer, Peer::new());
        server.register_peer(newcomer, 2);

        let placed = store.match_assignment(2).unwrap();
        assert_ne!(placed.match_id, full);
        assert_eq!(server.peers[&newcomer].match_id, placed.match_id);
        assert_eq!(server.peers[&newcomer].player_id, 0);
    }

    #[test]
    fn the_matchmaker_hears_which_ids_clients_still_have() {
        let store = Arc::new(SessionStore::new());
        let first = store.join_match(1).match_id;
        let mut server = MatchServer::bind("127.0.0.1:0", store.clone()).unwrap();
        // The others left; this client's roster still lists all twenty.
        server
            .peers
            .insert(peer_addr(11_022), loading_peer(first, 0));
        server.report_held_player_ids();
        assert_ne!(store.join_match(2).match_id, first);
    }

    #[test]
    fn a_player_who_leaves_before_joining_is_greyed_out_where_it_was_listed() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let ghost = peer_addr(11_200);
        let listed = peer_addr(11_201);
        let unlisted = peer_addr(11_202);
        let mut ghost_peer = loading_peer(DEFAULT_MATCH_ID, 1);
        ghost_peer.last_seen = Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);
        server.peers.insert(ghost, ghost_peer);
        // Its startup roster listed players 0 and 1; the other's did not.
        server.peers.insert(
            listed,
            Peer {
                roster: vec![0, 1],
                ..joined_peer(DEFAULT_MATCH_ID, 0)
            },
        );
        server.peers.insert(
            unlisted,
            Peer {
                roster: vec![0, 2],
                ..joined_peer(DEFAULT_MATCH_ID, 2)
            },
        );

        server.reap_peers();

        let greyed = |addr| {
            sent_to(&server, addr).iter().any(|message| {
                matches!(
                    message,
                    ServerGameMessageKind::PlayerVisibilityChanged {
                        player_id: 1,
                        is_visible: false,
                    }
                )
            })
        };
        assert!(greyed(listed));
        assert!(!greyed(unlisted));
        // Its client keeps numbering the others by their place in the roster.
        assert_eq!(server.peers[&listed].roster, [0, 1]);
    }

    #[test]
    fn removes_an_over_capacity_unassigned_peer() {
        let store = Arc::new(SessionStore::new());
        let mut server = MatchServer::bind("127.0.0.1:0", store).unwrap();

        for session_id in 0..=20 {
            let addr = peer_addr(12_000 + session_id as u16);
            server.peers.insert(addr, Peer::new());
            server.register_peer(addr, session_id);
        }

        assert!(!server.peers.contains_key(&peer_addr(12_020)));
        assert_eq!(MAX_PLAYERS_PER_MATCH, 20);
    }

    #[test]
    fn registration_uses_the_profile_selected_in_the_lobby() {
        let store = Arc::new(SessionStore::new());
        let mut selected = raw_player_profile::new_dummy(1, 400_000, "selected");
        selected.slots[profile_slot_enum::weapon1_slot].dict_id = 55;
        store.ready_for_match(0xDD00, selected, &[selected]);

        let mut server = MatchServer::bind("127.0.0.1:0", Arc::clone(&store)).unwrap();
        let addr = peer_addr(12_100);
        server.peers.insert(addr, Peer::new());
        server.register_peer(addr, 0xDD00);

        assert_eq!(server.peers[&addr].profile.profile_id, 400_000);
        assert_eq!(
            server.peers[&addr].profile.slots[profile_slot_enum::weapon1_slot].dict_id,
            55
        );
    }

    /// The kill a combat event turns into, for the tests that need a dead
    /// player.
    fn kill(server: &mut MatchServer, victim: SocketAddr, killer_id: u8, at: Instant) {
        let victim_id = server.peers[&victim].player_id;
        server.kill_player_at(
            victim,
            PlayerKill {
                victim_id,
                killer_id,
                is_headshot: false,
                item_dict_id: 13,
            },
            at,
        );
    }

    #[test]
    fn suicide_kills_the_player_by_its_own_hand_once() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let addr = peer_addr(13_110);
        server.peers.insert(addr, loading_peer(DEFAULT_MATCH_ID, 3));
        server.handle_game_message(addr, ClientGameMessageKind::JoinMatch);

        server.handle_game_message(addr, ClientGameMessageKind::CommitSuicide);
        server.handle_game_message(addr, ClientGameMessageKind::CommitSuicide);

        assert!(!server.peers[&addr].alive);
        let kills: Vec<PlayerKill> = sent_to(&server, addr)
            .into_iter()
            .filter_map(|message| match message {
                ServerGameMessageKind::KillPlayer(kill) => Some(kill),
                _ => None,
            })
            .collect();
        assert_eq!(kills.len(), 1);
        assert_eq!((kills[0].victim_id, kills[0].killer_id), (3, 3));
        assert_ne!(kills[0].item_dict_id, 0, "the kill feed needs an item");
    }

    #[test]
    fn a_repeated_join_does_not_revive_a_dead_player() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let addr = peer_addr(13_100);
        server.peers.insert(addr, loading_peer(DEFAULT_MATCH_ID, 0));
        server.handle_game_message(addr, ClientGameMessageKind::JoinMatch);
        kill(&mut server, addr, 1, Instant::now());

        server.handle_game_message(addr, ClientGameMessageKind::JoinMatch);
        assert!(!server.peers[&addr].alive);
        let spawns = sent_to(&server, addr)
            .into_iter()
            .filter(|message| matches!(message, ServerGameMessageKind::SpawnPlayer { .. }))
            .count();
        assert_eq!(spawns, 1);
    }

    #[test]
    fn relayed_updates_carry_the_server_clock() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let moving = peer_addr(13_200);
        let watching = peer_addr(13_201);
        server
            .peers
            .insert(moving, joined_peer(DEFAULT_MATCH_ID, 0));
        server
            .peers
            .insert(watching, joined_peer(DEFAULT_MATCH_ID, 1));
        let feet = map::RESPAWN_POINTS[0].position;
        let start = server.started;
        // A client whose clock is behind the last one that had this id.
        for (client_ms, after_ms) in [(90_000, 10), (5, 20), (21, 36)] {
            let ClientGameMessageKind::ClientPlayerUpdate {
                player_input,
                player_state,
                ..
            } = update_at(feet, false)
            else {
                unreachable!();
            };
            server.handle_game_message_at(
                moving,
                ClientGameMessageKind::ClientPlayerUpdate {
                    player_input,
                    player_state,
                    time_in_ms: client_ms,
                },
                start + Duration::from_millis(after_ms),
            );
        }
        let stamps: Vec<u32> = sent_to(&server, watching)
            .into_iter()
            .filter_map(|message| match message {
                ServerGameMessageKind::ServerPlayerInput { time_in_ms, .. } => Some(time_in_ms),
                _ => None,
            })
            .collect();
        assert_eq!(stamps, [10, 20, 36]);
    }

    #[test]
    fn ignores_client_hit_reports() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let attacker = peer_addr(13_000);
        let victim = peer_addr(13_001);
        server
            .peers
            .insert(attacker, joined_peer(DEFAULT_MATCH_ID, 0));
        server
            .peers
            .insert(victim, joined_peer(DEFAULT_MATCH_ID, 1));

        for _ in 0..8 {
            server.handle_game_message(
                attacker,
                ClientGameMessageKind::ClientPlayerHit(player_hit(0, 1)),
            );
        }
        assert!(server.peers[&victim].alive);
        assert_eq!(sent_to(&server, victim), []);
    }

    #[test]
    fn a_kill_is_announced_once_and_starts_the_respawn() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let attacker = peer_addr(15_000);
        let victim = peer_addr(15_001);
        server
            .peers
            .insert(attacker, joined_peer(DEFAULT_MATCH_ID, 0));
        server
            .peers
            .insert(victim, joined_peer(DEFAULT_MATCH_ID, 1));

        let now = Instant::now();
        kill(&mut server, victim, 0, now);
        kill(&mut server, victim, 0, now);

        let kills: Vec<ServerGameMessageKind> = sent_to(&server, attacker)
            .into_iter()
            .filter(|message| matches!(message, ServerGameMessageKind::KillPlayer(..)))
            .collect();
        assert_eq!(
            kills,
            [ServerGameMessageKind::KillPlayer(PlayerKill {
                victim_id: 1,
                killer_id: 0,
                is_headshot: false,
                item_dict_id: 13,
            })],
            "a dead player must not die again"
        );
        assert!(!server.peers[&victim].alive);
        assert_eq!(
            server.peers[&victim].respawn_at,
            Some(now + server.rules.respawn_time())
        );
    }

    #[test]
    fn respawns_a_dead_player_after_ten_seconds_at_a_random_point() {
        let mut server = MatchServer::bind("127.0.0.1:0", Arc::new(SessionStore::new())).unwrap();
        server.spawn_rng = SpawnRng::new(123);
        let attacker = peer_addr(15_100);
        let victim = peer_addr(15_101);
        server
            .peers
            .insert(attacker, joined_peer(DEFAULT_MATCH_ID, 0));
        server
            .peers
            .insert(victim, joined_peer(DEFAULT_MATCH_ID, 1));
        let killed_at = Instant::now();
        let seeds = server.peers[&victim].weapon_seeds;

        kill(&mut server, victim, 0, killed_at);
        let respawn_time = server.rules.respawn_time();
        server.respawn_players_at(killed_at + respawn_time - Duration::from_millis(1));
        assert!(!server.peers[&victim].alive);

        server.respawn_players_at(killed_at + respawn_time);
        let victim = &server.peers[&victim];
        assert!(victim.alive);
        assert_eq!(victim.respawn_at, None);
        assert_ne!(
            victim.weapon_seeds, seeds,
            "every spawn reseeds the weapons"
        );
        let point = &map::RESPAWN_POINTS[victim.spawn_index];
        assert_eq!(point.team, game_team_id::team_1);
        assert_eq!(point.priority, 2);
    }

    #[test]
    fn assigns_unused_spawn_points_within_a_match() {
        let mut server = MatchServer::bind("127.0.0.1:0", Arc::new(SessionStore::new())).unwrap();
        server.spawn_rng = SpawnRng::new(123);

        let first = peer_addr(16_000);
        let second = peer_addr(16_001);
        let first_spawn = server.select_spawn_index(DEFAULT_MATCH_ID, game_team_id::team_1, first);
        let mut first_peer = joined_peer(DEFAULT_MATCH_ID, 0);
        first_peer.spawn_index = first_spawn;
        server.peers.insert(first, first_peer);

        let second_spawn =
            server.select_spawn_index(DEFAULT_MATCH_ID, game_team_id::team_1, second);
        let mut second_peer = joined_peer(DEFAULT_MATCH_ID, 1);
        second_peer.spawn_index = second_spawn;
        server.peers.insert(second, second_peer);

        for spawn in [first_spawn, second_spawn] {
            assert_eq!(map::RESPAWN_POINTS[spawn].team, game_team_id::team_1);
            assert_eq!(map::RESPAWN_POINTS[spawn].priority, 2);
        }
        assert_ne!(first_spawn, second_spawn);
    }

    #[test]
    fn each_team_spawns_at_its_own_base_then_behind_it_when_full() {
        let mut server = MatchServer::bind("127.0.0.1:0", Arc::new(SessionStore::new())).unwrap();
        server.spawn_rng = SpawnRng::new(99);
        let (base, safe) = points_by_priority(game_team_id::team_2);
        assert!(!base.is_empty() && !safe.is_empty());

        // A teammate stands on every base point (at the spawn, not moved yet).
        for (player_id, point) in base.iter().enumerate() {
            let mut peer = joined_peer(DEFAULT_MATCH_ID, player_id as u8);
            peer.team = game_team_id::team_2;
            peer.spawn_index = *point;
            server
                .peers
                .insert(peer_addr(16_100 + player_id as u16), peer);
        }
        let spawning = peer_addr(16_199);
        let fallback = server.select_spawn_index(DEFAULT_MATCH_ID, game_team_id::team_2, spawning);
        assert!(
            safe.contains(&fallback),
            "every base point is taken: {fallback}"
        );

        let other_team =
            server.select_spawn_index(DEFAULT_MATCH_ID, game_team_id::team_1, spawning);
        assert_eq!(map::RESPAWN_POINTS[other_team].team, game_team_id::team_1);
        assert_eq!(map::RESPAWN_POINTS[other_team].priority, 2);
    }

    /// A team's base points (priority 2) and its "safe" ones (priority 1).
    fn points_by_priority(team: game_team_id) -> (Vec<usize>, Vec<usize>) {
        game::team_respawn_points(team)
            .into_iter()
            .partition(|index| map::RESPAWN_POINTS[*index].priority == 2)
    }

    #[test]
    fn a_teammate_who_walked_off_no_longer_holds_the_point() {
        let mut server = MatchServer::bind("127.0.0.1:0", Arc::new(SessionStore::new())).unwrap();
        let (base, _) = points_by_priority(game_team_id::team_2);
        for (player_id, point) in base.iter().enumerate() {
            let mut peer = joined_peer(DEFAULT_MATCH_ID, player_id as u8);
            peer.team = game_team_id::team_2;
            peer.spawn_index = *point;
            // Out in the field, far from the base.
            peer.position = Some(float3 {
                x: 0.0,
                y: 0.5,
                z: -30.0,
            });
            server
                .peers
                .insert(peer_addr(16_300 + player_id as u16), peer);
        }

        for seed in 0..20 {
            server.spawn_rng = SpawnRng::new(seed);
            let spawn = server.select_spawn_index(
                DEFAULT_MATCH_ID,
                game_team_id::team_2,
                peer_addr(16_399),
            );
            assert!(base.contains(&spawn), "seed {seed}: {spawn}");
        }
    }

    #[test]
    fn an_enemy_in_the_base_sends_respawns_behind_it() {
        let mut server = MatchServer::bind("127.0.0.1:0", Arc::new(SessionStore::new())).unwrap();
        let (base, safe) = points_by_priority(game_team_id::team_1);
        let enemy = peer_addr(16_400);
        let mut enemy_peer = joined_peer(DEFAULT_MATCH_ID, 0);
        enemy_peer.team = game_team_id::team_2;
        // On team 1's base floor, between its spawn points.
        enemy_peer.position = Some(float3 {
            x: -34.0,
            y: 0.51,
            z: -76.0,
        });
        server.peers.insert(enemy, enemy_peer);
        let spawning = peer_addr(16_401);

        for seed in 0..20 {
            server.spawn_rng = SpawnRng::new(seed);
            let spawn = server.select_spawn_index(DEFAULT_MATCH_ID, game_team_id::team_1, spawning);
            assert!(safe.contains(&spawn), "seed {seed}: {spawn}");
        }

        // A dead enemy, or one in another match, closes nothing.
        server.peers.get_mut(&enemy).unwrap().alive = false;
        let spawn = server.select_spawn_index(DEFAULT_MATCH_ID, game_team_id::team_1, spawning);
        assert!(base.contains(&spawn), "{spawn}");
        let enemy_peer = server.peers.get_mut(&enemy).unwrap();
        enemy_peer.alive = true;
        enemy_peer.match_id = DEFAULT_MATCH_ID + 1;
        let spawn = server.select_spawn_index(DEFAULT_MATCH_ID, game_team_id::team_1, spawning);
        assert!(base.contains(&spawn), "{spawn}");
    }

    #[test]
    fn a_respawning_player_does_not_block_its_own_point() {
        let mut server = MatchServer::bind("127.0.0.1:0", Arc::new(SessionStore::new())).unwrap();
        let (base, _) = points_by_priority(game_team_id::team_2);
        // Re-joining while standing on a base point closes nothing for itself.
        let spawning = peer_addr(16_500);
        let mut peer = joined_peer(DEFAULT_MATCH_ID, 0);
        peer.team = game_team_id::team_2;
        peer.spawn_index = base[0];
        server.peers.insert(spawning, peer);

        for seed in 0..20 {
            server.spawn_rng = SpawnRng::new(seed);
            let spawn = server.select_spawn_index(DEFAULT_MATCH_ID, game_team_id::team_2, spawning);
            assert!(base.contains(&spawn), "seed {seed}: {spawn}");
        }
    }

    #[test]
    fn spawns_face_the_way_the_map_points_them() {
        let profile = raw_player_profile::new_dummy(0, 0, "demo_player");
        for (index, point) in map::RESPAWN_POINTS.iter().enumerate() {
            let player = game::spawn_content(index, &profile, [(0, 1); 2], |_| None);
            assert_eq!(player.position, point.position);
            assert_eq!(player.orientation, point.yaw);
        }
    }

    #[test]
    fn answers_a_time_sync_request_with_who_is_connected() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let asking = peer_addr(16_600);
        let other = peer_addr(16_601);
        let elsewhere = peer_addr(16_602);
        server
            .peers
            .insert(asking, joined_peer(DEFAULT_MATCH_ID, 0));
        server.peers.insert(other, joined_peer(DEFAULT_MATCH_ID, 3));
        server
            .peers
            .insert(elsewhere, joined_peer(DEFAULT_MATCH_ID + 1, 1));

        let now = Instant::now();
        server.handle_game_message_at(
            asking,
            ClientGameMessageKind::TimeSynchronizationRequest {
                client_time_ms: 185_628,
            },
            now,
        );
        assert_eq!(
            sent_to(&server, asking),
            [ServerGameMessageKind::SyncResponse {
                is_connected_bitmask: 0b1001
            }]
        );
        assert!(sent_to(&server, other).is_empty());
        assert!(sent_to(&server, elsewhere).is_empty());

        // The client confirms the answer; that closes its timing.
        assert_eq!(server.peers[&asking].sync_answered, Some(now));
        server.handle_game_message_at(
            asking,
            ClientGameMessageKind::TimeSynchronizationConfirmation,
            now + Duration::from_millis(40),
        );
        assert_eq!(server.peers[&asking].sync_answered, None);
    }

    #[test]
    fn team_bases_info_gets_the_others_spawned_and_no_unasked_sync() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let loading = peer_addr(16_610);
        let playing = peer_addr(16_611);
        server
            .peers
            .insert(loading, loading_peer(DEFAULT_MATCH_ID, 0));
        server
            .peers
            .insert(playing, joined_peer(DEFAULT_MATCH_ID, 1));

        server.handle_game_message_at(
            loading,
            ClientGameMessageKind::TeamBasesInitializeInfo,
            Instant::now(),
        );
        let sent = sent_to(&server, loading);
        assert!(matches!(
            sent[..],
            [ServerGameMessageKind::SpawnPlayer { player_id: 1, .. }]
        ));
    }

    fn facing_update(feet: float3, yaw: f32, look_pitch: f32) -> ClientGameMessageKind {
        use survarium::player_input::{player_input, player_state};
        use vostok::math::float2;
        ClientGameMessageKind::ClientPlayerUpdate {
            player_input: player_input {
                angular_velocity: float2 { x: 0.0, y: 0.0 },
                angualr_acceleration: float2 { x: 0.0, y: 0.0 },
                action_mask: 0,
            },
            player_state: player_state {
                translation_w: feet,
                euler_angle_y: yaw,
                look_pitch,
            },
            time_in_ms: 0,
        }
    }

    /// The spawns a client got, by player id.
    fn spawns_sent_to(
        server: &MatchServer,
        addr: SocketAddr,
    ) -> Vec<(u8, survarium::player_input::player)> {
        sent_to(server, addr)
            .into_iter()
            .filter_map(|message| match message {
                ServerGameMessageKind::SpawnPlayer { player_id, player } => {
                    Some((player_id, player))
                }
                _ => None,
            })
            .collect()
    }

    /// A newcomer sees the others where they last stood and looked, a player
    /// who has not moved yet at its spawn point, and no dead player.
    #[test]
    fn a_late_joiner_sees_the_others_where_they_are_and_not_the_dead() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let newcomer = peer_addr(16_620);
        let moved = peer_addr(16_621);
        let dead = peer_addr(16_622);
        let still = peer_addr(16_623);
        server
            .peers
            .insert(newcomer, loading_peer(DEFAULT_MATCH_ID, 0));
        server.peers.insert(moved, joined_peer(DEFAULT_MATCH_ID, 1));
        server.peers.insert(
            dead,
            Peer {
                alive: false,
                ..joined_peer(DEFAULT_MATCH_ID, 2)
            },
        );
        server.peers.insert(
            still,
            Peer {
                spawn_index: 5,
                ..joined_peer(DEFAULT_MATCH_ID, 3)
            },
        );
        let feet = float3 {
            x: 10.0,
            y: 0.5,
            z: -20.0,
        };
        server.handle_game_message(moved, facing_update(feet, 1.25, -0.375));

        server.handle_game_message(newcomer, ClientGameMessageKind::TeamBasesInitializeInfo);

        let spawns = spawns_sent_to(&server, newcomer);
        assert_eq!(spawns.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [1, 3]);
        let (_, moved) = &spawns[0];
        assert_eq!(moved.position, feet);
        assert_eq!((moved.orientation, moved.look_pitch), (1.25, -0.375));
        assert!(moved.is_alive);
        let (_, still) = &spawns[1];
        let point = &map::RESPAWN_POINTS[5];
        assert_eq!(still.position, point.position);
        assert_eq!(
            (still.orientation, still.look_pitch),
            (point.yaw, game::SPAWN_LOOK_PITCH)
        );
    }

    /// A newcomer sees a player's weapon in hand, its magazine as fired, its
    /// random generators where the next shot draws, and its stamina times on
    /// the newcomer's own clock.
    #[test]
    fn a_late_joiner_sees_what_a_player_holds_now() {
        use crate::combat::weapon::bits;
        use survarium::player_input::{player_inventory_slot, weapon_targets};
        let Some(mut server) = combat_server() else {
            return;
        };
        let shooter = peer_addr(20_300);
        let newcomer = peer_addr(20_301);
        server
            .peers
            .insert(shooter, loading_peer(DEFAULT_MATCH_ID, 0));
        let start = Instant::now();
        server.handle_game_message_at(shooter, ClientGameMessageKind::JoinMatch, start);
        let initial_seeds = server.peers[&shooter].weapon_seeds;
        let (from, yaw, _) = server.combat.as_ref().unwrap().firing_range(8.0);
        let mut last = 0;
        for frame in 0..=150_u64 {
            last = frame as u32 * 16;
            let mask = match frame {
                0..=40 => bits::FIRE,
                60 => bits::WEAPON_2,
                _ => 0,
            };
            let at = start + Duration::from_millis(frame * 16);
            server.handle_game_message_at(shooter, aim_update(from, yaw, mask, last), at);
            server.tick_combat_at(at);
        }
        let at = start + Duration::from_millis(u64::from(last));

        server
            .peers
            .insert(newcomer, loading_peer(DEFAULT_MATCH_ID, 1));
        server.handle_game_message_at(
            newcomer,
            ClientGameMessageKind::TimeSynchronizationRequest {
                client_time_ms: 50_000,
            },
            at,
        );
        server.handle_game_message_at(newcomer, ClientGameMessageKind::TeamBasesInitializeInfo, at);

        let kit = server.combat.as_ref().unwrap().kit(shooter).unwrap();
        let spawns = spawns_sent_to(&server, newcomer);
        let [(0, shown)] = spawns.as_slice() else {
            panic!("{spawns:?}");
        };
        // Shown with the weapon a freshly inserted player holds, switching to
        // the one in hand: activating it while reading would crash the client.
        assert_eq!(
            shown.server_current_active_slot,
            profile_slot_enum::weapon1_slot
        );
        assert_eq!(
            shown.server_target_active_slot,
            profile_slot_enum::weapon2_slot
        );
        let player_inventory_slot::weapon_slot(ak) = shown.player_inventory[0] else {
            panic!("{:?}", shown.player_inventory[0]);
        };
        let ak_now = kit.weapons[0].unwrap();
        assert!(ak.ammo_in_magazine < 30, "{}", ak.ammo_in_magazine);
        assert_eq!(ak.ammo_in_magazine, ak_now.load.0);
        assert_eq!((ak.random_seed, ak.normal_random_seed as i32), ak_now.seeds);
        assert_ne!(ak_now.seeds, initial_seeds[0], "the shots drew from them");
        assert_eq!(ak.weapon_target, weapon_targets::idle);
        let carried = kit
            .amounts
            .iter()
            .find(|(slot, _)| *slot == profile_slot_enum::ammo1_weapon1_slot)
            .unwrap()
            .1;
        assert_eq!(
            shown.player_inventory[1],
            player_inventory_slot::item_amount(carried as u16)
        );
        // Its last tick was this very moment, on the newcomer's clock.
        assert_eq!(kit.stamina.last_tick_time_in_ms, last);
        assert_eq!(shown.player_stamina.last_tick_time_in_ms, 50_000);
        assert_eq!(shown.player_stamina.value, kit.stamina.value);
    }

    #[test]
    fn times_cross_between_client_clocks_through_the_server() {
        let Some(mut server) = combat_server() else {
            return;
        };
        let (a, b, unknown) = (peer_addr(20_310), peer_addr(20_311), peer_addr(20_312));
        let at = Instant::now() + Duration::from_secs(100);
        let combat = server.combat.as_mut().unwrap();
        // a's clock reads 70 s, b's 1 s, at the same moment.
        combat.note_sync_request(a, 70_000, at);
        combat.note_sync_request(b, 1_000, at);
        assert_eq!(server.time_for(a, b, 69_000), Some(1));
        assert_eq!(server.time_for(a, b, 69_500), Some(500));
        assert_eq!(server.time_for(b, a, 900), Some(69_900));
        assert_eq!(server.time_for(a, b, 0), Some(0), "never stays never");
        assert_eq!(server.time_for(a, unknown, 69_000), None);
        assert_eq!(server.time_for(a, a, 123), Some(123));
    }

    /// A player joined at `start`, its client's clock reading 1000 ms at
    /// `start + 1 s`, then hurt in the left leg to breaking (leg damage) and
    /// in the head below half (concussion) at `start + 1.5 s`.
    fn wounded_player(server: &mut MatchServer, addr: SocketAddr, start: Instant) {
        server.peers.insert(addr, loading_peer(DEFAULT_MATCH_ID, 0));
        server.handle_game_message_at(addr, ClientGameMessageKind::JoinMatch, start);
        let feet = Vec3::from(map::RESPAWN_POINTS[0].position);
        let at = start + Duration::from_secs(1);
        server.handle_game_message_at(addr, aim_update(feet, 0.0, 0, 1000), at);
        let combat = server.combat.as_mut().unwrap();
        let health = |combat: &Combat, part: usize| combat.damage_state(addr).unwrap()[part].health;
        let (head, left_leg) = (2, 5);
        let hurt_at = start + Duration::from_millis(1500);
        let leg = health(combat, left_leg);
        combat.hurt(addr, "left_leg", leg, hurt_at);
        let head_health = health(combat, head);
        combat.hurt(addr, "head", head_health * 0.6, hurt_at);
        assert!(combat.damage_state(addr).is_some(), "still alive");
    }

    fn damage_models_sent_to(
        server: &MatchServer,
        addr: SocketAddr,
    ) -> Vec<(u8, Vec<BodyPartState>)> {
        sent_to(server, addr)
            .into_iter()
            .filter_map(|message| match message {
                ServerGameMessageKind::DamageModelState { player_id, parts } => {
                    Some((player_id, parts))
                }
                _ => None,
            })
            .collect()
    }

    /// Right after a player's spawn, a newcomer gets its damage model, the
    /// times on the newcomer's clock, with only the affects it hears of.
    #[test]
    fn a_late_joiner_gets_each_players_damage_model_after_its_spawn() {
        use crate::combat::damage::affect;
        let Some(mut server) = combat_server() else {
            return;
        };
        let wounded = peer_addr(20_400);
        let newcomer = peer_addr(20_401);
        let start = Instant::now();
        wounded_player(&mut server, wounded, start);
        let at = start + Duration::from_secs(2);
        server
            .peers
            .insert(newcomer, loading_peer(DEFAULT_MATCH_ID, 1));
        // A client that rejoins asks before it loads: its clock reads 20 s.
        server.handle_game_message_at(
            newcomer,
            ClientGameMessageKind::TimeSynchronizationRequest {
                client_time_ms: 20_000,
            },
            at,
        );
        server.handle_game_message_at(newcomer, ClientGameMessageKind::TeamBasesInitializeInfo, at);

        let sent = sent_to(&server, newcomer);
        let spawn = sent
            .iter()
            .position(|message| matches!(message, ServerGameMessageKind::SpawnPlayer { .. }))
            .unwrap();
        let ServerGameMessageKind::DamageModelState { player_id, parts } = &sent[spawn + 1] else {
            panic!("{:?}", sent[spawn + 1]);
        };
        assert_eq!(*player_id, 0);
        let state = server
            .combat
            .as_ref()
            .unwrap()
            .damage_state(wounded)
            .unwrap();
        assert_eq!(parts.len(), 15);
        let (head, left_leg) = (&parts[2], &parts[5]);
        assert_eq!(left_leg.health, state[5].health);
        assert_eq!(left_leg.health, 0.0);
        // Hurt 0.5 s before the newcomer's clock read 20 s.
        assert_eq!(left_leg.last_hit_time_ms, 19_500);
        assert_eq!(left_leg.affects, [(affect::LEG_DAMAGE, 19_500)]);
        assert_eq!(head.last_hit_time_ms, 19_500);
        assert!(
            state[2]
                .affects
                .iter()
                .any(|(a, _)| *a == affect::CONCUSSION),
            "{:?}",
            state[2]
        );
        assert_eq!(head.affects, [], "its copy would never run it out");
        assert_eq!(parts[0].last_hit_time_ms, 0, "never hit");
    }

    /// A newcomer's clock is unknown until its first time sync request: its
    /// damage models follow that.
    #[test]
    fn damage_models_wait_for_the_newcomers_clock() {
        let Some(mut server) = combat_server() else {
            return;
        };
        let wounded = peer_addr(20_410);
        let newcomer = peer_addr(20_411);
        let start = Instant::now();
        wounded_player(&mut server, wounded, start);
        let at = start + Duration::from_secs(2);
        server
            .peers
            .insert(newcomer, loading_peer(DEFAULT_MATCH_ID, 1));
        server.handle_game_message_at(newcomer, ClientGameMessageKind::TeamBasesInitializeInfo, at);
        server.handle_game_message_at(newcomer, ClientGameMessageKind::JoinMatch, at);
        assert_eq!(damage_models_sent_to(&server, newcomer), []);

        server.handle_game_message_at(
            newcomer,
            ClientGameMessageKind::TimeSynchronizationRequest {
                client_time_ms: 5_000,
            },
            at + Duration::from_millis(300),
        );
        let models = damage_models_sent_to(&server, newcomer);
        let [(0, parts)] = models.as_slice() else {
            panic!("{models:?}");
        };
        assert_eq!(parts[5].last_hit_time_ms, 5_000 - 800);
        assert!(server.peers[&newcomer].owed_damage_models.is_empty());
        // Nothing more with the next request.
        server.handle_game_message_at(
            newcomer,
            ClientGameMessageKind::TimeSynchronizationRequest {
                client_time_ms: 9_000,
            },
            at + Duration::from_millis(4300),
        );
        assert_eq!(damage_models_sent_to(&server, newcomer).len(), 1);
    }

    /// A client that got its roster before a player joined has no such player:
    /// nothing about it reaches that client (the 16:14:04 crash).
    #[test]
    fn a_later_player_is_never_named_to_a_client_that_does_not_have_it() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let early = peer_addr(16_700);
        let late = peer_addr(16_701);
        server
            .peers
            .insert(early, Peer::new_registered(DEFAULT_MATCH_ID, 0));
        let now = Instant::now();
        server.handle_game_message_at(early, ClientGameMessageKind::GetStartupInfo, now);
        server.handle_game_message_at(early, ClientGameMessageKind::JoinMatch, now);
        server
            .peers
            .insert(late, Peer::new_registered(DEFAULT_MATCH_ID, 1));
        server.handle_game_message_at(late, ClientGameMessageKind::GetStartupInfo, now);
        server.handle_game_message_at(late, ClientGameMessageKind::JoinMatch, now);
        server.handle_game_message_at(late, ClientGameMessageKind::TeamBasesInitializeInfo, now);
        kill(&mut server, late, 0, now);

        assert_eq!(server.peers[&early].roster, [0]);
        let named = |message: &ServerGameMessageKind| match message {
            ServerGameMessageKind::SpawnPlayer { player_id, .. }
            | ServerGameMessageKind::PlayerVisibilityChanged { player_id, .. } => Some(*player_id),
            ServerGameMessageKind::KillPlayer(kill) => Some(kill.victim_id),
            ServerGameMessageKind::PlayerKdStatsChanged { player_id, .. } => Some(*player_id),
            _ => None,
        };
        let to_early: Vec<u8> = sent_to(&server, early).iter().filter_map(named).collect();
        assert!(to_early.iter().all(|id| *id == 0), "{to_early:?}");
        let spawned_for_late: Vec<u8> = sent_to(&server, late)
            .iter()
            .filter_map(|message| match message {
                ServerGameMessageKind::SpawnPlayer { player_id, .. } => Some(*player_id),
                _ => None,
            })
            .collect();
        assert_eq!(spawned_for_late, [1, 0]);
    }

    /// Two accounts readied together through a queue of two: placed in the
    /// same new match at once.
    fn two_placed_together(store: &SessionStore) -> (u32, u32) {
        let ready = |email: &str| {
            let account_id = store.get_or_create_account([127, 0, 0, 1].into(), email);
            let session_id = store.create_session(account_id);
            let profile = store.account(account_id).unwrap().locked().profile_contents[0];
            (
                session_id,
                store.ready_for_match(session_id, profile, &[profile]),
            )
        };
        let (first, waiting) = ready("first@e2e.invalid");
        assert!(matches!(waiting, session::QueueState::Waiting { .. }));
        let (second, placed) = ready("second@e2e.invalid");
        assert!(matches!(placed, session::QueueState::Placed(_)));
        (first, second)
    }

    fn roster_sizes(server: &MatchServer, addr: SocketAddr) -> Vec<usize> {
        let mut sizes = Vec::new();
        for message in sent_to(server, addr) {
            match message {
                ServerGameMessageKind::MatchOptions { .. } => sizes.push(0),
                ServerGameMessageKind::PlayerProfile { .. } => *sizes.last_mut().unwrap() += 1,
                _ => (),
            }
        }
        sizes
    }

    /// Two players the queue sent to a new match together each get both in
    /// their list of players, though one connects and asks before the
    /// other has connected.
    #[test]
    fn players_sent_to_a_match_together_are_listed_to_each_other() {
        let store = Arc::new(SessionStore::with_queue_min_players(2));
        let (first, second) = two_placed_together(&store);
        let mut server = recording_server(store);
        let (a, b) = (peer_addr(16_720), peer_addr(16_721));
        let now = Instant::now();
        server.peers.insert(a, Peer::new());
        server.handle_game_message_at(
            a,
            ClientGameMessageKind::ConnectionRequest { session_id: first },
            now,
        );
        server.handle_game_message_at(a, ClientGameMessageKind::GetStartupInfo, now);
        server.answer_startup_requests(now + Duration::from_millis(300));
        assert!(roster_sizes(&server, a).is_empty(), "waits for the second");

        let later = now + Duration::from_millis(650);
        server.peers.insert(b, Peer::new());
        server.handle_game_message_at(
            b,
            ClientGameMessageKind::ConnectionRequest { session_id: second },
            later,
        );
        server.handle_game_message_at(b, ClientGameMessageKind::GetStartupInfo, later);
        server.answer_startup_requests(later);
        assert_eq!(roster_sizes(&server, a), [2]);
        assert_eq!(roster_sizes(&server, b), [2]);
        assert_eq!(server.peers[&a].roster, [0, 1]);
        assert_eq!(server.peers[&b].roster, [0, 1]);
    }

    /// A player placed with others who never connects holds their lists of
    /// players back `ROSTER_GRACE` at most.
    #[test]
    fn a_list_of_players_waits_for_a_no_show_only_so_long() {
        let store = Arc::new(SessionStore::with_queue_min_players(2));
        let (first, _) = two_placed_together(&store);
        let mut server = recording_server(store);
        let a = peer_addr(16_722);
        let now = Instant::now();
        server.peers.insert(a, Peer::new());
        server.handle_game_message_at(
            a,
            ClientGameMessageKind::ConnectionRequest { session_id: first },
            now,
        );
        server.handle_game_message_at(a, ClientGameMessageKind::GetStartupInfo, now);
        server.answer_startup_requests(now + ROSTER_GRACE / 2);
        assert!(roster_sizes(&server, a).is_empty());
        server.answer_startup_requests(now + ROSTER_GRACE);
        assert_eq!(roster_sizes(&server, a), [1]);
        // Answered once.
        server.answer_startup_requests(now + ROSTER_GRACE * 2);
        assert_eq!(roster_sizes(&server, a), [1]);
    }

    /// A player joining a running match on its own gets its list at once,
    /// with everyone already there.
    #[test]
    fn a_lone_newcomer_gets_its_list_at_once() {
        let store = Arc::new(SessionStore::with_queue_min_players(2));
        let (first, second) = two_placed_together(&store);
        let account_id = store.get_or_create_account([127, 0, 0, 1].into(), "third@e2e.invalid");
        let third = store.create_session(account_id);
        let profile = store.account(account_id).unwrap().locked().profile_contents[0];
        let mut server = recording_server(store.clone());
        let addrs = [peer_addr(16_723), peer_addr(16_724), peer_addr(16_725)];
        let now = Instant::now();
        for (addr, session_id) in addrs.into_iter().zip([first, second]) {
            server.peers.insert(addr, Peer::new());
            server.handle_game_message_at(
                addr,
                ClientGameMessageKind::ConnectionRequest { session_id },
                now,
            );
            server.handle_game_message_at(addr, ClientGameMessageKind::GetStartupInfo, now);
        }
        // The match has room: the third goes straight to it.
        assert!(matches!(
            store.ready_for_match(third, profile, &[profile]),
            session::QueueState::Placed(_)
        ));
        server.peers.insert(addrs[2], Peer::new());
        server.handle_game_message_at(
            addrs[2],
            ClientGameMessageKind::ConnectionRequest { session_id: third },
            now,
        );
        server.handle_game_message_at(addrs[2], ClientGameMessageKind::GetStartupInfo, now);
        assert_eq!(roster_sizes(&server, addrs[0]), [2]);
        assert_eq!(roster_sizes(&server, addrs[1]), [2]);
        assert_eq!(roster_sizes(&server, addrs[2]), [3]);
    }

    /// With player 1 gone, a client's roster lists players 0 and 2: it calls
    /// them 0 and 1.
    #[test]
    fn a_roster_with_a_gap_is_numbered_by_place() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let first = peer_addr(16_710);
        let third = peer_addr(16_712);
        server
            .peers
            .insert(first, Peer::new_registered(DEFAULT_MATCH_ID, 0));
        server
            .peers
            .insert(third, Peer::new_registered(DEFAULT_MATCH_ID, 2));
        let now = Instant::now();
        server.handle_game_message_at(third, ClientGameMessageKind::GetStartupInfo, now);
        server.handle_game_message_at(third, ClientGameMessageKind::JoinMatch, now);

        let sent = sent_to(&server, third);
        let profiles: Vec<(u32, bool)> = sent
            .iter()
            .filter_map(|message| match message {
                ServerGameMessageKind::PlayerProfile { player_profile } => {
                    Some((player_profile.account_id, player_profile.is_local))
                }
                _ => None,
            })
            .collect();
        assert_eq!(profiles, [(0, false), (1, true)]);
        assert!(sent.iter().any(|message| matches!(
            message,
            ServerGameMessageKind::SpawnPlayer { player_id: 1, .. }
        )));
    }

    fn recording_server(store: Arc<SessionStore>) -> MatchServer {
        let mut server = MatchServer::bind("127.0.0.1:0", store).unwrap();
        server.rules = MatchRules::default();
        server.recorded = Some(Vec::new());
        server
    }

    impl Peer {
        /// A registered player whose client has not asked for its roster yet.
        fn new_registered(match_id: u32, player_id: u8) -> Self {
            Self {
                registered: true,
                match_id,
                player_id,
                ..Self::new()
            }
        }
    }

    fn loading_peer(match_id: u32, player_id: u8) -> Peer {
        Peer {
            registered: true,
            match_id,
            player_id,
            roster: full_roster(),
            ..Peer::new()
        }
    }

    fn sent_to(server: &MatchServer, addr: SocketAddr) -> Vec<ServerGameMessageKind> {
        server
            .recorded
            .iter()
            .flatten()
            .filter(|(to, _)| *to == addr)
            .map(|(_, message)| message.clone())
            .collect()
    }

    /// Only the lifecycle messages (status, timers, scoreboard, finish).
    fn lifecycle_sent_to(server: &MatchServer, addr: SocketAddr) -> Vec<ServerGameMessageKind> {
        sent_to(server, addr)
            .into_iter()
            .filter(|message| {
                matches!(
                    message,
                    ServerGameMessageKind::GameStatusChanged { .. }
                        | ServerGameMessageKind::MatchWaitTimeChanged { .. }
                        | ServerGameMessageKind::MatchTimeChanged { .. }
                        | ServerGameMessageKind::RespawnTimeChanged { .. }
                        | ServerGameMessageKind::PlayerKdStatsChanged { .. }
                        | ServerGameMessageKind::MatchFinished
                )
            })
            .collect()
    }

    fn status(status: game_status) -> ServerGameMessageKind {
        ServerGameMessageKind::GameStatusChanged { status }
    }

    fn kd(player_id: u8, kills: u32, deaths: u32) -> ServerGameMessageKind {
        ServerGameMessageKind::PlayerKdStatsChanged {
            player_id,
            kills,
            deaths,
        }
    }

    fn match_in(server: &MatchServer, phase: Phase, now: Instant) -> MatchState {
        let mut state = MatchState::new(now, &server.rules);
        state.set_phase(phase);
        state.mark_announced(now);
        state
    }

    #[test]
    fn first_join_spawns_the_player_then_starts_the_final_countdown() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let addr = peer_addr(18_000);
        server.peers.insert(addr, loading_peer(DEFAULT_MATCH_ID, 0));

        server.handle_game_message(addr, ClientGameMessageKind::JoinMatch);

        let sent = sent_to(&server, addr);
        assert!(
            matches!(
                sent[0],
                ServerGameMessageKind::SpawnPlayer { player_id: 0, .. }
            ),
            "the status must follow the player's own spawn: {sent:?}"
        );
        assert_eq!(
            sent[1..3],
            [
                status(game_status::final_countdown),
                ServerGameMessageKind::MatchWaitTimeChanged { time_left: 10 },
            ]
        );
        assert!(
            matches!(
                &sent[3..],
                [ServerGameMessageKind::InitializeVictoryItems { items, .. }] if items.len() == 10
            ),
            "then the match's batteries: {sent:?}"
        );
    }

    #[test]
    fn waiting_for_players_is_shown_when_a_share_of_players_is_required() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        server.rules.wait_player_percent = 50;
        let addr = peer_addr(18_050);
        server.peers.insert(addr, loading_peer(DEFAULT_MATCH_ID, 0));

        server.handle_game_message(addr, ClientGameMessageKind::JoinMatch);

        assert_eq!(
            lifecycle_sent_to(&server, addr),
            [
                status(game_status::waiting_for_players),
                ServerGameMessageKind::MatchWaitTimeChanged { time_left: 60 },
            ]
        );
    }

    #[test]
    fn late_joiner_gets_the_running_match_and_its_scoreboard() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let now = Instant::now();
        let veteran = peer_addr(18_100);
        let mut veteran_peer = joined_peer(DEFAULT_MATCH_ID, 0);
        veteran_peer.kills = 3;
        veteran_peer.deaths = 1;
        server.peers.insert(veteran, veteran_peer);
        let running = match_in(
            &server,
            Phase::InProcess {
                ends_at: now + Duration::from_secs(300),
            },
            now,
        );
        server.matches.insert(DEFAULT_MATCH_ID, running);

        let newcomer = peer_addr(18_101);
        server
            .peers
            .insert(newcomer, loading_peer(DEFAULT_MATCH_ID, 1));
        server.handle_game_message(newcomer, ClientGameMessageKind::JoinMatch);

        let lifecycle = lifecycle_sent_to(&server, newcomer);
        assert_eq!(lifecycle[0], status(game_status::inprocess));
        assert!(
            matches!(
                lifecycle[1],
                ServerGameMessageKind::MatchTimeChanged { time_left_ms } if (299_000..=300_000).contains(&time_left_ms)
            ),
            "{lifecycle:?}"
        );
        assert_eq!(lifecycle[2..], [kd(0, 3, 1), kd(1, 0, 0)]);
        assert!(
            lifecycle_sent_to(&server, veteran).is_empty(),
            "the running match is not re-announced to the others"
        );
    }

    #[test]
    fn countdown_end_starts_the_match_resets_scores_and_revives_the_dead() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let start = Instant::now();
        let alive = peer_addr(18_200);
        let dead = peer_addr(18_201);
        let mut alive_peer = joined_peer(DEFAULT_MATCH_ID, 0);
        alive_peer.kills = 2;
        let mut dead_peer = joined_peer(DEFAULT_MATCH_ID, 1);
        dead_peer.alive = false;
        dead_peer.deaths = 2;
        dead_peer.respawn_at = Some(start + Duration::from_secs(60));
        server.peers.insert(alive, alive_peer);
        server.peers.insert(dead, dead_peer);
        let mut state = MatchState::new(start, &server.rules);
        state.advance(start, &server.rules, 2, 20);
        server.matches.insert(DEFAULT_MATCH_ID, state);

        let countdown_end = start + server.rules.countdown();
        server.tick_matches_at(countdown_end - Duration::from_millis(1));
        assert_eq!(
            lifecycle_sent_to(&server, dead),
            [ServerGameMessageKind::MatchWaitTimeChanged { time_left: 1 }]
        );
        server.recorded = Some(Vec::new());
        server.tick_matches_at(countdown_end);
        server.respawn_players_at(countdown_end);

        let sent = sent_to(&server, dead);
        assert_eq!(
            sent[..5],
            [
                status(game_status::inprocess),
                ServerGameMessageKind::MatchTimeChanged {
                    time_left_ms: 900_000
                },
                kd(0, 0, 0),
                kd(1, 0, 0),
                ServerGameMessageKind::RespawnTimeChanged { time_left: 0 },
            ]
        );
        assert!(matches!(
            sent[5],
            ServerGameMessageKind::SpawnPlayer { player_id: 1, .. }
        ));
        assert!(server.peers[&dead].alive);
        assert_eq!(server.peers[&alive].kills, 0);
    }

    #[test]
    fn pushes_the_match_timer_every_second() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let now = Instant::now();
        let player = peer_addr(18_250);
        server
            .peers
            .insert(player, joined_peer(DEFAULT_MATCH_ID, 0));
        let running = match_in(
            &server,
            Phase::InProcess {
                ends_at: now + Duration::from_secs(60),
            },
            now,
        );
        server.matches.insert(DEFAULT_MATCH_ID, running);

        for millis in [0, 500, 1_000, 1_999, 2_000] {
            server.tick_matches_at(now + Duration::from_millis(millis));
        }

        assert_eq!(
            lifecycle_sent_to(&server, player),
            [
                ServerGameMessageKind::MatchTimeChanged {
                    time_left_ms: 59_000
                },
                ServerGameMessageKind::MatchTimeChanged {
                    time_left_ms: 58_000
                },
            ]
        );
    }

    #[test]
    fn kills_update_the_scoreboard_only_while_the_match_is_in_process() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let killer = peer_addr(18_300);
        let victim = peer_addr(18_301);
        server
            .peers
            .insert(killer, joined_peer(DEFAULT_MATCH_ID, 0));
        server
            .peers
            .insert(victim, joined_peer(DEFAULT_MATCH_ID, 1));
        let now = Instant::now();
        let countdown = match_in(
            &server,
            Phase::FinalCountdown {
                ends_at: now + Duration::from_secs(10),
            },
            now,
        );
        server.matches.insert(DEFAULT_MATCH_ID, countdown);

        let kill = PlayerKill {
            victim_id: 1,
            killer_id: 0,
            is_headshot: false,
            item_dict_id: 13,
        };
        server.record_kill(DEFAULT_MATCH_ID, &kill, now);
        assert_eq!(server.peers[&killer].kills, 0);
        assert_eq!(server.peers[&victim].deaths, 0);
        assert!(lifecycle_sent_to(&server, killer).is_empty());

        server
            .matches
            .get_mut(&DEFAULT_MATCH_ID)
            .unwrap()
            .set_phase(Phase::InProcess {
                ends_at: now + Duration::from_secs(60),
            });
        server.record_kill(DEFAULT_MATCH_ID, &kill, now);
        for addr in [killer, victim] {
            assert_eq!(lifecycle_sent_to(&server, addr), [kd(0, 1, 0), kd(1, 0, 1)]);
        }

        let suicide = PlayerKill {
            victim_id: 0,
            killer_id: 0,
            ..kill
        };
        server.record_kill(DEFAULT_MATCH_ID, &suicide, now);
        assert_eq!(server.peers[&killer].kills, 1);
        assert_eq!(server.peers[&killer].deaths, 1);
    }

    #[test]
    fn dead_player_sees_a_respawn_countdown_that_clears_on_respawn() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let attacker = peer_addr(18_400);
        let victim = peer_addr(18_401);
        server
            .peers
            .insert(attacker, joined_peer(DEFAULT_MATCH_ID, 0));
        server
            .peers
            .insert(victim, joined_peer(DEFAULT_MATCH_ID, 1));
        let killed_at = Instant::now();

        kill(&mut server, victim, 0, killed_at);
        for elapsed in [Duration::from_millis(500), Duration::from_secs(1)] {
            server.respawn_players_at(killed_at + elapsed);
        }
        server.respawn_players_at(killed_at + server.rules.respawn_time());

        let timers: Vec<u32> = sent_to(&server, victim)
            .into_iter()
            .filter_map(|message| match message {
                ServerGameMessageKind::RespawnTimeChanged { time_left } => Some(time_left),
                _ => None,
            })
            .collect();
        assert_eq!(timers, [10, 9, 0]);
        assert!(server.peers[&victim].alive);
        assert!(
            sent_to(&server, attacker).iter().all(|message| !matches!(
                message,
                ServerGameMessageKind::RespawnTimeChanged { .. }
            )),
            "only the dead player sees its respawn countdown"
        );
    }

    #[test]
    fn time_up_finishes_the_match_and_frees_its_matchmaking_slots() {
        let store = Arc::new(SessionStore::new());
        let assignment = store.join_match(1);
        store.join_match(2);
        let mut server = recording_server(Arc::clone(&store));
        let first = peer_addr(18_500);
        let second = peer_addr(18_501);
        for (addr, session_id, player_id) in [(first, 1, 0), (second, 2, 1)] {
            let mut peer = joined_peer(assignment.match_id, player_id);
            peer.session_id = Some(session_id);
            server.peers.insert(addr, peer);
        }
        let now = Instant::now();
        let ending = match_in(
            &server,
            Phase::InProcess {
                ends_at: now + Duration::from_secs(5),
            },
            now,
        );
        server.matches.insert(assignment.match_id, ending);

        let match_end = now + Duration::from_secs(5);
        server.tick_matches_at(match_end);
        for addr in [first, second] {
            assert_eq!(
                lifecycle_sent_to(&server, addr),
                [ServerGameMessageKind::MatchTimeChanged { time_left_ms: 0 }]
            );
        }
        server.tick_matches_at(match_end + server.rules.time_up());
        for addr in [first, second] {
            assert_eq!(
                lifecycle_sent_to(&server, addr),
                [
                    ServerGameMessageKind::MatchTimeChanged { time_left_ms: 0 },
                    ServerGameMessageKind::MatchFinished,
                ]
            );
        }
        assert_eq!(store.match_assignment(1), None);
        assert_eq!(store.match_assignment(2), None);
        assert_ne!(
            store.join_match(3).match_id,
            assignment.match_id,
            "the next ready starts a new match"
        );
    }

    /// A joined player of the default match with a signed-in session.
    fn player_with_session(store: &SessionStore, email: &str, player_id: u8) -> (u32, Peer) {
        let account = store.get_or_create_account(std::net::IpAddr::from([127, 0, 0, 1]), email);
        let session_id = store.create_session(account);
        let peer = Peer {
            session_id: Some(session_id),
            ..joined_peer(DEFAULT_MATCH_ID, player_id)
        };
        (session_id, peer)
    }

    #[test]
    fn a_player_who_vanishes_mid_match_keeps_its_kills_but_not_the_finish() {
        let store = Arc::new(SessionStore::new());
        let mut server = recording_server(store.clone());
        let (session_id, mut peer) = player_with_session(&store, "vanisher", 0);
        peer.kills = 2;
        peer.steals = 1;
        peer.last_seen = Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);
        server.peers.insert(peer_addr(19_600), peer);

        server.reap_peers();

        assert_eq!(
            store.take_match_results(session_id),
            [MatchResult {
                kills: 2,
                steals: 1,
                finished: false,
                used: Vec::new(),
            }]
        );
    }

    #[test]
    fn a_finished_match_records_each_player_once() {
        let store = Arc::new(SessionStore::new());
        let mut server = recording_server(store.clone());
        let (session_id, mut peer) = player_with_session(&store, "finisher", 0);
        peer.kills = 3;
        let addr = peer_addr(19_601);
        server.peers.insert(addr, peer);

        server.finish_match(DEFAULT_MATCH_ID);
        // It leaves afterwards, as clients do.
        server.peers.get_mut(&addr).unwrap().last_seen =
            Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);
        server.reap_peers();

        let results = store.take_match_results(session_id);
        assert_eq!(results.len(), 1);
        assert!(results[0].finished);
        assert_eq!(results[0].kills, 3);
    }

    #[test]
    fn a_player_who_never_joined_has_no_result() {
        let store = Arc::new(SessionStore::new());
        let mut server = recording_server(store.clone());
        let (session_id, peer) = player_with_session(&store, "loader", 0);
        server.peers.insert(
            peer_addr(19_602),
            Peer {
                joined: false,
                last_seen: Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1),
                ..peer
            },
        );
        server.reap_peers();
        assert!(store.take_match_results(session_id).is_empty());
    }

    #[test]
    fn a_spawn_hands_out_the_life_the_supplies_give() {
        let store = Arc::new(SessionStore::new());
        let mut server = recording_server(store.clone());
        let (_, mut peer) = player_with_session(&store, "supplied", 0);
        // 40 rounds of 5.45 left: the AKS-74U's magazine takes 30.
        peer.profile.slots[profile_slot_enum::ammo1_weapon1_slot].amount_in_inventory = 40;
        let addr = peer_addr(19_603);
        server.peers.insert(addr, peer);

        server.spawn_fighter(addr, Instant::now());
        let ServerGameMessageKind::SpawnPlayer { player, .. } =
            server.spawn_message(DEFAULT_MATCH_ID, 0)
        else {
            panic!("not a spawn");
        };
        let survarium::player_input::player_inventory_slot::weapon_slot(ak) =
            &player.player_inventory[0]
        else {
            panic!("{:?}", player.player_inventory[0]);
        };
        assert_eq!(ak.ammo_in_magazine, 30);
        assert_eq!(
            // The slots in order: the weapon, then its first ammo.
            player.player_inventory[1],
            survarium::player_input::player_inventory_slot::item_amount(10),
            "the rest of the 40, carried"
        );
    }

    #[test]
    fn nothing_follows_match_finished() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let leaving = peer_addr(19_500);
        let staying = peer_addr(19_501);
        server
            .peers
            .insert(leaving, joined_peer(DEFAULT_MATCH_ID, 0));
        server
            .peers
            .insert(staying, joined_peer(DEFAULT_MATCH_ID, 1));
        server.send_to(leaving, ServerGameMessageKind::MatchFinished);
        let trap_fired = ServerGameMessageKind::TrapFired {
            player_id: 1,
            slot: 0x0E,
            index: 0,
        };
        server.broadcast_to_match(DEFAULT_MATCH_ID, trap_fired.clone());

        assert_eq!(
            sent_to(&server, leaving),
            [ServerGameMessageKind::MatchFinished]
        );
        assert_eq!(sent_to(&server, staying), [trap_fired]);
    }

    #[test]
    fn rejoining_a_finished_match_id_starts_a_new_lifecycle() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let now = Instant::now();
        let finished = match_in(&server, Phase::Finished { at: now }, now);
        server.matches.insert(DEFAULT_MATCH_ID, finished);
        let addr = peer_addr(18_550);
        server.peers.insert(addr, loading_peer(DEFAULT_MATCH_ID, 0));

        server.handle_game_message(addr, ClientGameMessageKind::JoinMatch);

        assert_eq!(
            lifecycle_sent_to(&server, addr)[0],
            status(game_status::final_countdown)
        );
    }

    #[test]
    fn finished_match_drops_lingering_peers_after_the_grace_period() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let lingering = peer_addr(18_600);
        server
            .peers
            .insert(lingering, joined_peer(DEFAULT_MATCH_ID, 0));
        let now = Instant::now();
        let finished = match_in(&server, Phase::Finished { at: now }, now);
        server.matches.insert(DEFAULT_MATCH_ID, finished);
        let grace = server.rules.finished_grace();

        server.tick_matches_at(now + grace - Duration::from_millis(1));
        assert!(server.peers.contains_key(&lingering));

        server.tick_matches_at(now + grace);
        assert!(!server.peers.contains_key(&lingering));
        assert!(!server.matches.contains_key(&DEFAULT_MATCH_ID));
    }

    #[test]
    fn match_closes_when_its_last_player_leaves() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let now = Instant::now();
        let running = match_in(
            &server,
            Phase::InProcess {
                ends_at: now + Duration::from_secs(60),
            },
            now,
        );
        server.matches.insert(DEFAULT_MATCH_ID, running);

        server.tick_matches_at(now);

        assert!(server.matches.is_empty());
    }

    #[test]
    fn console_commands_start_retime_and_end_a_match() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let player = peer_addr(18_700);
        server
            .peers
            .insert(player, joined_peer(DEFAULT_MATCH_ID, 0));
        let now = Instant::now();
        let mut state = MatchState::new(now, &server.rules);
        state.advance(now, &server.rules, 1, 20);
        server.matches.insert(DEFAULT_MATCH_ID, state);

        server.run_console_command(Command::Start { match_id: None }, now);
        assert!(server.matches[&DEFAULT_MATCH_ID].phase().is_in_process());
        server.run_console_command(
            Command::Time {
                seconds: 30,
                match_id: Some(DEFAULT_MATCH_ID),
            },
            now,
        );
        server.tick_matches_at(now);
        server.run_console_command(
            Command::End {
                match_id: Some(DEFAULT_MATCH_ID),
            },
            now,
        );
        server.run_console_command(
            Command::End {
                match_id: Some(0x999),
            },
            now,
        );
        server.run_console_command(Command::Status, now);
        server.tick_matches_at(now + server.rules.time_up());

        assert_eq!(
            lifecycle_sent_to(&server, player),
            [
                status(game_status::inprocess),
                ServerGameMessageKind::MatchTimeChanged {
                    time_left_ms: 900_000
                },
                kd(0, 0, 0),
                ServerGameMessageKind::MatchTimeChanged {
                    time_left_ms: 30_000
                },
                ServerGameMessageKind::MatchTimeChanged { time_left_ms: 0 },
                ServerGameMessageKind::MatchFinished,
            ]
        );
    }

    #[test]
    fn spawn_selection_is_isolated_per_match() {
        let mut server = MatchServer::bind("127.0.0.1:0", Arc::new(SessionStore::new())).unwrap();
        server.spawn_rng = SpawnRng::new(7);
        let occupied = peer_addr(17_000);
        server
            .peers
            .insert(occupied, joined_peer(DEFAULT_MATCH_ID, 0));
        let (base, _) = points_by_priority(game_team_id::team_1);
        server.peers.get_mut(&occupied).unwrap().spawn_index = base[0];
        let spawning = peer_addr(17_001);

        let selection =
            server.select_spawn_index(DEFAULT_MATCH_ID + 1, game_team_id::team_1, spawning);

        let mut empty_server =
            MatchServer::bind("127.0.0.1:0", Arc::new(SessionStore::new())).unwrap();
        empty_server.spawn_rng = SpawnRng::new(7);
        assert_eq!(
            selection,
            empty_server.select_spawn_index(DEFAULT_MATCH_ID + 1, game_team_id::team_1, spawning),
            "an occupied point in another match must not affect this match"
        );
    }

    /// A client update standing at `feet`, with the use key down or up.
    fn update_at(feet: float3, use_key: bool) -> ClientGameMessageKind {
        use survarium::player_input::{player_input, player_state};
        use vostok::math::float2;
        ClientGameMessageKind::ClientPlayerUpdate {
            player_input: player_input {
                angular_velocity: float2 { x: 0.0, y: 0.0 },
                angualr_acceleration: float2 { x: 0.0, y: 0.0 },
                action_mask: if use_key { USE_ACTION } else { 0 },
            },
            player_state: player_state {
                translation_w: feet,
                euler_angle_y: 0.0,
                look_pitch: 0.0,
            },
            time_in_ms: 0,
        }
    }

    /// Feet beside `point`, close enough to use it.
    fn beside(point: float3) -> float3 {
        float3 {
            x: point.x + 0.5,
            y: point.y - 0.8,
            ..point
        }
    }

    fn far_away() -> float3 {
        float3 {
            x: 500.0,
            y: 0.0,
            z: 500.0,
        }
    }

    /// A running match whose `count` batteries lie at the first spawners.
    fn running_battery_match(server: &mut MatchServer, count: u8, now: Instant) {
        let running = match_in(
            server,
            Phase::InProcess {
                ends_at: now + Duration::from_secs(300),
            },
            now,
        );
        server.matches.insert(DEFAULT_MATCH_ID, running);
        server
            .batteries
            .insert(DEFAULT_MATCH_ID, Batteries::spawn(count, |_| 0));
    }

    fn battery_messages(server: &MatchServer, addr: SocketAddr) -> Vec<ServerGameMessageKind> {
        sent_to(server, addr)
            .into_iter()
            .filter(|message| {
                matches!(
                    message,
                    ServerGameMessageKind::InitializeVictoryItems { .. }
                        | ServerGameMessageKind::VictoryItemTakeOrPut { .. }
                )
            })
            .collect()
    }

    fn took_or_put(player_id: u8, item_id: u8, action: VictoryItemAction) -> ServerGameMessageKind {
        ServerGameMessageKind::VictoryItemTakeOrPut {
            player_id,
            item_id,
            action,
        }
    }

    fn team_base(team: game_team_id) -> &'static map::BatteryBase {
        map::battery_base_of(team).unwrap()
    }

    #[test]
    fn the_use_key_picks_up_a_battery_in_reach_once_the_match_runs() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let carrier = peer_addr(19_000);
        let teammate = peer_addr(19_001);
        server
            .peers
            .insert(carrier, joined_peer(DEFAULT_MATCH_ID, 0));
        server
            .peers
            .insert(teammate, joined_peer(DEFAULT_MATCH_ID, 1));
        let now = Instant::now();
        running_battery_match(&mut server, 2, now);
        let countdown = match_in(
            &server,
            Phase::FinalCountdown {
                ends_at: now + Duration::from_secs(5),
            },
            now,
        );
        server.matches.insert(DEFAULT_MATCH_ID, countdown);
        let at_battery = beside(map::BATTERY_SPAWNS[0]);

        server.handle_game_message(carrier, update_at(at_battery, true));
        assert!(
            battery_messages(&server, carrier).is_empty(),
            "nothing can be used before the match starts"
        );

        server.handle_game_message(carrier, update_at(at_battery, false));
        running_battery_match(&mut server, 2, now);
        server.handle_game_message(carrier, update_at(far_away(), true));
        assert!(
            battery_messages(&server, carrier).is_empty(),
            "out of reach"
        );

        // Walking in with the key held uses the battery on arrival, once.
        server.handle_game_message(carrier, update_at(at_battery, true));
        server.handle_game_message(carrier, update_at(at_battery, true));
        let taken = [took_or_put(0, 0, VictoryItemAction::TakeFromGround)];
        assert_eq!(battery_messages(&server, carrier), taken);
        assert_eq!(battery_messages(&server, teammate), taken);
    }

    #[test]
    fn a_held_use_key_survives_the_clients_brief_drops_of_the_bit() {
        let mut peer = joined_peer(DEFAULT_MATCH_ID, 0);
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);

        assert_eq!(peer.track_use_key(true, at(0)), Some(true));
        assert_eq!(peer.track_use_key(false, at(33)), None);
        assert_eq!(peer.track_use_key(false, at(133)), None);
        assert_eq!(
            peer.track_use_key(true, at(166)),
            Some(false),
            "still the same press"
        );

        peer.use_target = Some(UseTarget::Battery(0));
        assert_eq!(peer.track_use_key(false, at(200)), None);
        assert_eq!(peer.track_use_key(false, at(399)), None);
        assert!(peer.use_down);
        assert_eq!(peer.track_use_key(false, at(400)), None);
        assert!(!peer.use_down, "released after 200 ms without the bit");
        assert_eq!(peer.use_target, None);
        assert_eq!(peer.track_use_key(true, at(450)), Some(true));
    }

    #[test]
    fn a_dead_player_cannot_use_anything() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let addr = peer_addr(19_050);
        let mut dead = joined_peer(DEFAULT_MATCH_ID, 0);
        dead.alive = false;
        server.peers.insert(addr, dead);
        running_battery_match(&mut server, 1, Instant::now());

        server.handle_game_message(addr, update_at(beside(map::BATTERY_SPAWNS[0]), true));

        assert!(battery_messages(&server, addr).is_empty());
    }

    #[test]
    fn storing_every_battery_at_the_own_base_ends_the_match() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let carrier = peer_addr(19_100);
        let enemy = peer_addr(19_101);
        server
            .peers
            .insert(carrier, joined_peer(DEFAULT_MATCH_ID, 0));
        let mut enemy_peer = joined_peer(DEFAULT_MATCH_ID, 1);
        enemy_peer.team = game_team_id::team_2;
        server.peers.insert(enemy, enemy_peer);
        running_battery_match(&mut server, 1, Instant::now());
        let base = team_base(game_team_id::team_1);

        server.handle_game_message(carrier, update_at(beside(map::BATTERY_SPAWNS[0]), true));
        server.handle_game_message(
            carrier,
            update_at(
                beside(team_base(game_team_id::team_2).use_zone_center),
                true,
            ),
        );
        server.handle_game_message(carrier, update_at(beside(base.use_zone_center), true));

        let expected = [
            took_or_put(0, 0, VictoryItemAction::TakeFromGround),
            took_or_put(
                0,
                0,
                VictoryItemAction::PutInBase {
                    container_id: base.id,
                },
            ),
        ];
        assert_eq!(battery_messages(&server, carrier), expected);
        assert_eq!(battery_messages(&server, enemy), expected);
        assert!(matches!(
            server.matches[&DEFAULT_MATCH_ID].phase(),
            Phase::TimeUp { .. }
        ));
        assert_eq!(
            lifecycle_sent_to(&server, enemy),
            [ServerGameMessageKind::MatchTimeChanged { time_left_ms: 0 }]
        );
    }

    #[test]
    fn an_enemy_steals_from_a_stocked_base() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let defender = peer_addr(19_150);
        let thief = peer_addr(19_151);
        server
            .peers
            .insert(defender, joined_peer(DEFAULT_MATCH_ID, 0));
        let mut thief_peer = joined_peer(DEFAULT_MATCH_ID, 1);
        thief_peer.team = game_team_id::team_2;
        server.peers.insert(thief, thief_peer);
        running_battery_match(&mut server, 2, Instant::now());
        let base = team_base(game_team_id::team_1);
        server.handle_game_message(defender, update_at(beside(map::BATTERY_SPAWNS[0]), true));
        server.handle_game_message(defender, update_at(beside(base.use_zone_center), true));

        server.handle_game_message(thief, update_at(beside(base.use_zone_center), true));

        assert_eq!(
            battery_messages(&server, defender).last(),
            Some(&took_or_put(
                1,
                0,
                VictoryItemAction::TakeFromBase {
                    container_id: base.id,
                },
            ))
        );
    }

    #[test]
    fn a_killed_carrier_drops_the_battery_where_it_fell() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let shooter = peer_addr(19_200);
        let carrier = peer_addr(19_201);
        server
            .peers
            .insert(shooter, joined_peer(DEFAULT_MATCH_ID, 0));
        server
            .peers
            .insert(carrier, joined_peer(DEFAULT_MATCH_ID, 1));
        running_battery_match(&mut server, 1, Instant::now());
        server.handle_game_message(carrier, update_at(beside(map::BATTERY_SPAWNS[0]), true));
        let fell_at = float3 {
            x: 10.0,
            y: 0.5,
            z: -20.0,
        };
        server.handle_game_message(carrier, update_at(fell_at, false));

        kill(&mut server, carrier, 0, Instant::now());

        let sent = sent_to(&server, shooter);
        let kill = sent
            .iter()
            .position(|message| matches!(message, ServerGameMessageKind::KillPlayer(..)))
            .expect("the carrier was killed");
        assert_eq!(
            sent[kill + 1..]
                .iter()
                .filter(|message| matches!(
                    message,
                    ServerGameMessageKind::VictoryItemTakeOrPut { .. }
                ))
                .collect::<Vec<_>>(),
            [&took_or_put(
                1,
                0,
                VictoryItemAction::Drop { position: fell_at }
            )]
        );
    }

    #[test]
    fn a_leaving_carrier_drops_the_battery_before_it_disappears() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let carrier = peer_addr(19_250);
        let remaining = peer_addr(19_251);
        server
            .peers
            .insert(carrier, joined_peer(DEFAULT_MATCH_ID, 0));
        server
            .peers
            .insert(remaining, joined_peer(DEFAULT_MATCH_ID, 1));
        running_battery_match(&mut server, 1, Instant::now());
        let at_battery = beside(map::BATTERY_SPAWNS[0]);
        server.handle_game_message(carrier, update_at(at_battery, true));
        server.peers.get_mut(&carrier).unwrap().last_seen =
            Instant::now() - IDLE_TIMEOUT - Duration::from_secs(1);

        server.reap_peers();

        let sent = sent_to(&server, remaining);
        let drop = sent
            .iter()
            .position(|message| {
                *message
                    == took_or_put(
                        0,
                        0,
                        VictoryItemAction::Drop {
                            position: at_battery,
                        },
                    )
            })
            .expect("the battery is dropped");
        let gone = sent
            .iter()
            .position(|message| {
                *message
                    == ServerGameMessageKind::PlayerVisibilityChanged {
                        player_id: 0,
                        is_visible: false,
                    }
            })
            .expect("the carrier disappears");
        assert!(drop < gone, "{sent:?}");
    }

    #[test]
    fn a_late_joiner_gets_the_battery_state_once() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let carrier = peer_addr(19_300);
        server
            .peers
            .insert(carrier, joined_peer(DEFAULT_MATCH_ID, 0));
        running_battery_match(&mut server, 2, Instant::now());
        let carrying_at = beside(map::BATTERY_SPAWNS[0]);
        server.handle_game_message(carrier, update_at(carrying_at, true));

        let newcomer = peer_addr(19_301);
        server
            .peers
            .insert(newcomer, loading_peer(DEFAULT_MATCH_ID, 1));
        server.handle_game_message(newcomer, ClientGameMessageKind::JoinMatch);
        server.send_battery_state(newcomer);

        let sent = battery_messages(&server, newcomer);
        let [ServerGameMessageKind::InitializeVictoryItems { items, .. }] = sent.as_slice() else {
            panic!("{sent:?}");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].carrier, Some(0));
        assert_eq!(items[0].position, carrying_at);
        assert_eq!(items[1].carrier, None);
    }

    /// A server with the game's combat data, or `None` (the test is skipped)
    /// where `resources.db` is not available.
    fn combat_server() -> Option<MatchServer> {
        let assets = CombatAssets::for_tests()?;
        let mut server = recording_server(Arc::new(SessionStore::new()));
        server.combat = Some(Combat::new(assets));
        Some(server)
    }

    fn aim_update(
        feet: Vec3,
        yaw: f32,
        action_mask: u32,
        time_in_ms: u32,
    ) -> ClientGameMessageKind {
        use survarium::player_input::{player_input, player_state};
        use vostok::math::float2;
        ClientGameMessageKind::ClientPlayerUpdate {
            player_input: player_input {
                angular_velocity: float2 { x: 0.0, y: 0.0 },
                angualr_acceleration: float2 { x: 0.0, y: 0.0 },
                action_mask,
            },
            player_state: player_state {
                translation_w: feet.into(),
                euler_angle_y: yaw,
                look_pitch: 0.0,
            },
            time_in_ms,
        }
    }

    /// Joins the shooter (player 0) and the victim (player 1, in
    /// `victim_match`), then for `millis` the victim stands 8 m in front of
    /// the crouched shooter, which holds aim and fire (its eye line crosses
    /// a standing torso). Client frames are 16 ms apart; `before_firing` runs
    /// once both have joined.
    fn fire_at_a_standing_player(
        server: &mut MatchServer,
        victim_match: u32,
        millis: u64,
        before_firing: impl FnOnce(&mut MatchServer, Instant),
    ) -> (SocketAddr, SocketAddr) {
        use crate::combat::weapon::bits;
        let shooter = peer_addr(20_000);
        let victim = peer_addr(20_001);
        server
            .peers
            .insert(shooter, loading_peer(DEFAULT_MATCH_ID, 0));
        server.peers.insert(victim, loading_peer(victim_match, 1));
        let start = Instant::now();
        for addr in [shooter, victim] {
            server.handle_game_message_at(addr, ClientGameMessageKind::JoinMatch, start);
        }
        before_firing(server, start);

        let (from, yaw, to) = server.combat.as_ref().unwrap().firing_range(8.0);
        let facing_the_shooter = yaw + std::f32::consts::PI;
        for frame in 0..=millis / 16 {
            let at = start + Duration::from_millis(frame * 16);
            let time = frame as u32 * 16;
            server.handle_game_message_at(victim, aim_update(to, facing_the_shooter, 0, time), at);
            let mask = bits::FIRE | bits::AIM | bits::CROUCH;
            server.handle_game_message_at(shooter, aim_update(from, yaw, mask, time), at);
            server.tick_combat_at(at);
        }
        (shooter, victim)
    }

    fn hits(server: &MatchServer, to: SocketAddr) -> Vec<PlayerHit> {
        sent_to(server, to)
            .into_iter()
            .filter_map(|message| match message {
                ServerGameMessageKind::HitPlayer(hit) => Some(hit),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn replayed_shots_hurt_and_kill_with_the_stock_messages() {
        let Some(mut server) = combat_server() else {
            return;
        };
        let (shooter, victim) =
            fire_at_a_standing_player(&mut server, DEFAULT_MATCH_ID, 3000, |_, _| {});

        let seen_by_shooter = hits(&server, shooter);
        assert!(!seen_by_shooter.is_empty(), "no hits");
        assert_eq!(seen_by_shooter, hits(&server, victim));
        for hit in &seen_by_shooter {
            assert_eq!((hit.hit_initiator, hit.being_hit), (0, 1));
            assert_eq!(hit.damage_type, "injury");
            assert!(hit.amount > 0.0 && hit.armor_piercing > 0.0, "{hit:?}");
        }
        let sent = sent_to(&server, shooter);
        let kills: Vec<&PlayerKill> = sent
            .iter()
            .filter_map(|message| match message {
                ServerGameMessageKind::KillPlayer(kill) => Some(kill),
                _ => None,
            })
            .collect();
        assert_eq!(kills.len(), 1, "{kills:?}");
        assert_eq!((kills[0].victim_id, kills[0].killer_id), (1, 0));
        // The AK-74u in the dummy loadout, for the kill feed icon.
        assert_eq!(kills[0].item_dict_id, 13);
        let last_hit = sent
            .iter()
            .rposition(|message| matches!(message, ServerGameMessageKind::HitPlayer(..)))
            .unwrap();
        let kill = sent
            .iter()
            .position(|message| matches!(message, ServerGameMessageKind::KillPlayer(..)))
            .unwrap();
        assert!(last_hit < kill, "nothing hits a dead player");
        assert!(!server.peers[&victim].alive);
        assert!(server.peers[&victim].respawn_at.is_some());
    }

    #[test]
    fn nobody_is_hurt_once_time_is_up() {
        let Some(mut server) = combat_server() else {
            return;
        };
        let (shooter, victim) =
            fire_at_a_standing_player(&mut server, DEFAULT_MATCH_ID, 1000, |server, start| {
                let time_up = match_in(
                    server,
                    Phase::TimeUp {
                        ends_at: start + Duration::from_secs(60),
                    },
                    start,
                );
                server.matches.insert(DEFAULT_MATCH_ID, time_up);
            });
        assert_eq!(hits(&server, shooter), []);
        assert!(server.peers[&victim].alive);
    }

    #[test]
    fn bullets_only_hit_players_of_their_match() {
        let Some(mut server) = combat_server() else {
            return;
        };
        let (_, victim) =
            fire_at_a_standing_player(&mut server, DEFAULT_MATCH_ID + 1, 1000, |_, _| {});
        assert_eq!(hits(&server, victim), []);
        assert!(server.peers[&victim].alive);
    }

    #[test]
    fn spawns_carry_the_weapon_seeds_and_the_game_data_loads() {
        let Some(mut server) = combat_server() else {
            return;
        };
        let addr = peer_addr(20_100);
        server.peers.insert(addr, loading_peer(DEFAULT_MATCH_ID, 0));
        server.handle_game_message(addr, ClientGameMessageKind::JoinMatch);

        let seeds = server.peers[&addr].weapon_seeds;
        let spawn = sent_to(&server, addr)
            .into_iter()
            .find_map(|message| match message {
                ServerGameMessageKind::SpawnPlayer { player, .. } => Some(player),
                _ => None,
            })
            .expect("spawned");
        let weapons: Vec<_> = spawn
            .player_inventory
            .iter()
            .filter_map(|slot| match slot {
                survarium::player_input::player_inventory_slot::weapon_slot(weapon) => {
                    Some(*weapon)
                }
                _ => None,
            })
            .collect();
        assert_eq!(weapons.len(), 2);
        for (weapon, (random, normal)) in weapons.iter().zip(seeds) {
            assert_eq!(
                (weapon.random_seed, weapon.normal_random_seed),
                (random, normal as u32)
            );
        }
        // AK-74u, then the Remington 700 with a round chambered from its
        // 5-round magazine.
        assert_eq!(
            weapons
                .iter()
                .map(|weapon| (
                    weapon.ammo_in_magazine,
                    weapon.is_there_chamber_a_round_state
                ))
                .collect::<Vec<_>>(),
            [(30, None), (4, Some(true))]
        );
    }

    #[test]
    fn relays_the_weapon_slot_a_player_switches_to() {
        use crate::combat::weapon::bits;
        let Some(mut server) = combat_server() else {
            return;
        };
        let switching = peer_addr(20_200);
        let watching = peer_addr(20_201);
        for (addr, player_id) in [(switching, 0), (watching, 1)] {
            server
                .peers
                .insert(addr, loading_peer(DEFAULT_MATCH_ID, player_id));
            server.handle_game_message(addr, ClientGameMessageKind::JoinMatch);
        }
        let feet = Vec3::from(map::RESPAWN_POINTS[0].position);
        server.handle_game_message(switching, aim_update(feet, 0.0, 0, 0));
        server.handle_game_message(switching, aim_update(feet, 0.0, bits::WEAPON_2, 16));

        let slots: Vec<u8> = sent_to(&server, watching)
            .into_iter()
            .filter_map(|message| match message {
                ServerGameMessageKind::ServerPlayerInput { weapon_state, .. } => {
                    Some(weapon_state.slot_id)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            slots,
            [
                profile_slot_enum::weapon1_slot as u8,
                profile_slot_enum::weapon2_slot as u8
            ]
        );
    }

    /// The kinds of messages a client got, in order.
    fn kinds_sent_to(server: &MatchServer, addr: SocketAddr) -> Vec<&'static str> {
        sent_to(server, addr)
            .iter()
            .map(|message| match message {
                ServerGameMessageKind::WorldSynchronizationRequest => "world sync",
                ServerGameMessageKind::SpawnPlayer { .. } => "spawn",
                ServerGameMessageKind::ServerPlayerInput { .. } => "input",
                ServerGameMessageKind::InitializeVictoryItems { .. } => "batteries",
                ServerGameMessageKind::GameWorldObjectState { .. } => "trap",
                ServerGameMessageKind::DamageModelState { .. } => "damage model",
                ServerGameMessageKind::KillPlayer(..) => "kill",
                ServerGameMessageKind::PlayerKdStatsChanged { .. } => "kd",
                ServerGameMessageKind::RespawnTimeChanged { .. } => "respawn time",
                ServerGameMessageKind::VictoryItemTakeOrPut { .. } => "battery move",
                _ => "other",
            })
            .collect()
    }

    /// Between asking a client to clear its world and its confirmation,
    /// nothing about the world reaches it but kills; then it gets back every
    /// living player (itself included) and the batteries, and the updates
    /// flow again.
    /// A client that stops reading in a running match (its game froze) is
    /// not sent the moves and timers that piled up, only the events.
    #[test]
    fn a_frozen_client_gets_the_events_and_not_the_old_moves() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let frozen = peer_addr(16_900);
        let other = peer_addr(16_901);
        server
            .peers
            .insert(frozen, joined_peer(DEFAULT_MATCH_ID, 0));
        server.peers.insert(other, joined_peer(DEFAULT_MATCH_ID, 1));
        let feet = float3 {
            x: 1.0,
            y: 0.5,
            z: 2.0,
        };
        for _ in 0..STALL_BACKLOG {
            server.handle_game_message(other, facing_update(feet, 0.5, 0.0));
        }
        server.catch_up_stalled_peers();
        assert_eq!(server.peers[&frozen].transport.queued_len(), STALL_BACKLOG);

        server.send_to(frozen, kd(1, 3, 0));
        server.handle_game_message(other, facing_update(feet, 0.5, 0.0));
        server.catch_up_stalled_peers();
        assert_eq!(server.peers[&frozen].transport.queued_len(), 1);
        assert!(server.peers[&frozen].world_sync_requested.is_none());
        assert_eq!(server.peers[&other].transport.queued_len(), 0);
    }

    #[test]
    fn a_world_resync_holds_the_world_until_the_client_confirms() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let resyncing = peer_addr(16_800);
        let moving = peer_addr(16_801);
        let dying = peer_addr(16_802);
        let now = Instant::now();
        running_battery_match(&mut server, 2, now);
        for (addr, player_id) in [(resyncing, 0), (moving, 1), (dying, 2)] {
            server
                .peers
                .insert(addr, joined_peer(DEFAULT_MATCH_ID, player_id));
        }
        let feet = float3 {
            x: 1.0,
            y: 0.5,
            z: 2.0,
        };

        server.run_console_command(
            Command::Resync {
                match_id: Some(DEFAULT_MATCH_ID),
            },
            now,
        );
        assert_eq!(kinds_sent_to(&server, resyncing), ["world sync"]);
        server.handle_game_message(moving, facing_update(feet, 0.5, 0.0));
        kill(&mut server, dying, 1, now);
        // A confirmation from a client that already confirmed is ignored.
        server.handle_game_message(
            moving,
            ClientGameMessageKind::WorldSynchronizationConfirmation,
        );
        server.handle_game_message(
            moving,
            ClientGameMessageKind::WorldSynchronizationConfirmation,
        );
        server.recorded = Some(Vec::new());
        server.run_console_command(
            Command::Resync {
                match_id: Some(DEFAULT_MATCH_ID),
            },
            now,
        );
        assert_eq!(
            kinds_sent_to(&server, resyncing),
            [] as [&str; 0],
            "already asked"
        );

        server.handle_game_message(
            resyncing,
            ClientGameMessageKind::WorldSynchronizationConfirmation,
        );
        let sent = sent_to(&server, resyncing);
        assert_eq!(
            kinds_sent_to(&server, resyncing),
            ["spawn", "spawn", "batteries"]
        );
        let shown: Vec<u8> = spawns_sent_to(&server, resyncing)
            .iter()
            .map(|(player_id, _)| *player_id)
            .collect();
        assert_eq!(shown, [0, 1], "the dead player stays out");
        let ServerGameMessageKind::SpawnPlayer { player, .. } = &sent[1] else {
            panic!();
        };
        assert_eq!(player.position, feet);

        server.handle_game_message(moving, facing_update(feet, 0.5, 0.0));
        assert_eq!(kinds_sent_to(&server, resyncing).last(), Some(&"input"));
    }

    #[test]
    fn the_messages_held_during_a_resync_are_the_worlds() {
        let mut server = recording_server(Arc::new(SessionStore::new()));
        let resyncing = peer_addr(16_810);
        let other = peer_addr(16_811);
        server
            .peers
            .insert(resyncing, joined_peer(DEFAULT_MATCH_ID, 0));
        server.peers.insert(other, joined_peer(DEFAULT_MATCH_ID, 1));
        let now = Instant::now();
        server.request_world_sync(resyncing, now);
        server.recorded = Some(Vec::new());
        server.handle_game_message(
            other,
            facing_update(map::RESPAWN_POINTS[0].position, 0.0, 0.0),
        );
        kill(&mut server, other, 0, now);
        server.broadcast_to_match(
            DEFAULT_MATCH_ID,
            ServerGameMessageKind::TrapFired {
                player_id: 1,
                slot: 0x0E,
                index: 0,
            },
        );
        server.broadcast_to_match(
            DEFAULT_MATCH_ID,
            ServerGameMessageKind::HitPlayer(player_hit(1, 0)),
        );
        server.broadcast_to_match(DEFAULT_MATCH_ID, kd(1, 2, 3));
        assert_eq!(kinds_sent_to(&server, resyncing), ["kill", "kd"]);
        assert_eq!(
            kinds_sent_to(&server, other),
            ["kill", "respawn time", "other", "other", "kd"]
        );
    }

    /// After a resync its own client restarts the player's weapons from the
    /// spawn it is sent (idle, the magazine as it is, the random generators
    /// where they are): the replay restarts them alike, so shots carry on
    /// from there. Its own damage model comes with every affect.
    #[test]
    fn a_resynced_player_carries_on_from_its_spawn() {
        use crate::combat::damage::affect;
        use crate::combat::weapon::bits;
        use survarium::player_input::player_inventory_slot;
        let Some(mut server) = combat_server() else {
            return;
        };
        let addr = peer_addr(20_500);
        let start = Instant::now();
        wounded_player(&mut server, addr, start);
        let (from, yaw, _) = server.combat.as_ref().unwrap().firing_range(8.0);
        let mut time = 1000;
        let mut frame = |server: &mut MatchServer, mask: u32| {
            time += 16;
            let at = start + Duration::from_millis(u64::from(time));
            server.handle_game_message_at(addr, aim_update(from, yaw, mask, time), at);
            server.tick_combat_at(at);
        };
        for _ in 0..60 {
            frame(&mut server, bits::FIRE);
        }
        server.request_world_sync(addr, start + Duration::from_secs(2));
        server.recorded = Some(Vec::new());
        server.handle_game_message_at(
            addr,
            ClientGameMessageKind::WorldSynchronizationConfirmation,
            start + Duration::from_secs(2),
        );
        let spawns = spawns_sent_to(&server, addr);
        let [(0, own)] = spawns.as_slice() else {
            panic!("{spawns:?}");
        };
        let player_inventory_slot::weapon_slot(ak) = own.player_inventory[0] else {
            panic!();
        };
        let kit = server.combat.as_ref().unwrap().kit(addr).unwrap();
        let ak_now = kit.weapons[0].unwrap();
        assert!(ak.ammo_in_magazine < 30);
        assert_eq!(
            (
                ak.random_seed,
                ak.normal_random_seed as i32,
                ak.ammo_in_magazine
            ),
            (ak_now.seeds.0, ak_now.seeds.1, ak_now.load.0)
        );
        // Its own stamina times stay on its own clock.
        assert_eq!(own.player_stamina, kit.stamina);
        let models = damage_models_sent_to(&server, addr);
        let [(0, parts)] = models.as_slice() else {
            panic!("{models:?}");
        };
        assert!(
            parts[2]
                .affects
                .iter()
                .any(|(a, _)| *a == affect::CONCUSSION)
        );
        assert!(
            parts[5]
                .affects
                .iter()
                .any(|(a, _)| *a == affect::LEG_DAMAGE)
        );

        // Firing on from the restarted weapon takes rounds from what was sent.
        for _ in 0..40 {
            frame(&mut server, 0);
        }
        for _ in 0..60 {
            frame(&mut server, bits::FIRE);
        }
        let after = server.combat.as_ref().unwrap().kit(addr).unwrap().weapons[0]
            .unwrap()
            .load
            .0;
        assert!(
            after < ak.ammo_in_magazine,
            "{after} after {}",
            ak.ammo_in_magazine
        );
    }
}
