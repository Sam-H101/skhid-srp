// Per-peer reliable-UDP transport for the match server.
//
// This holds the sequence-number / ack state for ONE peer and (de)serializes
// packets, but owns no socket and no threads: the `MatchServer` loop owns one
// shared `UdpSocket`, demultiplexes datagrams by source address, and drives a
// `Transport` per peer. The reliable-UDP math (`update_acknowledgements`) is
// unchanged from the original single-peer implementation.
//
// Game messages are queued and packed into datagrams once per server frame,
// and resent until the client acknowledges them, as the game's own
// `udp_match_connection` (the server's and the client's) does. Unlike it, at
// most `MAX_UNREPORTED` datagrams go out before the client reports them: a
// client whose game stalls stops reporting, and resending everything to it
// every `RESEND_AFTER` (most of it beyond what its reports can acknowledge)
// overran its packet pool once it ran again.

use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use vostok::network_packet::{Packet, UdpPacket};
use vostok::serde::{Deserialize, Serialize};

use crate::message;
use crate::message::raw::low_level_message_type_enum;
use crate::sequence_number::SN16;
use crate::utils;

#[derive(Copy, Clone, PartialEq)]
pub enum ConnectionState {
    WaitingForConnection,
    Connected,
    Disconnected,
}

pub struct Transport {
    // Last sequence_id sent to the client
    server_sequence_id: SN16,
    server_received_sequence_id: SN16,
    server_received_ack_bits: u16,

    // Last sequence_id recv from the client
    client_sequence_id: SN16,
    client_ack_bits: u16,

    server_order_id: SN16,
    /// The newest order id sent so far: packing can leave older messages
    /// waiting behind it, which keep their ids (`drop_queued`).
    newest_sent: u16,

    /// The newest client message delivered (`channel::received_order_id`).
    /// Every match message is ordered and reliable on channel 0
    /// (`network_packets_orderer`); the client packs its frames newest first
    /// and expects them handled in order, each once.
    client_order_id: SN16,
    /// Client messages that arrived ahead of a missing one, by order id.
    early_messages: HashMap<u16, message::ClientGameMessageKind>,

    /// Game messages not sent yet, oldest first: queued for the next `flush`,
    /// or held back while the client has `MAX_UNREPORTED` datagrams to report.
    queued: Vec<Outgoing>,
    /// Every datagram sent that the client can still acknowledge, in sequence
    /// order (consecutive ids): for its messages and to time the round trip.
    in_flight: VecDeque<Datagram>,
    /// Game messages sent and not yet acknowledged, by order id: the client
    /// waits for every one of them, in order.
    unacknowledged: HashMap<u16, Pending>,
    stats: Stats,
    last_report: Instant,

    last_send_time: Instant,

    connection_state: ConnectionState,

    /// The newest packet the client has acknowledged, as timed.
    newest_timed_ack: Option<SN16>,
    /// Recent round trips; the smallest is the path's latency (the client
    /// acknowledges on its next send, up to a frame batch later).
    round_trips: VecDeque<Duration>,
    /// Test switch: the percentage of datagrams to this peer dropped instead
    /// of sent, and the generator that picks them.
    simulated_loss: Option<(u64, u64)>,
}

/// A game message waiting for a datagram, with its size on the wire.
#[derive(Clone)]
struct Outgoing {
    message: message::ServerGameMessage,
    size: usize,
}

/// A datagram the client can still acknowledge.
struct Datagram {
    sequence_id: SN16,
    sent: Instant,
    /// The order ids of the game messages it carried.
    order_ids: Vec<u16>,
    acknowledged: bool,
}

/// A game message the client has not acknowledged.
struct Pending {
    outgoing: Outgoing,
    /// When it last went out.
    sent: Instant,
}

/// What a peer's datagrams did since the last report.
#[derive(Default)]
struct Stats {
    datagrams: u32,
    unacknowledged: u32,
    resent: u32,
}

const ROUND_TRIP_SAMPLES: usize = 32;
/// `udp_match_packet` holds 256 bytes, the 6-byte header included, and the
/// client reads into a 256-byte buffer: a longer datagram fails the read and
/// disconnects it.
const MAX_DATAGRAM_SIZE: usize = 256;
const MAX_MESSAGE_SIZE: usize = MAX_DATAGRAM_SIZE - 6;
/// The client's ack bits cover the newest packet and the 15 before it.
const ACK_BITS: u16 = 16;
/// Datagrams remembered while the client acknowledges none (well inside half
/// the sequence space, so their ids still compare).
const MAX_IN_FLIGHT: usize = 16_384;
/// `max_packet_wait_time_in_ms` of the game's connections (without the
/// network flow emulator): a message unacknowledged this long after it went
/// out is sent again.
const RESEND_AFTER: Duration = Duration::from_millis(500);
/// Datagrams sent that the client has not reported yet. Its next report names
/// its newest and the 15 before it, so a 17th sent in between could never be
/// acknowledged, and its messages would be resent for nothing. A datagram
/// unreported for `RESEND_AFTER` counts as lost and no longer holds a place:
/// a client that has stopped reporting (its game stalled) gets this many
/// datagrams every `RESEND_AFTER`, resends first, and little that is new.
/// Every datagram it receives takes a packet from its pool of 8192
/// (`match_client_impl`, 0x776251), which its own sends share; it crashes
/// when the pool runs out (`single_size_buffer_allocator - out of memory`).
const MAX_UNREPORTED: usize = ACK_BITS as usize;
/// Messages waiting for the client, sent or not: past this many it has not
/// been reading for too long, and the peer is dropped. Half the order id
/// range, so the client still tells their order apart.
const MAX_BACKLOG: usize = 0x4000;
/// How often each peer's datagram counts are logged.
const REPORT_INTERVAL: Duration = Duration::from_secs(10);
/// Messages held behind a missing one before giving up on it (seconds of
/// frames): the client resends it until acknowledged, so this is only a guard
/// against a peer that never does.
const MAX_EARLY_MESSAGES: usize = 1024;

/// Whether order id `a` comes after `b` (within half the range).
fn is_newer(a: u16, b: u16) -> bool {
    (1..0x8000).contains(&a.wrapping_sub(b))
}

impl Transport {
    pub fn new() -> Self {
        Self {
            server_sequence_id: 0xFFFF.into(),
            server_received_sequence_id: 0xFFFF.into(),
            server_received_ack_bits: 0b0000_0000_0000_0000,

            client_sequence_id: 0xFFFF.into(),
            client_ack_bits: 0b0000_0000_0000_0000,

            server_order_id: 0xFFFF.into(),
            newest_sent: 0xFFFF,

            client_order_id: 0xFFFF.into(),
            early_messages: HashMap::new(),

            queued: Vec::new(),
            in_flight: VecDeque::new(),
            unacknowledged: HashMap::new(),
            stats: Stats::default(),
            last_report: Instant::now(),
            last_send_time: Instant::now(),
            connection_state: ConnectionState::WaitingForConnection,
            newest_timed_ack: None,
            round_trips: VecDeque::new(),
            simulated_loss: None,
        }
    }

    /// Drop this percentage of the datagrams to this peer, to try the
    /// retransmission on one machine.
    pub fn simulate_loss(&mut self, percent: u64, seed: u64) {
        self.simulated_loss = (percent > 0).then_some((percent, seed.max(1)));
    }

    fn simulate_a_loss(&mut self) -> bool {
        let Some((percent, state)) = &mut self.simulated_loss else {
            return false;
        };
        // xorshift64
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state % 100 < *percent
    }

    /// The connection's round trip: the smallest of the recent samples.
    pub fn round_trip(&self) -> Option<Duration> {
        self.round_trips.iter().min().copied()
    }

    fn time_acknowledgement(&mut self, acked: SN16, now: Instant) {
        if self.newest_timed_ack.is_some_and(|newest| acked <= newest) {
            return;
        }
        self.newest_timed_ack = Some(acked);
        if let Some(sent) = self.in_flight(acked).map(|datagram| datagram.sent) {
            if self.round_trips.len() == ROUND_TRIP_SAMPLES {
                self.round_trips.pop_front();
            }
            self.round_trips.push_back(now.duration_since(sent));
        }
    }

    fn in_flight(&mut self, sequence_id: SN16) -> Option<&mut Datagram> {
        let first = self.in_flight.front()?.sequence_id;
        let index = usize::from(sequence_id.0.wrapping_sub(first.0));
        self.in_flight
            .get_mut(index)
            .filter(|datagram| datagram.sequence_id == sequence_id)
    }

    pub fn is_disconnected(&self) -> bool {
        self.connection_state == ConnectionState::Disconnected
    }

    /// Decode one incoming datagram: update acknowledgements, advance the
    /// connection state machine, send any transport-level reply
    /// (`ConnectionSuccessful`, disconnect confirmation) and return the decoded
    /// client game messages for the relay to act on.
    pub fn handle_datagram(
        &mut self,
        datagram: &[u8],
        socket: &UdpSocket,
        addr: SocketAddr,
    ) -> Vec<message::ClientGameMessageKind> {
        let mut buffer = datagram;

        let packet = match message::ClientMessage::deserialize(&mut buffer) {
            Ok(packet) => packet,
            Err(error) => {
                log::warn!(
                    "unreadable client packet from {addr} ({error}): {}",
                    message::client_message::hex(datagram)
                );
                utils::print_debug(datagram);
                return vec![];
            }
        };

        packet.print_debug();

        let message::ClientMessage {
            local_sequence_id,
            remote_sequence_id,
            remote_ack_bits,
            kind,
        } = packet;

        // Packet already received. Ignore
        if local_sequence_id == self.client_sequence_id {
            return vec![];
        }

        // `client_sequence_id` is the newest packet already processed. As in
        // `udp_match_connection::process_incoming_packet`, only a newer packet
        // moves the acknowledgements; a late one (UDP order 0 -> 2 -> 1) still
        // delivers its messages, and their order ids keep any already handled
        // from running twice.
        if self.client_sequence_id < local_sequence_id {
            self.update_acknowledgements(local_sequence_id, remote_sequence_id, remote_ack_bits);
            self.time_acknowledgement(remote_sequence_id, Instant::now());
            self.retire_unacknowledgeable();
        }

        match self.connection_state {
            ConnectionState::WaitingForConnection => {
                let valid_header = local_sequence_id == 0x0000.into()
                    && remote_sequence_id == 0xFFFF.into()
                    && remote_ack_bits == 0b1000_0000_0000_0000;

                let mut messages = match kind {
                    message::ClientMessageKind::Messages(messages) => messages.into_iter(),
                    message::ClientMessageKind::Low(_) => Vec::new().into_iter(),
                };
                let first = messages.next();

                // A malformed first packet must not crash the server (which would
                // restart the whole match and drop every peer): log and drop this
                // would-be peer instead.
                let Some(message::ClientGameMessage {
                    order_id: SN16(0),
                    game_message:
                        game_message @ message::ClientGameMessageKind::ConnectionRequest { .. },
                }) = first.filter(|_| valid_header)
                else {
                    log::warn!("dropping malformed connection attempt from {addr}");
                    self.connection_state = ConnectionState::Disconnected;
                    return vec![];
                };

                self.connection_state = ConnectionState::Connected;
                self.client_order_id = SN16(0);
                // Transport-level: acknowledge the connection. The relay learns of
                // the new peer from the returned `ConnectionRequest`.
                self.send_game_message(message::ServerGameMessageKind::ConnectionSuccessful);

                let mut delivered = vec![game_message];
                delivered.extend(self.deliver_in_order(messages.collect()));
                delivered
            }
            _ => match kind {
                message::ClientMessageKind::Low(msg_type) => {
                    self.handle_low_level_message(msg_type, socket, addr);
                    vec![]
                }
                message::ClientMessageKind::Messages(messages) => self.deliver_in_order(messages),
            },
        }
    }

    /// `udp_match_connection::call_predicate`: drop a message already handled
    /// or already held, hold it, then hand on the run that follows the last
    /// one handled, in order.
    fn deliver_in_order(
        &mut self,
        messages: Vec<message::ClientGameMessage>,
    ) -> Vec<message::ClientGameMessageKind> {
        for message in messages {
            if message.order_id <= self.client_order_id
                || self.early_messages.contains_key(&message.order_id.0)
            {
                continue;
            }
            self.early_messages
                .insert(message.order_id.0, message.game_message);
        }
        if self.early_messages.len() > MAX_EARLY_MESSAGES {
            let newest = self.client_order_id.0;
            if let Some(&oldest) = self
                .early_messages
                .keys()
                .min_by_key(|order_id| order_id.wrapping_sub(newest))
            {
                log::warn!(
                    "client messages {} to {} never arrived; skipping them",
                    newest.wrapping_add(1),
                    oldest.wrapping_sub(1)
                );
                self.client_order_id = SN16(oldest.wrapping_sub(1));
            }
        }
        let mut delivered = Vec::new();
        loop {
            let next = self.client_order_id.0.wrapping_add(1);
            let Some(message) = self.early_messages.remove(&next) else {
                break;
            };
            delivered.push(message);
            self.client_order_id = SN16(next);
        }
        delivered
    }

    fn handle_low_level_message(
        &mut self,
        msg_type: low_level_message_type_enum,
        socket: &UdpSocket,
        addr: SocketAddr,
    ) {
        match msg_type {
            low_level_message_type_enum::continuous_flow => (),
            low_level_message_type_enum::initiate_disconnection => {
                self.send_packet(
                    socket,
                    addr,
                    message::ServerMessageKind::Low(
                        low_level_message_type_enum::confirm_disconnection,
                    ),
                    Vec::new(),
                    Instant::now(),
                );
                // Don't panic the whole server: mark this peer disconnected so the
                // loop can drop it and notify the others.
                self.connection_state = ConnectionState::Disconnected;
            }
            // We never initiate a disconnect, so a confirmation is unexpected;
            // ignore it rather than crash on a stray/malicious packet.
            low_level_message_type_enum::confirm_disconnection => {
                log::debug!("ignoring unexpected confirm_disconnection");
            }
        }
    }

    /// Send a keepalive if we've been idle, so the client's channel stays alive.
    /// Not while the client has `MAX_UNREPORTED` datagrams to report: it has
    /// heard from us, and a keepalive would take one's place.
    pub fn maintain(&mut self, socket: &UdpSocket, addr: SocketAddr) {
        const MAX_IDLE_TIME_IN_MS: u64 = 0x21; // Copied from the game
        let now = Instant::now();
        if now - self.last_send_time > Duration::from_millis(MAX_IDLE_TIME_IN_MS)
            && self.send_budget(now) > 0
        {
            self.send_packet(
                socket,
                addr,
                message::ServerMessageKind::Low(low_level_message_type_enum::continuous_flow),
                Vec::new(),
                Instant::now(),
            );
        }
    }

    /// Queue a game message for the next `flush`, under the next order id.
    pub fn send_game_message(&mut self, game_message: message::ServerGameMessageKind) {
        let message = message::ServerGameMessage {
            order_id: self.server_order_id + 1.into(),
            game_message,
        };
        let mut packet = UdpPacket::new();
        message.clone().serialize(&mut packet);
        let size = packet.cursor_index();
        if size > MAX_MESSAGE_SIZE {
            // The client could not take it, and an order id it never gets
            // would hold back every message after it.
            log::error!(
                "not sending a {size}-byte message: {:?}",
                message.game_message
            );
            return;
        }
        self.server_order_id = message.order_id;
        self.queued.push(Outgoing { message, size });
    }

    /// Game messages queued and not sent yet.
    pub fn queued_len(&self) -> usize {
        self.queued.len()
    }

    /// Throw away the queued messages newer than any sent that `stale`
    /// picks, and give the rest of those the order ids that follow the
    /// newest sent, so the client never waits for an id that will not come.
    /// Older ones still waiting (packing left them behind) fill gaps the
    /// client already waits on, and stay. Returns how many went.
    pub fn drop_queued(
        &mut self,
        stale: impl Fn(&message::ServerGameMessageKind) -> bool,
    ) -> usize {
        let newest_sent = self.newest_sent;
        let before = self.queued.len();
        self.queued.retain(|outgoing| {
            !is_newer(outgoing.message.order_id.0, newest_sent)
                || !stale(&outgoing.message.game_message)
        });
        // Oldest first (`send_oldest`), so the new ids keep their order.
        let mut next = newest_sent.wrapping_add(1);
        for outgoing in &mut self.queued {
            if is_newer(outgoing.message.order_id.0, newest_sent) {
                outgoing.message.order_id = SN16(next);
                next = next.wrapping_add(1);
            }
        }
        self.server_order_id = SN16(next.wrapping_sub(1));
        before - self.queued.len()
    }

    /// How many datagrams may go out now (see `MAX_UNREPORTED`).
    fn send_budget(&self, now: Instant) -> usize {
        let reported = self.server_received_sequence_id;
        let unreported = self
            .in_flight
            .iter()
            .rev()
            .take_while(|datagram| datagram.sequence_id > reported)
            .filter(|datagram| now.duration_since(datagram.sent) < RESEND_AFTER)
            .count();
        MAX_UNREPORTED.saturating_sub(unreported)
    }

    /// Send the messages unacknowledged for `RESEND_AFTER`, then the queued
    /// ones, oldest first, in as few datagrams as they fit, as many datagrams
    /// as `send_budget` allows. The rest wait for the next `flush`.
    pub fn flush(&mut self, socket: &UdpSocket, addr: SocketAddr, now: Instant) {
        let budget = self.send_budget(now);
        if budget > 0 {
            self.send_oldest(budget, socket, addr, now);
        }
        let waiting = self.queued.len() + self.unacknowledged.len();
        if waiting > MAX_BACKLOG {
            log::warn!("{addr}: {waiting} messages waiting; dropping the peer");
            self.connection_state = ConnectionState::Disconnected;
        }
        self.report(addr, now);
    }

    fn send_oldest(&mut self, budget: usize, socket: &UdpSocket, addr: SocketAddr, now: Instant) {
        // Oldest first: order ids count back from the newest.
        let newest = self.server_order_id.0;
        let age = |outgoing: &Outgoing| newest.wrapping_sub(outgoing.message.order_id.0);
        let mut due: Vec<Outgoing> = self
            .unacknowledged
            .values()
            .filter(|pending| now.duration_since(pending.sent) >= RESEND_AFTER)
            .map(|pending| pending.outgoing.clone())
            .collect();
        due.sort_by_key(|outgoing| std::cmp::Reverse(age(outgoing)));
        let mut candidates = due;
        candidates.append(&mut self.queued);

        // What `budget` full datagrams carry (a lone message has no length
        // byte); packing may leave some over.
        let mut room = budget * (MAX_MESSAGE_SIZE + 1);
        let fitting = candidates
            .iter()
            .take_while(|outgoing| match room.checked_sub(outgoing.size + 1) {
                Some(left) => {
                    room = left;
                    true
                }
                None => false,
            })
            .count();
        let mut held: Vec<Outgoing> = candidates.split_off(fitting);
        let mut datagrams = pack(candidates);
        for datagram in datagrams.split_off(budget.min(datagrams.len())) {
            held.extend(datagram);
        }
        // Resends held back stay due; new messages wait, oldest first.
        held.retain(|outgoing| {
            !self
                .unacknowledged
                .contains_key(&outgoing.message.order_id.0)
        });
        held.sort_by_key(|outgoing| std::cmp::Reverse(age(outgoing)));
        self.queued = held;

        for datagram in datagrams {
            self.stats.resent += datagram
                .iter()
                .filter(|outgoing| {
                    self.unacknowledged
                        .contains_key(&outgoing.message.order_id.0)
                })
                .count() as u32;
            let order_ids = datagram
                .iter()
                .map(|outgoing| outgoing.message.order_id.0)
                .collect();
            for outgoing in &datagram {
                if is_newer(outgoing.message.order_id.0, self.newest_sent) {
                    self.newest_sent = outgoing.message.order_id.0;
                }
                self.unacknowledged.insert(
                    outgoing.message.order_id.0,
                    Pending {
                        outgoing: outgoing.clone(),
                        sent: now,
                    },
                );
            }
            let messages = datagram
                .into_iter()
                .map(|outgoing| outgoing.message)
                .collect();
            self.send_packet(
                socket,
                addr,
                message::ServerMessageKind::Messages(messages),
                order_ids,
                now,
            );
        }
    }

    fn report(&mut self, addr: SocketAddr, now: Instant) {
        if now.duration_since(self.last_report) < REPORT_INTERVAL {
            return;
        }
        self.last_report = now;
        let stats = std::mem::take(&mut self.stats);
        if stats.datagrams == 0 {
            return;
        }
        let waiting = self.queued.len();
        let level = if stats.unacknowledged > 0 || stats.resent > 0 || waiting > 0 {
            log::Level::Info
        } else {
            log::Level::Debug
        };
        log::log!(
            level,
            "{addr}: {} datagrams sent in {REPORT_INTERVAL:?}; {} left the ack window \
             unacknowledged, {} messages resent, {waiting} waiting to be sent",
            stats.datagrams,
            stats.unacknowledged,
            stats.resent
        );
    }

    fn send_packet(
        &mut self,
        socket: &UdpSocket,
        addr: SocketAddr,
        kind: message::ServerMessageKind,
        order_ids: Vec<u16>,
        now: Instant,
    ) {
        self.server_sequence_id += 1.into();
        self.last_send_time = now;
        if self.in_flight.len() == MAX_IN_FLIGHT {
            self.retire_oldest();
        }
        self.in_flight.push_back(Datagram {
            sequence_id: self.server_sequence_id,
            sent: now,
            order_ids,
            acknowledged: false,
        });
        self.stats.datagrams += 1;

        let mut packet = UdpPacket::new();
        let message = message::ServerMessage {
            remote_sequence_id: self.server_sequence_id,
            local_sequence_id: self.client_sequence_id,
            local_ack_bits: self.client_ack_bits,
            kind,
        };

        message.print_debug();
        message.serialize(&mut packet);

        if self.simulate_a_loss() {
            return;
        }
        // Best-effort send; a lost datagram's messages are resent.
        let _ = socket.send_to(packet.get_message(), addr);
    }

    /// Forget the datagrams the client can no longer acknowledge: its ack bits
    /// reach 15 behind the newest it has acknowledged.
    fn retire_unacknowledgeable(&mut self) {
        let newest = self.server_received_sequence_id.0;
        while let Some(oldest) = self.in_flight.front() {
            let behind = newest.wrapping_sub(oldest.sequence_id.0);
            if !(ACK_BITS..0x8000).contains(&behind) {
                break;
            }
            self.retire_oldest();
        }
    }

    fn retire_oldest(&mut self) {
        if let Some(datagram) = self.in_flight.pop_front()
            && !datagram.acknowledged
        {
            self.stats.unacknowledged += 1;
        }
    }

    /// The client acknowledged a datagram: it has every message in it.
    fn forget_packet(&mut self, sequence_id: SN16) {
        let Some(datagram) = self.in_flight(sequence_id) else {
            return;
        };
        datagram.acknowledged = true;
        for order_id in std::mem::take(&mut datagram.order_ids) {
            self.unacknowledged.remove(&order_id);
        }
    }

    pub fn update_acknowledgements(
        &mut self,
        client_sequence_id: SN16, // local here means client
        server_sequence_id: SN16,
        server_ack_bits: u16, // server packets which were acknowledged by the client
    ) {
        //
        // Updates info on the client packages received
        //
        let diff = client_sequence_id - self.client_sequence_id;

        let mut client_ack_bits = 0;
        if diff < 0x10 {
            client_ack_bits = self.client_ack_bits >> diff;
        }
        client_ack_bits |= 0x8000;

        self.client_ack_bits = client_ack_bits;
        self.client_sequence_id = client_sequence_id;

        //
        // Given what client knows about our packets,
        // remove already received unacknowledged packets
        //

        if server_sequence_id > self.server_sequence_id {
            // The client acked a packet we never sent (malformed/stale): ignore
            // the server-side ack bookkeeping instead of crashing.
            log::warn!("client acked an unsent packet; ignoring");
            return;
        }

        if server_sequence_id == self.server_received_sequence_id {
            // The packet was duplicated. Ignore
            return;
        }

        if server_sequence_id < self.server_received_sequence_id {
            // Packet came out of order, and we already handled a more recent packet.
            // Simply mark this packet as acknowledged.

            let diff = self.server_received_sequence_id - server_sequence_id;
            if diff <= 15 {
                self.server_received_ack_bits |= 1 << (15 - diff);
            }

            self.forget_packet(server_sequence_id);
        } else {
            // This is the most recent state about the server from the client

            let diff = server_sequence_id - self.server_received_sequence_id;

            let last_server_received_ack_bits = if diff < 16 {
                self.server_received_ack_bits >> diff
            } else {
                0
            };

            if server_ack_bits & last_server_received_ack_bits != last_server_received_ack_bits {
                // Packet is inconsistent with what the client told us previously
                // Some packet which was told that it was acknowledged is no longer acknowledged
                return;
            }

            self.server_received_ack_bits = server_ack_bits;
            self.server_received_sequence_id = server_sequence_id;

            let mut acknowledgment_bits = server_ack_bits ^ last_server_received_ack_bits;
            let mut sequence_id = server_sequence_id.0;

            while acknowledgment_bits != 0 {
                if (acknowledgment_bits & 0x8000) != 0 {
                    self.forget_packet(sequence_id.into());
                }
                sequence_id = sequence_id.wrapping_sub(1);
                acknowledgment_bits <<= 1;
            }
        }
    }
}

/// `udp_match_connection::send_queued_packets`: largest first, each datagram
/// filled with the largest that still fit. A lone message goes in the single
/// format, which the serializer picks for one message.
fn pack(mut messages: Vec<Outgoing>) -> Vec<Vec<Outgoing>> {
    messages.sort_by_key(|outgoing| outgoing.size);
    let mut datagrams = Vec::new();
    while let Some(first) = messages.pop() {
        // Every message of a multiple-format datagram has a length byte.
        let mut size_left = MAX_MESSAGE_SIZE as isize - first.size as isize - 1;
        let mut datagram = vec![first];
        let mut index = messages.len();
        while size_left > 1 && index > 0 {
            index -= 1;
            if size_left > messages[index].size as isize {
                size_left -= messages[index].size as isize + 1;
                datagram.push(messages.remove(index));
            }
        }
        datagrams.push(datagram);
    }
    datagrams
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection_packet() -> Vec<u8> {
        vec![
            0x00, 0x00, // local sequence
            0xFF, 0xFF, // remote sequence
            0x00, 0x00, // single packet / ack bits
            0x40, 0x00, 0x00, // connection request / order ID
            0x00, 0xDD, 0x00, 0x00, // session ID
        ]
    }

    fn game_packet(local_sequence: u16, message_type: u8, order_id: u16) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend(local_sequence.to_le_bytes());
        packet.extend(0u16.to_le_bytes());
        packet.extend(0u16.to_le_bytes());
        packet.push(message_type);
        packet.extend(order_id.to_le_bytes());
        packet
    }

    /// One packet of client frames as the client packs them: newest first.
    fn update_batch(local_sequence: u16, frames: &[(u16, u32)]) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend(local_sequence.to_le_bytes());
        packet.extend(0u16.to_le_bytes());
        packet.extend(1u16.to_le_bytes()); // multiple packets
        for &(order_id, time_in_ms) in frames {
            packet.push(47);
            packet.push(0x43); // client_player_update
            packet.extend(order_id.to_le_bytes());
            packet.extend([0; 40]); // input and state
            packet.extend(time_in_ms.to_le_bytes());
        }
        packet
    }

    fn connected() -> (Transport, UdpSocket, SocketAddr) {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let peer_addr = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let mut transport = Transport::new();
        assert_eq!(
            transport
                .handle_datagram(&connection_packet(), &socket, peer_addr)
                .len(),
            1
        );
        (transport, socket, peer_addr)
    }

    fn times(messages: &[message::ClientGameMessageKind]) -> Vec<u32> {
        messages
            .iter()
            .map(|message| match message {
                message::ClientGameMessageKind::ClientPlayerUpdate { time_in_ms, .. } => {
                    *time_in_ms
                }
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    #[test]
    fn delivers_a_late_packet_in_order_and_only_once() {
        let (mut transport, socket, peer_addr) = connected();

        // Order 2 arrives first and waits for order 1 (UDP order 0 -> 2 -> 1).
        assert!(
            transport
                .handle_datagram(&game_packet(2, 0x42, 2), &socket, peer_addr)
                .is_empty()
        );
        assert_eq!(
            transport.handle_datagram(&game_packet(1, 0x41, 1), &socket, peer_addr),
            [
                message::ClientGameMessageKind::GetStartupInfo,
                message::ClientGameMessageKind::JoinMatch
            ]
        );
        // The client's resend of order 1 is a duplicate.
        assert!(
            transport
                .handle_datagram(&game_packet(3, 0x41, 1), &socket, peer_addr)
                .is_empty()
        );
    }

    #[test]
    fn delivers_frames_packed_newest_first_in_their_order() {
        let (mut transport, socket, peer_addr) = connected();

        let first = transport.handle_datagram(
            &update_batch(1, &[(3, 30), (2, 20), (1, 10)]),
            &socket,
            peer_addr,
        );
        assert_eq!(times(&first), [10, 20, 30]);
        // Across the order id wrap, with a resend of the newest frame handled.
        transport.client_order_id = SN16(0xFFFE);
        let wrapped = update_batch(2, &[(1, 70), (0, 60), (0xFFFF, 55), (0xFFFE, 0)]);
        assert_eq!(
            times(&transport.handle_datagram(&wrapped, &socket, peer_addr)),
            [55, 60, 70]
        );
    }

    #[test]
    fn an_unread_message_holds_its_place_in_the_order() {
        let (mut transport, socket, peer_addr) = connected();

        // order 2 is a bullets info request the relay doesn't read.
        let mut packet = update_batch(1, &[(3, 30)]);
        packet.extend([5, 0x47, 2, 0, 7, 7]);
        packet.extend(&update_batch(0, &[(1, 10)])[6..]);
        let delivered = transport.handle_datagram(&packet, &socket, peer_addr);
        assert_eq!(delivered.len(), 3);
        assert_eq!(
            delivered[1],
            message::ClientGameMessageKind::Unhandled(
                message::client_message::raw::match_client_message_types_enum::bullets_info_request
            )
        );
        assert_eq!(
            times(&[delivered[0].clone(), delivered[2].clone()]),
            [10, 30]
        );
    }

    #[test]
    fn gives_up_on_a_message_that_never_comes() {
        let (mut transport, socket, peer_addr) = connected();

        // Order 1 is lost for good; everything after it waits, up to a limit.
        let mut delivered = Vec::new();
        for order_id in 2..=(MAX_EARLY_MESSAGES as u16 + 2) {
            // One packet per frame, numbered after the connection's.
            delivered.extend(transport.handle_datagram(
                &update_batch(order_id - 1, &[(order_id, u32::from(order_id))]),
                &socket,
                peer_addr,
            ));
        }
        assert_eq!(
            times(&delivered),
            (2..=(MAX_EARLY_MESSAGES as u32 + 2)).collect::<Vec<_>>()
        );
    }

    fn keepalives(transport: &mut Transport, socket: &UdpSocket, to: SocketAddr, count: usize) {
        for _ in 0..count {
            transport.send_packet(
                socket,
                to,
                message::ServerMessageKind::Low(low_level_message_type_enum::continuous_flow),
                Vec::new(),
                Instant::now(),
            );
        }
    }

    #[test]
    fn times_an_ack_of_a_packet_far_back_in_a_busy_stream() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut transport = Transport::new();
        keepalives(&mut transport, &socket, peer.local_addr().unwrap(), 200);
        // A busy match: the client acknowledges a packet 90 sends back.
        let acked: SN16 = transport.server_sequence_id.0.wrapping_sub(90).into();
        transport.time_acknowledgement(acked, Instant::now());
        assert!(transport.round_trip().is_some());
    }

    #[test]
    fn remembers_a_bounded_number_of_unacknowledged_datagrams() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut transport = Transport::new();
        // Past a sequence number wrap.
        keepalives(&mut transport, &socket, peer.local_addr().unwrap(), 70_000);
        assert_eq!(transport.in_flight.len(), MAX_IN_FLIGHT);
        assert_eq!(
            transport.stats.unacknowledged as usize,
            70_000 - MAX_IN_FLIGHT
        );
        let newest = transport.server_sequence_id;
        transport.forget_packet(newest);
        assert!(transport.in_flight.back().unwrap().acknowledged);
    }

    /// The handshake from a client socket that stays open to read the replies.
    fn connected_to(client: &UdpSocket) -> (Transport, UdpSocket, SocketAddr) {
        client
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let client_addr = client.local_addr().unwrap();
        let mut transport = Transport::new();
        transport.handle_datagram(&connection_packet(), &socket, client_addr);
        (transport, socket, client_addr)
    }

    /// A client keepalive acknowledging `acked` and the packets its bits name
    /// (bit 15 is `acked` itself).
    fn ack_packet(local_sequence: u16, acked: u16, bits: u16) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend(local_sequence.to_le_bytes());
        packet.extend(acked.to_le_bytes());
        packet.extend((bits << 1 | 1).to_le_bytes());
        packet.extend([1, 2]); // continuous flow
        packet
    }

    /// One server datagram as the client reads it: its sequence id and the
    /// order id of every game message in it.
    fn read_datagram(client: &UdpSocket) -> (u16, Vec<u16>) {
        let mut buffer = [0; 2048];
        let (len, _) = client.recv_from(&mut buffer).unwrap();
        assert!(len <= MAX_DATAGRAM_SIZE, "a {len}-byte datagram");
        let datagram = &buffer[..len];
        let sequence_id = u16::from_le_bytes([datagram[0], datagram[1]]);
        let order_id = |message: &[u8]| u16::from_le_bytes([message[1], message[2]]);
        if datagram[4] & 1 == 0 {
            return (sequence_id, vec![order_id(&datagram[6..])]);
        }
        let mut messages = &datagram[6..];
        let mut order_ids = Vec::new();
        while let Some((&size, rest)) = messages.split_first() {
            let (message, rest) = rest.split_at(usize::from(size));
            order_ids.push(order_id(message));
            messages = rest;
        }
        // The client reads a lone message in this format as a low-level one.
        assert!(order_ids.len() > 1);
        (sequence_id, order_ids)
    }

    fn kd_stats(player_id: u8) -> message::ServerGameMessageKind {
        message::ServerGameMessageKind::PlayerKdStatsChanged {
            player_id,
            kills: 1,
            deaths: 2,
        }
    }

    #[test]
    fn packs_a_frame_of_messages_into_datagrams_the_client_can_read() {
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (mut transport, socket, client_addr) = connected_to(&client);
        for player_id in 0..30 {
            transport.send_game_message(kd_stats(player_id));
        }
        // The connection reply (order 0) and 30 13-byte messages.
        transport.flush(&socket, client_addr, Instant::now());
        let mut order_ids = Vec::new();
        for _ in 0..2 {
            order_ids.extend(read_datagram(&client).1);
        }
        order_ids.sort();
        assert_eq!(order_ids, (0..=30).collect::<Vec<_>>());

        // A lone message goes in the single format.
        transport.send_game_message(kd_stats(0));
        transport.flush(&socket, client_addr, Instant::now());
        assert_eq!(read_datagram(&client).1, [31]);
    }

    #[test]
    fn resends_an_unacknowledged_message_in_a_new_datagram() {
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (mut transport, socket, client_addr) = connected_to(&client);
        transport.send_game_message(kd_stats(0));
        let start = Instant::now();
        transport.flush(&socket, client_addr, start);
        let (first, mut order_ids) = read_datagram(&client);
        order_ids.sort();
        assert_eq!(order_ids, [0, 1]);

        transport.flush(&socket, client_addr, start + RESEND_AFTER / 2);
        assert_eq!(transport.server_sequence_id, first.into());
        transport.flush(&socket, client_addr, start + RESEND_AFTER);
        let (resend, mut order_ids) = read_datagram(&client);
        order_ids.sort();
        assert_eq!((resend, order_ids), (first + 1, vec![0, 1]));
        assert_eq!(transport.stats.resent, 2);
    }

    #[test]
    fn does_not_resend_what_the_client_acknowledged() {
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (mut transport, socket, client_addr) = connected_to(&client);
        let start = Instant::now();
        transport.flush(&socket, client_addr, start);
        let (first, _) = read_datagram(&client);
        transport.handle_datagram(&ack_packet(1, first, 0x8000), &socket, client_addr);
        assert!(transport.unacknowledged.is_empty());
        transport.flush(&socket, client_addr, start + RESEND_AFTER * 2);
        assert_eq!(transport.server_sequence_id, first.into());
    }

    #[test]
    fn an_acknowledged_earlier_send_releases_a_resent_message() {
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (mut transport, socket, client_addr) = connected_to(&client);
        let start = Instant::now();
        transport.flush(&socket, client_addr, start);
        let (first, _) = read_datagram(&client);
        transport.flush(&socket, client_addr, start + RESEND_AFTER);
        read_datagram(&client);
        // The first send arrived after all; its resend is the client's to drop.
        transport.handle_datagram(&ack_packet(1, first, 0x8000), &socket, client_addr);
        assert!(transport.unacknowledged.is_empty());
    }

    #[test]
    fn a_client_gets_every_message_through_a_lossy_link() {
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (mut transport, socket, client_addr) = connected_to(&client);
        transport.simulate_loss(30, 7);
        client.set_nonblocking(true).unwrap();
        let start = Instant::now();
        let mut received = std::collections::BTreeSet::new();
        // The client's acknowledgement state.
        let (mut newest, mut bits) = (0xFFFF_u16, 0_u16);
        for step in 0..1000_u16 {
            if step < 300 {
                transport.send_game_message(kd_stats(0));
            }
            transport.flush(
                &socket,
                client_addr,
                start + Duration::from_millis(10) * u32::from(step),
            );
            let mut buffer = [0; 2048];
            while let Ok((len, _)) = client.recv_from(&mut buffer) {
                let sequence_id = u16::from_le_bytes([buffer[0], buffer[1]]);
                let ahead = sequence_id.wrapping_sub(newest);
                if ahead != 0 && ahead < 0x8000 {
                    bits = if ahead < 16 { bits >> ahead } else { 0 } | 0x8000;
                    newest = sequence_id;
                }
                let datagram = &buffer[..len];
                if datagram[4] & 1 == 0 {
                    received.insert(u16::from_le_bytes([datagram[7], datagram[8]]));
                } else {
                    let mut messages = &datagram[6..];
                    while let Some((&size, rest)) = messages.split_first() {
                        received.insert(u16::from_le_bytes([rest[1], rest[2]]));
                        messages = &rest[usize::from(size)..];
                    }
                }
            }
            transport.handle_datagram(&ack_packet(step + 1, newest, bits), &socket, client_addr);
            if received.len() == 301 {
                break;
            }
        }
        assert_eq!(received, (0..=300).collect());
        assert!(transport.stats.resent > 0);
    }

    #[test]
    fn drops_a_peer_that_acknowledges_nothing() {
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (mut transport, socket, client_addr) = connected_to(&client);
        for _ in 0..MAX_BACKLOG - 1 {
            transport.send_game_message(kd_stats(0));
        }
        transport.flush(&socket, client_addr, Instant::now());
        assert!(!transport.is_disconnected());
        transport.send_game_message(kd_stats(0));
        transport.flush(&socket, client_addr, Instant::now());
        assert!(transport.is_disconnected());
    }

    /// Every datagram waiting at the client: its sequence id and the order ids
    /// of its messages.
    fn read_all(client: &UdpSocket) -> Vec<(u16, Vec<u16>)> {
        let mut datagrams = Vec::new();
        let mut buffer = [0; 2048];
        while let Ok((len, _)) = client.recv_from(&mut buffer) {
            assert!(len <= MAX_DATAGRAM_SIZE, "a {len}-byte datagram");
            let datagram = &buffer[..len];
            let sequence_id = u16::from_le_bytes([datagram[0], datagram[1]]);
            let mut order_ids = Vec::new();
            if datagram[4] & 1 == 0 {
                if len > 8 {
                    order_ids.push(u16::from_le_bytes([datagram[7], datagram[8]]));
                }
            } else {
                let mut messages = &datagram[6..];
                while let Some((&size, rest)) = messages.split_first() {
                    order_ids.push(u16::from_le_bytes([rest[1], rest[2]]));
                    messages = &rest[usize::from(size)..];
                }
            }
            datagrams.push((sequence_id, order_ids));
        }
        datagrams
    }

    /// A client whose game stalls reports nothing: it gets a few datagrams
    /// every `RESEND_AFTER`, mostly the same messages again, while the rest
    /// wait at the server.
    #[test]
    fn a_stalled_client_is_not_flooded() {
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (mut transport, socket, client_addr) = connected_to(&client);
        client.set_nonblocking(true).unwrap();
        let start = Instant::now();
        let mut datagrams = Vec::new();
        // 10 s at 120 frames a second, 10 messages a frame.
        for frame in 0..1200_u32 {
            for _ in 0..10 {
                transport.send_game_message(kd_stats(0));
            }
            transport.flush(
                &socket,
                client_addr,
                start + Duration::from_millis(25) * frame / 3,
            );
            datagrams.extend(read_all(&client));
        }
        // 16 datagrams each `RESEND_AFTER`: 20 of them in 10 s, and the first.
        assert!(
            datagrams.len() <= MAX_UNREPORTED * 21,
            "{}",
            datagrams.len()
        );
        let distinct: std::collections::BTreeSet<u16> = datagrams
            .iter()
            .flat_map(|(_, ids)| ids.iter().copied())
            .collect();
        // About what 16 datagrams carry, of the 12000 queued.
        assert!(
            distinct.len() < 2 * MAX_UNREPORTED * 18,
            "{}",
            distinct.len()
        );
        assert!(!transport.is_disconnected());
        assert!(transport.queued.len() > 11_000);
    }

    /// The client's side of the link: acknowledges as the stock client does
    /// (its newest datagram and the 15 before it) once per call, and collects
    /// the order ids it got.
    struct Client {
        socket: UdpSocket,
        newest: u16,
        bits: u16,
        sequence: u16,
        received: std::collections::BTreeSet<u16>,
    }

    impl Client {
        fn new(socket: UdpSocket) -> Self {
            socket.set_nonblocking(true).unwrap();
            Self {
                socket,
                newest: 0xFFFF,
                bits: 0,
                sequence: 0,
                received: std::collections::BTreeSet::new(),
            }
        }

        fn read_and_report(&mut self, transport: &mut Transport, socket: &UdpSocket) {
            for (sequence_id, order_ids) in read_all(&self.socket) {
                let ahead = sequence_id.wrapping_sub(self.newest);
                if ahead != 0 && ahead < 0x8000 {
                    self.bits = if ahead < 16 { self.bits >> ahead } else { 0 } | 0x8000;
                    self.newest = sequence_id;
                } else {
                    let behind = self.newest.wrapping_sub(sequence_id);
                    if behind < 16 {
                        self.bits |= 0x8000 >> behind;
                    }
                }
                self.received.extend(order_ids);
            }
            self.sequence += 1;
            let addr = self.socket.local_addr().unwrap();
            transport.handle_datagram(
                &ack_packet(self.sequence, self.newest, self.bits),
                socket,
                addr,
            );
        }
    }

    /// Once its game runs again, the client gets everything that waited, and
    /// no message is ever beyond what its reports can acknowledge.
    #[test]
    fn a_client_that_runs_again_gets_everything() {
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (mut transport, socket, client_addr) = connected_to(&client);
        let mut client = Client::new(client);
        let start = Instant::now();
        let mut now = start;
        for _ in 0..3000 {
            transport.send_game_message(kd_stats(0));
        }
        // Stalled for 2 s.
        for _ in 0..240 {
            now += Duration::from_millis(8);
            transport.flush(&socket, client_addr, now);
        }
        read_all(&client.socket);
        // Then a report every other frame.
        for frame in 0..2000 {
            now += Duration::from_millis(8);
            transport.flush(&socket, client_addr, now);
            if frame % 2 == 0 {
                client.read_and_report(&mut transport, &socket);
            }
            if client.received.len() == 3001 && transport.unacknowledged.is_empty() {
                break;
            }
        }
        assert_eq!(client.received, (0..=3000).collect());
        assert!(transport.unacknowledged.is_empty());
        assert!(transport.queued.is_empty());
    }

    /// What a stalled client would get late can go: the rest close up behind
    /// the last message sent, and the client gets an unbroken run.
    #[test]
    fn dropped_messages_leave_no_gap() {
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (mut transport, socket, client_addr) = connected_to(&client);
        let mut client = Client::new(client);
        let mut now = Instant::now();
        for i in 0..3000 {
            transport.send_game_message(kd_stats((i % 2) as u8));
        }
        transport.flush(&socket, client_addr, now);
        let waiting = transport.queued_len();
        assert!(waiting > 2000, "{waiting}");
        let dropped = transport.drop_queued(|message| {
            matches!(
                message,
                message::ServerGameMessageKind::PlayerKdStatsChanged { player_id: 0, .. }
            )
        });
        assert!(dropped.abs_diff(waiting / 2) <= 5, "{dropped} of {waiting}");
        assert_eq!(transport.queued_len(), waiting - dropped);
        for _ in 0..2000 {
            now += Duration::from_millis(8);
            client.read_and_report(&mut transport, &socket);
            transport.flush(&socket, client_addr, now);
            if transport.queued.is_empty() && transport.unacknowledged.is_empty() {
                break;
            }
        }
        client.read_and_report(&mut transport, &socket);
        assert_eq!(
            client.received,
            (0..=transport.server_order_id.0).collect(),
            "an unbroken run of order ids"
        );
        assert_eq!(client.received.len(), 3001 - dropped);
    }

    #[test]
    fn counts_datagrams_that_leave_the_ack_window_unacknowledged() {
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        let (mut transport, socket, client_addr) = connected_to(&client);
        keepalives(&mut transport, &socket, client_addr, 20);
        // The client acknowledges the newest (19) only: 0 to 3 are past its
        // reach, 4 to 18 may still be acknowledged.
        transport.handle_datagram(&ack_packet(1, 19, 0x8000), &socket, client_addr);
        assert_eq!(transport.stats.unacknowledged, 4);
        assert_eq!(transport.in_flight.len(), 16);
    }
}
