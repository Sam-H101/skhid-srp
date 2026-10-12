//! The 0.21d match transport, the server's half of the client's
//! `udp_match_connection` (`docs/notes/transport_spec.md`).
//!
//! One `Transport` per client. It owns no socket: `receive` takes a datagram
//! from the client and `poll` returns the datagrams to send it.
//!
//! - Every datagram has a 12-byte header: `u16 seq`, `u16` the newest client
//!   seq received, `u64` acks of the 63 before it shifted left by one, with the
//!   multi flag in bit 0.
//! - Multi datagrams hold length-prefixed messages; a single one holds one
//!   message. A multi datagram whose first message fills it is a low-level
//!   packet (`01 tt`).
//! - Ordered server messages carry a `u16` order id on the client's one
//!   channel. One too large for a datagram goes as 0xFF parts.
//! - Reliable messages are resent under new seqs until a seq they went out
//!   under is acked. The client keeps no transport dedup for unordered
//!   messages, so resent client messages reach the caller again.

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

/// The client's receive buffer (`start_receiving`).
pub const MAX_DATAGRAM: usize = 0x1FC;
const HEADER: usize = 12;
/// The most one message may take in a single-format datagram.
const MAX_MESSAGE: usize = MAX_DATAGRAM - HEADER;
/// Data in part 0 and in the later parts of a multipart message, as the
/// client splits its own: each part fills a datagram.
const FIRST_PART_DATA: usize = 490;
const PART_DATA: usize = 491;
const MULTIPART: u8 = 0xFF;

/// The client flushes every 100 ms and times out after 5 s without a
/// datagram; a keepalive this often keeps its acks flowing.
const IDLE: Duration = Duration::from_millis(100);
/// How long a peer that sends nothing stays connected.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// The client acks only through its 63 seqs before the newest; this many
/// datagrams past the newest ack keeps every one inside that window.
const MAX_AHEAD: u16 = 48;
/// The client acks a message through the last 8 seqs it went under.
const SEQS_PER_MESSAGE: usize = 8;
/// A message whose datagram is unacked while one sent this many seqs later
/// was acked is taken as lost and goes again at once, instead of waiting out
/// its resend time: a lost relay then reaches the client in about a round
/// trip, not 200 ms later (the client rejects inputs older than 500 ms).
const FAST_RESEND_GAP: u16 = 3;
/// How many sent datagrams are remembered to time their acks.
const SENT_KEPT: usize = 128;
/// How long a disconnect waits for the client's confirm.
const CLOSING: Duration = Duration::from_secs(1);

/// `udp_match_low_level_message_types`.
mod low_level {
    pub const INITIATE_DISCONNECTION: u8 = 0;
    pub const CONFIRM_DISCONNECTION: u8 = 1;
    pub const CONTINUOUS_FLOW: u8 = 2;
}

/// `a` is newer than `b` in the client's half-range order.
fn is_newer(a: u16, b: u16) -> bool {
    let distance = a.wrapping_sub(b);
    distance != 0 && distance < 0x8000
}

/// How the client's `get_received_message_info` treats a server message:
/// whether it carries an order id, and how soon an unacked copy goes again.
fn message_info(id: u8) -> Option<(bool, Duration)> {
    match id {
        0x50 => Some((false, Duration::from_millis(500))),
        0x51 | 0x52 | 0x53 | 0x55 | 0x59 => Some((true, Duration::from_millis(500))),
        0x54 | 0x56 | 0x57 | 0x58 => Some((false, Duration::from_millis(200))),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// A client message, `0x40..=0x47`, without its id.
    Message { id: u8, payload: Vec<u8> },
    /// The client left, or confirmed the server's disconnect.
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Connected,
    /// The server disconnects and waits for the confirm.
    Closing {
        since: Instant,
    },
    Closed,
}

/// Why `send` refused a message.
#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    /// Not an id the client reads (0x50..=0x59).
    UnknownId(u8),
    /// Too large for one datagram and not ordered, or more than 255 parts.
    TooLarge(usize),
    NotConnected,
}

struct Message {
    /// Which `send` it came from: the parts of one message share it.
    serial: u64,
    /// Id, order id if ordered, payload; or a multipart part.
    bytes: Vec<u8>,
    resend: Duration,
    /// The datagram seqs it went out under, the newest last.
    seqs: VecDeque<u16>,
}

pub struct Transport {
    next_seq: u16,
    /// The newest client seq and which of the 63 before it arrived, with
    /// bit 63 for the newest itself.
    received: Option<u16>,
    received_bits: u64,
    /// The newest server seq the client acked.
    newest_acked: Option<u16>,
    next_order: u16,
    next_serial: u64,
    pending: VecDeque<Message>,
    unacked: Vec<(Message, Instant)>,
    low_level: VecDeque<u8>,
    last_sent: Option<Instant>,
    last_received: Instant,
    state: State,
    /// Messages sent again, after their resend time or as lost.
    resent: u32,
    /// When the latest datagrams went out, by seq: an ack for one is a
    /// round trip.
    sent_at: VecDeque<(u16, Instant)>,
    /// The round trips measured since `take_round_trips`, in ms.
    rtt_sum: u64,
    rtt_count: u32,
    rtt_max: u32,
    /// The latest round trip measured, in ms.
    rtt_last: Option<u32>,
}

impl Transport {
    pub fn new(now: Instant) -> Self {
        Self {
            next_seq: 0,
            received: None,
            received_bits: 0,
            newest_acked: None,
            next_order: 0,
            next_serial: 0,
            pending: VecDeque::new(),
            unacked: Vec::new(),
            low_level: VecDeque::new(),
            last_sent: None,
            last_received: now,
            state: State::Connected,
            resent: 0,
            sent_at: VecDeque::new(),
            rtt_sum: 0,
            rtt_count: 0,
            rtt_max: 0,
            rtt_last: None,
        }
    }

    /// How many messages went again: a count of lost or late datagrams.
    pub fn resent(&self) -> u32 {
        self.resent
    }

    /// The round trips measured since the last call, as the mean and the
    /// longest in ms: from a datagram's send to the ack that names it, the
    /// client's own delay in acking included. `None` if there were none.
    pub fn take_round_trips(&mut self) -> Option<(u32, u32)> {
        let count = std::mem::take(&mut self.rtt_count);
        let sum = std::mem::take(&mut self.rtt_sum);
        let max = std::mem::take(&mut self.rtt_max);
        (count > 0).then(|| ((sum / u64::from(count)) as u32, max))
    }

    /// The latest round trip measured (see [`Self::take_round_trips`]), in ms.
    pub fn last_round_trip(&self) -> Option<u32> {
        self.rtt_last
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// The client sent nothing for `TIMEOUT`, or a disconnect ended.
    pub fn is_gone(&self, now: Instant) -> bool {
        self.is_gone_after(now, TIMEOUT)
    }

    /// The same, for a client allowed to be silent for `timeout`: one that is
    /// loading its level sends and acks nothing until it is done.
    pub fn is_gone_after(&self, now: Instant, timeout: Duration) -> bool {
        self.state == State::Closed || now.duration_since(self.last_received) >= timeout
    }

    /// Reliable messages not yet acked, sent or not.
    pub fn backlog(&self) -> usize {
        self.pending.len() + self.unacked.len()
    }

    /// Handle one datagram from the client.
    pub fn receive(&mut self, datagram: &[u8], now: Instant) -> Vec<Event> {
        let Some((header, body)) = datagram.split_first_chunk::<HEADER>() else {
            return Vec::new();
        };
        let seq = u16::from_le_bytes([header[0], header[1]]);
        let acked = u16::from_le_bytes([header[2], header[3]]);
        let ack_multi = u64::from_le_bytes(header[4..].try_into().unwrap());
        self.last_received = now;
        if !self.record_received(seq) {
            return Vec::new();
        }
        self.acknowledge(acked, ack_multi, now);

        let mut events = Vec::new();
        if ack_multi & 1 == 0 {
            if !body.is_empty() {
                self.message(body, &mut events);
            }
            return events;
        }
        let mut rest = body;
        let mut first = true;
        while let Some((&len, tail)) = rest.split_first() {
            let (len, tail) = match len {
                0 => match tail.split_first_chunk::<2>() {
                    Some((len, tail)) => (usize::from(u16::from_le_bytes(*len)), tail),
                    None => break,
                },
                len => (usize::from(len), tail),
            };
            let Some((message, tail)) = tail.split_at_checked(len) else {
                break;
            };
            if first && tail.is_empty() {
                if let Some(&kind) = message.first() {
                    self.low_level_message(kind, now, &mut events);
                }
                return events;
            }
            first = false;
            if !message.is_empty() {
                self.message(message, &mut events);
            }
            rest = tail;
        }
        events
    }

    /// Record a client seq. False for one already received.
    fn record_received(&mut self, seq: u16) -> bool {
        let Some(newest) = self.received else {
            self.received = Some(seq);
            self.received_bits = 1 << 63;
            return true;
        };
        if seq == newest {
            return false;
        }
        if is_newer(newest, seq) {
            let back = newest.wrapping_sub(seq);
            if back < 64 {
                let bit = 1 << (63 - back);
                if self.received_bits & bit != 0 {
                    return false;
                }
                self.received_bits |= bit;
            }
        } else {
            let ahead = seq.wrapping_sub(newest);
            self.received_bits = if ahead >= 64 {
                0
            } else {
                self.received_bits >> ahead
            };
            self.received_bits |= 1 << 63;
            self.received = Some(seq);
        }
        true
    }

    /// The client's acks: `acked` and every seq its bits name.
    fn acknowledge(&mut self, acked: u16, ack_multi: u64, now: Instant) {
        // An ack for a seq not sent yet, such as 0xFFFF before any.
        if self.next_seq == 0 || is_newer(acked, self.next_seq.wrapping_sub(1)) {
            return;
        }
        // The ack's own seq is the datagram the client had just received:
        // the older ones named by its bits were acked later than they came.
        if let Some(at) = self.sent_at.iter().position(|(seq, _)| *seq == acked) {
            let (_, sent) = self.sent_at.remove(at).unwrap();
            let ms = now.saturating_duration_since(sent).as_millis() as u32;
            self.rtt_sum += u64::from(ms);
            self.rtt_count += 1;
            self.rtt_max = self.rtt_max.max(ms);
            self.rtt_last = Some(ms);
        }
        let mut seqs = HashSet::from([acked]);
        for bit in 1..64 {
            if ack_multi >> bit & 1 != 0 {
                seqs.insert(acked.wrapping_sub(64 - bit));
            }
        }
        self.unacked
            .retain(|(message, _)| !message.seqs.iter().any(|seq| seqs.contains(seq)));
        if self
            .newest_acked
            .is_none_or(|newest| is_newer(acked, newest))
        {
            self.newest_acked = Some(acked);
        }
        // What an ack skipped over was probably lost: send it again now.
        if let Some(newest) = self.newest_acked {
            for (message, at) in &mut self.unacked {
                let skipped = message.seqs.back().is_some_and(|sent| {
                    is_newer(newest, *sent) && newest.wrapping_sub(*sent) >= FAST_RESEND_GAP
                });
                if skipped && *at > now {
                    *at = now;
                }
            }
        }
    }

    fn message(&mut self, message: &[u8], events: &mut Vec<Event>) {
        let id = message[0];
        if (0x40..=0x47).contains(&id) {
            events.push(Event::Message {
                id,
                payload: message[1..].to_vec(),
            });
        } else {
            log::debug!("A client message with id {id:#04x}; ignored");
        }
    }

    fn low_level_message(&mut self, kind: u8, now: Instant, events: &mut Vec<Event>) {
        match kind {
            low_level::INITIATE_DISCONNECTION => {
                // The client asks until it hears the confirm.
                self.low_level.push_back(low_level::CONFIRM_DISCONNECTION);
                if self.state != State::Closed {
                    self.close();
                    events.push(Event::Disconnected);
                }
            }
            low_level::CONFIRM_DISCONNECTION => {
                if matches!(self.state, State::Closing { .. }) {
                    self.close();
                    events.push(Event::Disconnected);
                }
            }
            low_level::CONTINUOUS_FLOW => {}
            _ => log::debug!("A low-level packet of type {kind} at {now:?}; ignored"),
        }
    }

    fn close(&mut self) {
        self.state = State::Closed;
        self.pending.clear();
        self.unacked.clear();
    }

    /// Queue a server message (`0x50..=0x59`). An ordered one takes the next
    /// order id; one too large for a datagram goes in parts. Returns its
    /// serial for [`Self::is_delivered`].
    pub fn send(&mut self, id: u8, payload: &[u8]) -> Result<u64, SendError> {
        let (ordered, resend) = message_info(id).ok_or(SendError::UnknownId(id))?;
        if self.state != State::Connected {
            return Err(SendError::NotConnected);
        }
        let serial = self.next_serial;
        let header = if ordered { 3 } else { 1 };
        if header + payload.len() <= MAX_MESSAGE {
            let mut bytes = Vec::with_capacity(header + payload.len());
            bytes.push(id);
            if ordered {
                bytes.extend(self.take_order().to_le_bytes());
            }
            bytes.extend(payload);
            self.queue(bytes, resend);
            self.next_serial += 1;
            return Ok(serial);
        }
        // Only the ordered channel assembles parts.
        let parts = 1 + payload
            .len()
            .saturating_sub(FIRST_PART_DATA)
            .div_ceil(PART_DATA);
        if !ordered || parts > 255 {
            return Err(SendError::TooLarge(payload.len()));
        }
        let order = self.take_order().to_le_bytes();
        let (first, mut rest) = payload.split_at(FIRST_PART_DATA);
        let mut part = vec![MULTIPART, id, order[0], order[1], 0, parts as u8];
        part.extend(first);
        self.queue(part, resend);
        for index in 1..parts {
            let (data, tail) = rest.split_at(rest.len().min(PART_DATA));
            rest = tail;
            let mut part = vec![MULTIPART, id, order[0], order[1], index as u8];
            part.extend(data);
            self.queue(part, resend);
        }
        self.next_serial += 1;
        Ok(serial)
    }

    /// Whether the client acked every part of a message sent.
    pub fn is_delivered(&self, serial: u64) -> bool {
        serial < self.next_serial
            && !self.pending.iter().any(|message| message.serial == serial)
            && !self
                .unacked
                .iter()
                .any(|(message, _)| message.serial == serial)
    }

    fn take_order(&mut self) -> u16 {
        let order = self.next_order;
        self.next_order = order.wrapping_add(1);
        order
    }

    fn queue(&mut self, bytes: Vec<u8>, resend: Duration) {
        self.pending.push_back(Message {
            serial: self.next_serial,
            bytes,
            resend,
            seqs: VecDeque::new(),
        });
    }

    /// Disconnect the client: it confirms and leaves within 500 ms.
    pub fn disconnect(&mut self, now: Instant) {
        if self.state == State::Connected {
            self.pending.clear();
            self.unacked.clear();
            self.state = State::Closing { since: now };
        }
    }

    /// The datagrams to send now: due resends and new messages, low-level
    /// packets, and a keepalive when nothing went for a while.
    pub fn poll(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let mut datagrams = Vec::new();
        if let State::Closing { since } = self.state {
            if now.duration_since(since) >= CLOSING {
                self.state = State::Closed;
            } else if self
                .last_sent
                .is_none_or(|sent| now.duration_since(sent) >= IDLE)
            {
                self.low_level.push_back(low_level::INITIATE_DISCONNECTION);
            }
        }
        while let Some(kind) = self.low_level.pop_front() {
            datagrams.push(self.datagram(true, &[1, kind]));
        }

        // Resends first, oldest first, so a gap in the order closes soon.
        let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.unacked)
            .into_iter()
            .partition(|(_, at)| *at <= now);
        self.unacked = waiting;
        self.resent += due.len() as u32;
        let due = due.into_iter().map(|(message, _)| message);
        for message in due.into_iter().rev() {
            self.pending.push_front(message);
        }

        while !self.pending.is_empty() && self.ahead() < MAX_AHEAD {
            let mut taken = vec![self.pending.pop_front().unwrap()];
            let mut used = prefixed_len(&taken[0].bytes);
            while let Some(next) = self.pending.front() {
                let cost = prefixed_len(&next.bytes);
                if used + cost > MAX_MESSAGE {
                    break;
                }
                used += cost;
                taken.push(self.pending.pop_front().unwrap());
            }
            let datagram = if let [message] = taken.as_slice() {
                self.datagram(false, &message.bytes)
            } else {
                let mut body = Vec::with_capacity(used);
                for message in &taken {
                    let len = message.bytes.len();
                    if len < 256 {
                        body.push(len as u8);
                    } else {
                        body.push(0);
                        body.extend((len as u16).to_le_bytes());
                    }
                    body.extend(&message.bytes);
                }
                self.datagram(true, &body)
            };
            let seq = self.next_seq.wrapping_sub(1);
            for mut message in taken {
                if message.seqs.len() == SEQS_PER_MESSAGE {
                    message.seqs.pop_front();
                }
                message.seqs.push_back(seq);
                let at = now + message.resend;
                self.unacked.push((message, at));
            }
            datagrams.push(datagram);
        }

        if datagrams.is_empty()
            && self.state != State::Closed
            && self
                .last_sent
                .is_none_or(|sent| now.duration_since(sent) >= IDLE)
        {
            datagrams.push(self.datagram(true, &[1, low_level::CONTINUOUS_FLOW]));
        }
        if !datagrams.is_empty() {
            self.last_sent = Some(now);
        }
        for datagram in &datagrams {
            let seq = u16::from_le_bytes([datagram[0], datagram[1]]);
            if self.sent_at.len() == SENT_KEPT {
                self.sent_at.pop_front();
            }
            self.sent_at.push_back((seq, now));
        }
        datagrams
    }

    /// Datagrams sent past the newest the client acked.
    fn ahead(&self) -> u16 {
        match self.newest_acked {
            Some(acked) => self.next_seq.wrapping_sub(acked).wrapping_sub(1),
            None => self.next_seq,
        }
    }

    /// A datagram under the next seq, with the current acks.
    fn datagram(&mut self, multi: bool, body: &[u8]) -> Vec<u8> {
        let seq = self.next_seq;
        self.next_seq = seq.wrapping_add(1);
        let mut datagram = Vec::with_capacity(HEADER + body.len());
        datagram.extend(seq.to_le_bytes());
        datagram.extend(self.received.unwrap_or(0xFFFF).to_le_bytes());
        datagram.extend((self.received_bits << 1 | u64::from(multi)).to_le_bytes());
        datagram.extend(body);
        debug_assert!(datagram.len() <= MAX_DATAGRAM);
        datagram
    }
}

/// A message's size in a multi datagram.
fn prefixed_len(bytes: &[u8]) -> usize {
    if bytes.len() < 256 {
        1 + bytes.len()
    } else {
        3 + bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client datagram: `seq`, acking server `acked` and the seqs in
    /// `also` (each older than `acked`), then `body`.
    fn client(seq: u16, acked: u16, also: &[u16], multi: bool, body: &[u8]) -> Vec<u8> {
        let mut bits = u64::from(multi);
        for seq in also {
            bits |= 1 << (64 - acked.wrapping_sub(*seq));
        }
        let mut datagram = Vec::new();
        datagram.extend(seq.to_le_bytes());
        datagram.extend(acked.to_le_bytes());
        datagram.extend(bits.to_le_bytes());
        datagram.extend(body);
        datagram
    }

    fn connect() -> (Transport, Instant) {
        let now = Instant::now();
        let mut transport = Transport::new(now);
        let events = transport.receive(&client(0, 0xFFFF, &[], false, &[0x40, 1, 2, 3, 4, 0]), now);
        assert_eq!(
            events,
            [Event::Message {
                id: 0x40,
                payload: vec![1, 2, 3, 4, 0]
            }]
        );
        (transport, now)
    }

    fn seq_of(datagram: &[u8]) -> u16 {
        u16::from_le_bytes([datagram[0], datagram[1]])
    }

    #[test]
    fn a_lone_message_goes_in_the_single_format() {
        let (mut transport, now) = connect();
        transport.send(0x50, &[0xD4, 4, 0xD4, 4]).unwrap();
        let datagrams = transport.poll(now);
        assert_eq!(
            datagrams,
            [vec![
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x50, 0xD4, 4, 0xD4, 4
            ]]
        );
    }

    #[test]
    fn small_messages_share_a_multi_datagram_with_their_order_ids() {
        let (mut transport, now) = connect();
        transport.send(0x54, &[1, 2]).unwrap();
        transport.send(0x55, &[0xFB, 0xFF]).unwrap();
        transport.send(0x52, &[9]).unwrap();
        let datagrams = transport.poll(now);
        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0][4], 1, "multi");
        assert_eq!(
            datagrams[0][HEADER..],
            [3, 0x54, 1, 2, 5, 0x55, 0, 0, 0xFB, 0xFF, 4, 0x52, 1, 0, 9]
        );
    }

    #[test]
    fn a_large_ordered_message_goes_in_parts_that_fill_datagrams() {
        let (mut transport, now) = connect();
        transport.send(0x52, &[0]).unwrap();
        let payload: Vec<u8> = (0..12000u32).map(|i| i as u8).collect();
        transport.send(0x51, &payload).unwrap();
        let datagrams = transport.poll(now);
        // The small 0x52 shares no datagram with a full part.
        assert_eq!(datagrams.len(), 26);
        let part0 = &datagrams[1];
        assert_eq!(part0.len(), MAX_DATAGRAM);
        assert_eq!(part0[4] & 1, 0, "single");
        assert_eq!(part0[HEADER..HEADER + 6], [0xFF, 0x51, 1, 0, 0, 25]);
        assert_eq!(datagrams[2][HEADER..HEADER + 5], [0xFF, 0x51, 1, 0, 1]);
        assert_eq!(datagrams[2].len(), MAX_DATAGRAM);
        let last = &datagrams[25];
        assert_eq!(last[HEADER..HEADER + 5], [0xFF, 0x51, 1, 0, 24]);
        assert_eq!(last.len(), HEADER + 5 + 12000 - 490 - 23 * 491);
        let data: Vec<u8> = datagrams[1..]
            .iter()
            .enumerate()
            .flat_map(|(i, d)| d[HEADER + if i == 0 { 6 } else { 5 }..].to_vec())
            .collect();
        assert_eq!(data, payload);
    }

    #[test]
    fn an_unordered_message_too_large_for_a_datagram_is_refused() {
        let (mut transport, _) = connect();
        assert_eq!(
            transport.send(0x54, &[0; 500]),
            Err(SendError::TooLarge(500))
        );
        assert_eq!(transport.send(0x60, &[]), Err(SendError::UnknownId(0x60)));
    }

    #[test]
    fn a_message_the_client_skipped_over_in_its_acks_goes_again_at_once() {
        let (mut transport, now) = connect();
        // Four relays in four datagrams (seqs 0 to 3), the first one lost.
        let mut sent = Vec::new();
        for n in 0..4u8 {
            transport.send(0x54, &[n]).unwrap();
            sent.push(transport.poll(now).remove(0));
        }
        assert_eq!(seq_of(&sent[3]), 3);
        // The client acks 3, 2 and 1, but not 0: no waiting for 200 ms.
        transport.receive(&client(1, 3, &[1, 2], true, &[1, 2]), now);
        let again = transport.poll(now);
        assert!(
            again
                .iter()
                .any(|datagram| datagram[HEADER..] == sent[0][HEADER..]),
            "the lost relay goes again"
        );
        assert_eq!(transport.resent(), 1);
    }

    #[test]
    fn the_time_to_an_ack_is_the_round_trip() {
        let (mut transport, now) = connect();
        assert_eq!(transport.take_round_trips(), None);
        // Seqs 0 and 1 go out together; the client acks 0 after 40 ms and 1
        // after 90 ms (its ack of 1 also names 0, which counts once).
        transport.send(0x54, &[0]).unwrap();
        transport.poll(now);
        transport.send(0x54, &[1]).unwrap();
        transport.poll(now);
        transport.receive(
            &client(1, 0, &[], true, &[1, 2]),
            now + Duration::from_millis(40),
        );
        transport.receive(
            &client(2, 1, &[0], true, &[1, 2]),
            now + Duration::from_millis(90),
        );
        assert_eq!(transport.take_round_trips(), Some((65, 90)));
        // Taken: the next window starts empty, and a repeated ack adds none.
        transport.receive(
            &client(3, 1, &[0], true, &[1, 2]),
            now + Duration::from_millis(95),
        );
        assert_eq!(transport.take_round_trips(), None);
    }

    #[test]
    fn a_message_only_one_datagram_behind_the_acks_waits_for_its_time() {
        let (mut transport, now) = connect();
        transport.send(0x54, &[0]).unwrap();
        transport.poll(now);
        transport.send(0x54, &[1]).unwrap();
        transport.poll(now);
        // 1 acked, 0 not: that may be reordering, so no resend yet.
        transport.receive(&client(1, 1, &[], true, &[1, 2]), now);
        let again = transport.poll(now + Duration::from_millis(10));
        assert!(
            again
                .iter()
                .all(|datagram| datagram[HEADER..] != [3, 0x54, 0][..])
        );
        assert_eq!(transport.resent(), 0);
    }

    #[test]
    fn an_unacked_message_goes_again_under_a_new_seq_until_acked() {
        let (mut transport, now) = connect();
        let serial = transport.send(0x56, &[1]).unwrap();
        let first = transport.poll(now);
        assert_eq!(seq_of(&first[0]), 0);

        let later = now + Duration::from_millis(200);
        let again = transport.poll(later);
        assert_eq!(again.len(), 1);
        assert_eq!(seq_of(&again[0]), 1);
        assert_eq!(again[0][HEADER..], first[0][HEADER..]);

        // An ack of the first copy's seq is enough.
        assert!(!transport.is_delivered(serial));
        transport.receive(&client(1, 0, &[], true, &[1, 2]), later);
        assert_eq!(transport.backlog(), 0);
        assert!(transport.is_delivered(serial));
        let quiet = transport.poll(later + Duration::from_millis(400));
        assert_eq!(quiet.len(), 1);
        assert_eq!(quiet[0][HEADER..], [1, 2], "only a keepalive");
    }

    #[test]
    fn every_datagram_acks_what_arrived() {
        let (mut transport, now) = connect();
        // Seq 2 arrives before 1; 1 then arrives too, and 1 again is dropped.
        transport.receive(&client(2, 0xFFFF, &[], true, &[1, 2]), now);
        let datagram = transport.poll(now).remove(0);
        assert_eq!(datagram[2..4], [2, 0]);
        assert_eq!(
            u64::from_le_bytes(datagram[4..12].try_into().unwrap()),
            1 << 62 | 1
        );

        let events = transport.receive(&client(1, 0xFFFF, &[], false, &[0x47, 0, 0, 0, 0]), now);
        assert_eq!(events.len(), 1);
        let again = transport.receive(&client(1, 0xFFFF, &[], false, &[0x47, 0, 0, 0, 0]), now);
        assert!(again.is_empty());
        let datagram = transport.poll(now + IDLE).remove(0);
        assert_eq!(
            u64::from_le_bytes(datagram[4..12].try_into().unwrap()),
            1 << 63 | 1 << 62 | 1
        );
    }

    #[test]
    fn a_quiet_link_gets_a_keepalive_every_100_ms() {
        let (mut transport, now) = connect();
        assert_eq!(transport.poll(now)[0][HEADER..], [1, 2]);
        assert!(transport.poll(now + Duration::from_millis(50)).is_empty());
        let keepalive = transport.poll(now + IDLE);
        assert_eq!(keepalive.len(), 1);
        assert_eq!(keepalive[0].len(), 14);
    }

    #[test]
    fn the_client_leaving_is_confirmed() {
        let (mut transport, now) = connect();
        transport.send(0x56, &[1]).unwrap();
        let events = transport.receive(&client(1, 0xFFFF, &[], true, &[1, 0]), now);
        assert_eq!(events, [Event::Disconnected]);
        let datagrams = transport.poll(now);
        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0][HEADER..], [1, 1]);
        // Asked again, it confirms again without a second event.
        let events = transport.receive(&client(2, 0xFFFF, &[], true, &[1, 0]), now);
        assert!(events.is_empty());
        assert_eq!(transport.poll(now)[0][HEADER..], [1, 1]);
        assert!(transport.is_gone(now));
    }

    #[test]
    fn a_disconnect_waits_for_the_confirm() {
        let (mut transport, now) = connect();
        transport.disconnect(now);
        assert_eq!(transport.poll(now)[0][HEADER..], [1, 0]);
        let events = transport.receive(&client(1, 0, &[], true, &[1, 1]), now);
        assert_eq!(events, [Event::Disconnected]);
        assert!(transport.is_gone(now));
    }

    #[test]
    fn messages_in_a_multi_datagram_reach_the_caller() {
        let (mut transport, now) = connect();
        let body = [2, 0x44, 7, 3, 0x46, 8, 9];
        let events = transport.receive(&client(1, 0xFFFF, &[], true, &body), now);
        assert_eq!(
            events,
            [
                Event::Message {
                    id: 0x44,
                    payload: vec![7]
                },
                Event::Message {
                    id: 0x46,
                    payload: vec![8, 9]
                },
            ]
        );
    }

    #[test]
    fn sending_stops_short_of_the_ack_window() {
        let (mut transport, now) = connect();
        let payload = vec![0; 60 * 491];
        transport.send(0x53, &payload).unwrap();
        let sent = transport.poll(now);
        assert_eq!(sent.len(), usize::from(MAX_AHEAD));
        let newest = seq_of(sent.last().unwrap());
        let also: Vec<u16> = (0..newest).collect();
        let also: Vec<u16> = also.into_iter().filter(|seq| newest - seq < 64).collect();
        transport.receive(&client(1, newest, &also, true, &[1, 2]), now);
        let rest = transport.poll(now);
        assert_eq!(rest.len(), 61 - usize::from(MAX_AHEAD));
    }

    #[test]
    fn a_client_may_be_given_longer_to_be_silent() {
        let (transport, now) = connect();
        let long = TIMEOUT * 6;
        assert!(!transport.is_gone_after(now + TIMEOUT, long));
        assert!(transport.is_gone_after(now + long, long));
    }

    #[test]
    fn a_silent_client_is_gone_after_the_timeout() {
        let (transport, now) = connect();
        assert!(!transport.is_gone(now + TIMEOUT - Duration::from_millis(1)));
        assert!(transport.is_gone(now + TIMEOUT));
    }
}
