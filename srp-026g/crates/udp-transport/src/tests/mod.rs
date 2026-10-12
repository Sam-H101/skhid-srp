//! Tests of the whole transport: byte-exact datagrams (`bytes`), and
//! exchanges with a model of the client over a lossy link (`exchange`).

mod bytes;
mod client_model;
mod exchange;

use std::time::Instant;

use crate::ack;
use crate::{Event, Table, Transport};

/// A client datagram: `seq`, acking server `rseq` and the seqs in `also`
/// (each older than `rseq`), then `body`. 17 tail ack bytes.
fn client(seq: u8, rseq: u8, also: &[u8], multi: bool, body: &[u8]) -> Vec<u8> {
    let mut bits = 1u128 << 127;
    for seq in also {
        bits |= 1 << (127 - u32::from(rseq.wrapping_sub(*seq)));
    }
    let (ack0, tail) = ack::encode(bits, ack::MAX_TAIL, multi);
    let mut datagram = vec![seq, rseq, ack0];
    datagram.extend(body);
    datagram.extend(tail);
    datagram
}

/// A client keep-alive.
fn flow(seq: u8, rseq: u8, also: &[u8]) -> Vec<u8> {
    client(seq, rseq, also, false, &[0xFC])
}

/// A match transport after the client's first datagram (D8, seq 0).
fn connect_match() -> (Transport, Instant) {
    let now = Instant::now();
    let mut transport = Transport::new(Table::Match, now);
    let events = transport.receive(&[0x00, 0xFF, 0x00, 0xD8, 1, 2, 3, 4, 0x00], now);
    assert_eq!(
        events,
        [Event::Message {
            id: 0xD8,
            payload: vec![1, 2, 3, 4, 0]
        }]
    );
    (transport, now)
}

/// A lobby transport after the client's sign-in (2E, seq 0, unique id 0).
fn connect_lobby() -> (Transport, Instant) {
    let now = Instant::now();
    let mut transport = Transport::new(Table::Lobby, now);
    let events = transport.receive(&[0x00, 0xFF, 0x00, 0x2E, 0x00, 1, 2, 3, 4, 0x01], now);
    assert_eq!(
        events,
        [Event::Message {
            id: 0x2E,
            payload: vec![1, 2, 3, 4, 1]
        }]
    );
    (transport, now)
}
