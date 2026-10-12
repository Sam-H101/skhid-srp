//! A model of the 0.26g client's `udp_match_connection`, for tests only. It
//! follows the client's rules as strictly as transport.md states them, and
//! has its own header and message parser (written from the client's reader
//! formulas, not from `crate::wire`), so the server's encoder is checked
//! against a second reading of the format.
//!
//! - Receiving: §2.3 (order of checks), §2.4 (ack processing, with the
//!   "inconsistency" check), §3.1 (parser), §3.3 and §4.1 (plain, unique,
//!   ordered delivery), §5 and the 0x3F4A50 trace (split messages, with the
//!   EXPIRED disconnect), §3.4 (low-level ids).
//! - Sending: one message per datagram unless `join` is set, the send
//!   window of §2.5, resends with the last-8-seqs ring, `FC` when idle.
//!
//! Anything the real client would drop, log as an error or crash on is
//! recorded in `faults`: a correct server leaves it empty.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crate::ack;
use crate::table::{Class, Direction, Length, Lifetime, Lookup, Table};

const MAX_DATAGRAM: usize = 0x1FC;

/// The client's `is_newer`-style compare at 0x210000: true when `b` is not
/// newer than `a` (with the client's own treatment of a distance of 128).
fn not_newer(a: u8, b: u8) -> bool {
    if a < b {
        u32::from(a) + 128 <= u32::from(b)
    } else {
        u32::from(b) + 128 > u32::from(a)
    }
}

fn newer(a: u8, b: u8) -> bool {
    let distance = a.wrapping_sub(b);
    distance != 0 && distance < 128
}

/// The tracker at conn+0x170 / channel+0x18: newest (init 0xFF) and bits.
#[derive(Clone, Copy)]
struct Tracker {
    newest: u8,
    bits: u128,
}

impl Tracker {
    fn new() -> Self {
        Self {
            newest: 0xFF,
            bits: 0,
        }
    }

    /// 0x3F3AA0: in the window and marked. A value newer than the newest,
    /// or 128 behind it, is "not received".
    fn contains(&self, value: u8) -> bool {
        if newer(value, self.newest) {
            return false;
        }
        let back = u32::from(self.newest.wrapping_sub(value));
        back < 128 && self.bits >> (127 - back) & 1 != 0
    }

    fn record(&mut self, value: u8) {
        if newer(value, self.newest) {
            let ahead = u32::from(value.wrapping_sub(self.newest));
            self.bits = self.bits >> ahead | 1 << 127;
            self.newest = value;
        } else {
            let back = u32::from(self.newest.wrapping_sub(value));
            if back < 128 {
                self.bits |= 1 << (127 - back);
            }
        }
    }
}

struct Parsed {
    id: u8,
    low_level: bool,
    class: Class,
    x: u8,
    /// Part index and, for part 0, the count.
    part: Option<(u8, Option<u8>)>,
    payload: Vec<u8>,
}

/// The parts of a split message; the count is 0xFF until part 0 arrives
/// (packet init 0x278D92).
struct Assembly {
    id: u8,
    count: u8,
    parts: BTreeMap<u8, Vec<u8>>,
}

struct Packet {
    /// The message bytes: `id [x] payload` or a part.
    bytes: Vec<u8>,
    /// Where the payload starts (after id, x, part, count).
    header: usize,
    variable: bool,
    lifetime: Lifetime,
    resend: Duration,
    expires: Option<Instant>,
    seqs: VecDeque<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelState {
    Connected,
    /// The client asked to leave (state 1) and repeats `FA`.
    Leaving,
    /// The server asked (state 2): `FB` for 500 ms, then gone.
    Confirming {
        since: Instant,
    },
    Gone,
}

pub struct ClientModel {
    table: Table,
    /// The ED/EE mask size (getter+8); 0 until the roster is known.
    pub mask_bytes: usize,
    /// Join queued messages into multi datagrams (`-join_net_messages`).
    pub join: bool,
    /// Tail ack bytes to send; `None` picks a varying number.
    pub tail: Option<usize>,
    received: Tracker,
    /// L, the last own seq sent, and A, the newest own seq acked.
    last_sent_seq: u8,
    newest_acked: u8,
    acked_bits: u128,
    unique: Tracker,
    unique_newest_part: u8,
    unique_parts: BTreeMap<u8, Assembly>,
    order_last: u8,
    order_newest_part: u8,
    order_waiting: BTreeMap<u8, (u8, Vec<u8>)>,
    order_parts: BTreeMap<u8, Assembly>,
    next_unique: u8,
    queue: VecDeque<Packet>,
    unacked: Vec<(Packet, Instant)>,
    low_level: VecDeque<u8>,
    last_send: Option<Instant>,
    last_receive: Option<Instant>,
    sent_count: u32,
    pub state: ModelState,
    /// Every message handed to the game layer, in order.
    pub delivered: Vec<(u8, Vec<u8>)>,
    /// What a correct server never causes.
    pub faults: Vec<String>,
    /// Datagrams dropped as already received.
    pub duplicates: u32,
}

impl ClientModel {
    pub fn new(table: Table) -> Self {
        Self {
            table,
            mask_bytes: 0,
            join: false,
            tail: None,
            received: Tracker::new(),
            last_sent_seq: 0xFF,
            newest_acked: 0xFF,
            acked_bits: 0,
            unique: Tracker::new(),
            unique_newest_part: 0,
            unique_parts: BTreeMap::new(),
            order_last: 0xFF,
            order_newest_part: 0,
            order_waiting: BTreeMap::new(),
            order_parts: BTreeMap::new(),
            next_unique: 0,
            queue: VecDeque::new(),
            unacked: Vec::new(),
            low_level: VecDeque::new(),
            last_send: None,
            last_receive: None,
            sent_count: 0,
            state: ModelState::Connected,
            delivered: Vec::new(),
            faults: Vec::new(),
            duplicates: 0,
        }
    }

    /// Messages queued or waiting for their ack.
    pub fn backlog(&self) -> usize {
        self.queue.len() + self.unacked.len()
    }

    // ---------------------------------------------------------- receiving

    /// The stateless part of the client's checks: header, non-empty message
    /// area, every message by the table. `Err` is what the client logs.
    pub fn check(table: Table, mask_bytes: usize, datagram: &[u8]) -> Result<(), String> {
        let (_, _, _, _, multi, tail) = parse_header(datagram)?;
        parse_messages(
            table,
            mask_bytes,
            &datagram[3..datagram.len() - tail],
            multi,
        )
        .map(|_| ())
    }

    /// `process_incoming_packet` 0x3F3BB0.
    pub fn receive(&mut self, datagram: &[u8], now: Instant) {
        if self.state == ModelState::Gone {
            return;
        }
        // 1. last_recv = now, before any parsing.
        self.last_receive = Some(now);
        // 2. The header.
        let (seq, rseq, acks, count, multi, tail) = match parse_header(datagram) {
            Ok(header) => header,
            Err(fault) => return self.faults.push(fault),
        };
        // 3. 0x3F1C40: the same seq again, or an echo of a seq never sent.
        if seq == self.received.newest {
            self.duplicates += 1;
            return;
        }
        if newer(rseq, self.last_sent_seq) {
            return self.faults.push(format!(
                "rseq {rseq} is newer than the last seq sent {}",
                self.last_sent_seq
            ));
        }
        // 4. Every message; a failure drops the datagram unacked.
        let area = &datagram[3..datagram.len() - tail];
        let messages = match parse_messages(self.table, self.mask_bytes, area, multi) {
            Ok(messages) => messages,
            Err(fault) => return self.faults.push(fault),
        };
        // 5. The acks.
        self.update_acks(rseq, acks, count);
        // 6. The seq: an older one already marked is a duplicate.
        if self.received.contains(seq) {
            self.duplicates += 1;
            return;
        }
        self.received.record(seq);
        // 7. Dispatch.
        for message in messages {
            if self.state == ModelState::Gone {
                break;
            }
            if message.low_level {
                self.low_level_message(message.id, now);
            } else {
                self.message(message);
            }
        }
    }

    /// `update_local_acknowledgements` 0x3FD7E0.
    fn update_acks(&mut self, r: u8, acks: u128, count: u32) {
        let a = self.newest_acked;
        if r == a {
            return;
        }
        if !newer(r, a) {
            let d = u32::from(a.wrapping_sub(r));
            if d >= 128 {
                self.faults
                    .push("skip updating acknowledgements - not enough bits".into());
                return;
            }
            self.acknowledge(r, count, acks);
            self.acked_bits |= 1 << (127 - d);
            return;
        }
        let d = u32::from(r.wrapping_sub(a));
        let shifted = self.acked_bits >> d;
        let new = if count > d {
            let below = self.acked_bits << (count - d) >> count;
            let new = acks | below;
            if shifted & new != shifted {
                self.faults
                    .push("inconsistency in acknowledgements bits".into());
                return;
            }
            new
        } else {
            shifted | acks
        };
        self.acked_bits = new;
        self.newest_acked = r;
        self.acknowledge(r, count, new ^ shifted);
    }

    /// `acknowledge_packets` 0x3FD690: delete every unacked packet whose
    /// seq ring has an acked seq.
    fn acknowledge(&mut self, r: u8, count: u32, bits: u128) {
        for k in 0..count {
            if bits >> (127 - k) & 1 != 0 {
                let seq = r.wrapping_sub(k as u8);
                self.unacked
                    .retain(|(packet, _)| !packet.seqs.contains(&seq));
            }
        }
    }

    /// 0x3FDCF0.
    fn low_level_message(&mut self, id: u8, now: Instant) {
        match id {
            0xFA => {
                if self.state == ModelState::Connected {
                    self.state = ModelState::Confirming { since: now };
                    self.low_level.push_back(0xFB);
                }
            }
            0xFB => {
                if self.state == ModelState::Leaving {
                    self.state = ModelState::Gone;
                }
            }
            0xFC => {}
            0xFD => self.state = ModelState::Gone,
            _ => self.faults.push(format!("CRASH: low-level id {id:#04x}")),
        }
    }

    /// 0x3F4720.
    fn message(&mut self, message: Parsed) {
        let Parsed {
            id,
            class,
            x,
            part,
            payload,
            ..
        } = message;
        match (class, part) {
            (Class::Plain, _) => self.delivered.push((id, payload)),
            (Class::Unique, None) => {
                if !self.unique.contains(x) {
                    self.unique.record(x);
                    self.delivered.push((id, payload));
                }
            }
            (Class::Unique, Some(part)) => self.unique_part(id, x, part, payload),
            (Class::Ordered, None) => {
                if not_newer(self.order_last, x) || self.order_waiting.contains_key(&x) {
                    return;
                }
                self.order_waiting.insert(x, (id, payload));
                self.deliver_in_order();
            }
            (Class::Ordered, Some(part)) => self.ordered_part(id, x, part, payload),
        }
    }

    /// `process_multipacket_unique_message` 0x3F4DA0.
    fn unique_part(&mut self, id: u8, x: u8, part: (u8, Option<u8>), data: Vec<u8>) {
        if self.unique.contains(x) {
            return;
        }
        if newer(x, self.unique_newest_part) {
            self.unique_newest_part = x;
            // Assemblies now outside the window are removed.
            let expired: Vec<u8> = self
                .unique_parts
                .keys()
                .copied()
                .filter(|key| !not_newer(x, *key))
                .collect();
            for key in expired {
                self.unique_parts.remove(&key);
                self.faults.push(format!(
                    "removing expired unique multipacket message: {key}, but {x}"
                ));
            }
        }
        let assembly = self.unique_parts.entry(x).or_insert_with(|| Assembly {
            id,
            count: 0xFF,
            parts: BTreeMap::new(),
        });
        if let Some(whole) = add_part(assembly, id, part, data, &mut self.faults) {
            self.unique_parts.remove(&x);
            self.unique.record(x);
            self.delivered.push((id, whole));
        }
    }

    /// `process_multipacket_reliable_unique_ordered_message` 0x3F4A50.
    fn ordered_part(&mut self, id: u8, x: u8, part: (u8, Option<u8>), data: Vec<u8>) {
        if not_newer(self.order_last, x) || self.order_waiting.contains_key(&x) {
            return;
        }
        if newer(x, self.order_newest_part) {
            self.order_newest_part = x;
            if !self.order_parts.contains_key(&x) {
                // The assembly after x in key order, else the first one.
                let other = self
                    .order_parts
                    .range(x..)
                    .next()
                    .or_else(|| self.order_parts.iter().next())
                    .map(|(key, _)| *key);
                if let Some(key) = other
                    && !not_newer(x, key)
                {
                    self.faults.push(format!(
                        "ordered message has been EXPIRED: {key}, but {x} (disconnect)"
                    ));
                    self.state = ModelState::Gone;
                    return;
                }
            }
        }
        let assembly = self.order_parts.entry(x).or_insert_with(|| Assembly {
            id,
            count: 0xFF,
            parts: BTreeMap::new(),
        });
        if let Some(whole) = add_part(assembly, id, part, data, &mut self.faults) {
            self.order_parts.remove(&x);
            self.order_waiting.insert(x, (id, whole));
            self.deliver_in_order();
        }
    }

    /// The loop at 0x3F51F7: deliver while the next order id is waiting.
    fn deliver_in_order(&mut self) {
        loop {
            let next = self.order_last.wrapping_add(1);
            let Some((id, payload)) = self.order_waiting.remove(&next) else {
                break;
            };
            self.order_last = next;
            self.delivered.push((id, payload));
        }
    }

    // ------------------------------------------------------------ sending

    /// Queue a client message (`enqueue` 0x3FD060): split if it does not
    /// fit, the unique counter for a unique id.
    pub fn send(&mut self, id: u8, payload: &[u8]) {
        let info = self
            .table
            .info(Direction::ToServer, id)
            .expect("a client id");
        let variable = matches!(info.length, Length::Variable { .. });
        let x = (info.class != Class::Plain).then(|| {
            self.next_unique = self.next_unique.wrapping_add(1);
            self.next_unique.wrapping_sub(1)
        });
        let packet = |bytes: Vec<u8>, header: usize| Packet {
            bytes,
            header,
            variable,
            lifetime: info.lifetime,
            resend: info.resend,
            expires: None,
            seqs: VecDeque::new(),
        };
        // Room for the most tail bytes the model may add.
        let tail = ack::MAX_TAIL;
        if payload.len() + 4 + usize::from(x.is_some()) + tail <= MAX_DATAGRAM {
            let mut bytes = vec![id];
            bytes.extend(x);
            let header = bytes.len();
            bytes.extend(payload);
            self.queue.push_back(packet(bytes, header));
            return;
        }
        let x = x.expect("only unique client ids are split");
        let (first_data, part_data) = (500 - tail, 501 - tail);
        let count = 1 + (payload.len() - first_data).div_ceil(part_data);
        let (first, mut rest) = payload.split_at(first_data);
        let mut bytes = vec![0xFF, id, x, 0, count as u8];
        bytes.extend(first);
        self.queue.push_back(packet(bytes, 5));
        for index in 1..count {
            let (data, more) = rest.split_at(rest.len().min(part_data));
            rest = more;
            let mut bytes = vec![0xFF, id, x, index as u8];
            bytes.extend(data);
            self.queue.push_back(packet(bytes, 4));
        }
    }

    /// Ask to leave (`disconnect` 0x3FE000): `FA` until the confirm.
    pub fn leave(&mut self) {
        if self.state == ModelState::Connected {
            self.state = ModelState::Leaving;
        }
    }

    /// `send_queued_packets` 0x3FBF70.
    pub fn poll(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let mut datagrams = Vec::new();
        let idle = self.table.client_idle();
        let quiet = self
            .last_send
            .is_none_or(|sent| now.saturating_duration_since(sent) >= idle);
        match self.state {
            ModelState::Gone => return datagrams,
            ModelState::Connected => {
                if self
                    .last_receive
                    .is_some_and(|at| now.saturating_duration_since(at) >= Duration::from_secs(5))
                {
                    self.faults.push("disconnected by timeout".into());
                    self.state = ModelState::Gone;
                    return datagrams;
                }
            }
            ModelState::Leaving => {
                if quiet && self.queue.is_empty() {
                    self.low_level.push_back(0xFA);
                }
            }
            ModelState::Confirming { since } => {
                if now.saturating_duration_since(since) >= Duration::from_millis(500) {
                    self.state = ModelState::Gone;
                    return datagrams;
                }
                if quiet && self.low_level.is_empty() {
                    self.low_level.push_back(0xFB);
                }
            }
        }
        // Expire, then requeue what is due.
        self.unacked
            .retain(|(packet, _)| packet.expires.is_none_or(|expires| now < expires));
        let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.unacked)
            .into_iter()
            .partition(|(_, at)| *at <= now);
        self.unacked = waiting;
        for (packet, _) in due {
            self.queue.push_back(packet);
        }

        loop {
            // The send window: the next seq must be 1..=127 past A.
            let ahead = self
                .last_sent_seq
                .wrapping_add(1)
                .wrapping_sub(self.newest_acked);
            if !(1..=127).contains(&ahead) {
                break;
            }
            let tail = self
                .tail
                .unwrap_or_else(|| [17, 2, 0, 5, 1, 9, 3, 17][self.sent_count as usize % 8]);
            if let Some(id) = self.low_level.pop_front() {
                datagrams.push(self.datagram(false, &[id], tail));
                continue;
            }
            let Some(first) = self.queue.pop_front() else {
                break;
            };
            let mut packets = vec![first];
            if self.join {
                let room = MAX_DATAGRAM - 3 - tail;
                let mut used = multi_len(&packets[0]);
                while let Some(next) = self.queue.front() {
                    if used + multi_len(next) > room || packets.len() == 252 {
                        break;
                    }
                    used += multi_len(next);
                    packets.push(self.queue.pop_front().unwrap());
                }
            }
            let multi = packets.len() > 1;
            let mut body = Vec::new();
            for packet in &packets {
                if multi && packet.variable {
                    body.extend(&packet.bytes[..packet.header]);
                    let len = packet.bytes.len() - packet.header;
                    if len <= 0xFE {
                        body.push(len as u8);
                    } else {
                        body.push(0xFF);
                        body.extend((len as u16).to_le_bytes());
                    }
                    body.extend(&packet.bytes[packet.header..]);
                } else {
                    body.extend(&packet.bytes);
                }
            }
            datagrams.push(self.datagram(multi, &body, tail));
            let seq = self.last_sent_seq;
            for mut packet in packets {
                let lifetime = match packet.lifetime {
                    Lifetime::Once => continue,
                    Lifetime::For(lifetime) => Some(lifetime),
                    Lifetime::Forever => None,
                };
                if packet.expires.is_none() {
                    packet.expires = lifetime.map(|lifetime| now + lifetime);
                }
                if packet.seqs.len() == 8 {
                    packet.seqs.pop_front();
                }
                packet.seqs.push_back(seq);
                let at = now + packet.resend;
                self.unacked.push((packet, at));
            }
        }
        if datagrams.is_empty() && quiet {
            let ahead = self
                .last_sent_seq
                .wrapping_add(1)
                .wrapping_sub(self.newest_acked);
            if (1..=127).contains(&ahead) {
                datagrams.push(self.datagram(false, &[0xFC], self.tail.unwrap_or(17)));
            }
        }
        if !datagrams.is_empty() {
            self.last_send = Some(now);
        }
        datagrams
    }

    /// `fill_packet_header` 0x3FB390.
    fn datagram(&mut self, multi: bool, body: &[u8], tail: usize) -> Vec<u8> {
        let tail = tail.min(MAX_DATAGRAM - 3 - body.len());
        self.last_sent_seq = self.last_sent_seq.wrapping_add(1);
        self.sent_count += 1;
        let (ack0, tail_bytes) = ack::encode(self.received.bits | 1 << 127, tail, multi);
        let mut datagram = vec![self.last_sent_seq, self.received.newest, ack0];
        datagram.extend(body);
        datagram.extend(tail_bytes);
        assert!(datagram.len() <= MAX_DATAGRAM);
        datagram
    }
}

fn multi_len(packet: &Packet) -> usize {
    let payload = packet.bytes.len() - packet.header;
    let prefix = match (packet.variable, payload) {
        (false, _) => 0,
        (true, ..=0xFE) => 1,
        (true, _) => 3,
    };
    packet.bytes.len() + prefix
}

/// Insert a part (duplicates dropped); the whole message when the number of
/// parts reaches the count (0x3F4D64, 0x3F5100).
fn add_part(
    assembly: &mut Assembly,
    id: u8,
    part: (u8, Option<u8>),
    data: Vec<u8>,
    faults: &mut Vec<String>,
) -> Option<Vec<u8>> {
    if assembly.id != id {
        faults.push(format!(
            "a part of id {id:#04x} joins a message of id {:#04x}",
            assembly.id
        ));
    }
    if assembly.parts.contains_key(&part.0) {
        return None;
    }
    if let Some(count) = part.1 {
        assembly.count = count;
    }
    assembly.parts.insert(part.0, data);
    if assembly.parts.len() < usize::from(assembly.count) {
        return None;
    }
    // Complete by count: the indexes must be 0..count for it to be whole.
    let indexes: Vec<u8> = assembly.parts.keys().copied().collect();
    if indexes != (0..assembly.count).collect::<Vec<u8>>() {
        faults.push(format!(
            "a split message completes with parts {indexes:?} of {}",
            assembly.count
        ));
    }
    Some(assembly.parts.values().flatten().copied().collect())
}

/// The header reader 0x3FA0A0: `(seq, rseq, ack bits, count, multi, tail)`.
fn parse_header(datagram: &[u8]) -> Result<(u8, u8, u128, u32, bool, usize), String> {
    if datagram.len() < 3 {
        return Err("a datagram shorter than its header".into());
    }
    let mut bits = 1u128 << 127;
    let mut count = 1u32;
    let mut tail = 0usize;
    let mut byte = datagram[2];
    while byte & 0x80 != 0 {
        if tail == 17 {
            return Err("more than 17 tail ack bytes".into());
        }
        bits |= u128::from(byte & 0x7F) << 1 << (0x78 - count);
        count += 7;
        tail += 1;
        // The tail may not run into the first message byte.
        if datagram.len() - tail < 4 {
            return Err("the ack tail overlaps the message area".into());
        }
        byte = datagram[datagram.len() - tail];
    }
    bits |= u128::from(byte >> 1) << (0x7A - count);
    count += 6;
    Ok((
        datagram[0],
        datagram[1],
        bits,
        count.min(127),
        byte & 1 != 0,
        tail,
    ))
}

/// The parse loop 0x3FAF40 over `try_parse_single_message` 0x3FA240.
fn parse_messages(
    table: Table,
    mask_bytes: usize,
    area: &[u8],
    multi: bool,
) -> Result<Vec<Parsed>, String> {
    if area.is_empty() {
        return Err("an empty message area (header-only datagram)".into());
    }
    let mut messages = Vec::new();
    let mut at = 0;
    while at < area.len() {
        if messages.len() == 252 {
            return Err("ASSERT: more than 252 messages".into());
        }
        let mut id = area[at];
        at += 1;
        let multipart = id == 0xFF;
        if multipart {
            if area.len() - at < 2 {
                return Err("a multipart header cut short".into());
            }
            id = area[at];
            at += 1;
        } else if (0xFA..=0xFE).contains(&id) {
            if at != area.len() {
                return Err("non-empty low level packet".into());
            }
            messages.push(Parsed {
                id,
                low_level: true,
                class: Class::Plain,
                x: 0,
                part: None,
                payload: Vec::new(),
            });
            break;
        }
        let info = match table.lookup(Direction::ToClient, id) {
            Lookup::Info(info) => info,
            Lookup::OutOfRange => return Err(format!("invalid message type {id:#04x}")),
            Lookup::Null => return Err(format!("CRASH: jump to 0 for id {id:#04x}")),
        };
        let byte = |at: &mut usize, what: &str| -> Result<u8, String> {
            let byte = *area
                .get(*at)
                .ok_or_else(|| format!("id {id:#04x}: no {what} byte"))?;
            *at += 1;
            Ok(byte)
        };
        let x = match info.class {
            Class::Plain => 0,
            Class::Unique | Class::Ordered => byte(&mut at, "x")?,
        };
        let part = if multipart {
            if info.class == Class::Plain {
                return Err(format!("a part of the plain id {id:#04x}"));
            }
            let index = byte(&mut at, "part")?;
            let count = if index == 0 {
                Some(byte(&mut at, "count")?)
            } else {
                None
            };
            Some((index, count))
        } else {
            None
        };
        let first = part.is_none_or(|(index, _)| index == 0);
        let remaining = area.len() - at;
        // get_message_length: 0xFFFF = variable, with its `ok` flag.
        let fixed = match info.length {
            Length::Fixed(len) => Some(len),
            Length::Variable {
                first_min,
                part_min,
            } => {
                if remaining < if first { first_min } else { part_min } {
                    return Err(format!("id {id:#04x}: the length getter failed"));
                }
                None
            }
            Length::PlayerInputs { head } => {
                if remaining < head + mask_bytes + 16 {
                    return Err(format!("id {id:#04x}: multiple players inputs too short"));
                }
                let players: usize = area[at + head..at + head + mask_bytes]
                    .iter()
                    .map(|byte| byte.count_ones() as usize)
                    .sum();
                if remaining - head - mask_bytes < 8 * players {
                    return Err(format!("id {id:#04x}: multiple players inputs cut short"));
                }
                Some(head + mask_bytes + 8 * players)
            }
        };
        let len = match (fixed, multi) {
            (Some(len), false) => {
                if len != remaining {
                    return Err(format!(
                        "id {id:#04x}: single message packet, but extra space is found \
                         ({remaining} bytes for {len})"
                    ));
                }
                len
            }
            // No bounds check in the client: reading past the area.
            (Some(len), true) => {
                if len > remaining {
                    return Err(format!("id {id:#04x}: a fixed message runs past the end"));
                }
                len
            }
            (None, false) => remaining,
            (None, true) => {
                let mut len = usize::from(byte(&mut at, "length")?);
                if len == 0xFF {
                    len = usize::from(byte(&mut at, "length")?);
                    len |= usize::from(byte(&mut at, "length")?) << 8;
                    if len <= 0xFE {
                        return Err("3 bytes used, but one byte must be used instead".into());
                    }
                }
                if len > area.len() - at {
                    return Err(format!("id {id:#04x}: the length runs past the end"));
                }
                len
            }
        };
        messages.push(Parsed {
            id,
            low_level: false,
            class: info.class,
            x,
            part,
            payload: area[at..at + len].to_vec(),
        });
        at += len;
        if !multi && at != area.len() {
            return Err("a single datagram with more than one message".into());
        }
    }
    Ok(messages)
}
