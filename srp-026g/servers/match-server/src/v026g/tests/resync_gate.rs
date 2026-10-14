//! A resynced client's world gate (match.md section 30) in a busy match:
//! bounded in time and cheap per poll.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use survarium::v026g::match_msgs::{ServerMessage, id};
use udp_transport::wire;
use udp_transport::{Direction, Table};

use super::{Bench, MAP_BATTERIES};
use crate::v026g::Settings;

/// 5 ms pass; every 50 ms each client in `playing` makes an input.
fn step(bench: &mut Bench, playing: &[usize]) {
    if bench.ms.is_multiple_of(50) {
        for place in playing {
            if !bench.clients[*place].closed {
                bench.clients[*place].input(20, 0x21);
            }
        }
    }
    bench.step(5);
}

fn play(bench: &mut Bench, playing: &[usize], limit_ms: u64, done: impl Fn(&Bench) -> bool) -> bool {
    let end = bench.ms + limit_ms;
    while bench.ms < end && !done(bench) {
        step(bench, playing);
    }
    done(bench)
}

/// A lossy line: about `percent` % of the datagrams to the client lost,
/// the same ones on every run.
fn lossy(seed: u32, percent: u32) -> super::client::Lose {
    let mut state = seed | 1;
    Box::new(move |_| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state % 100 < percent
    })
}

/// What a busy resync gave.
struct Busy {
    /// Each world sent in the resync: its snapshot's time and the match
    /// clock when it went.
    worlds: Vec<(u32, u32)>,
    /// From the resync's start to its client in play again (ms).
    took: u64,
    /// The gate's entries at most, and its looks.
    most: usize,
    checks: u64,
    /// The server's polls over 10 s of match from the resync's start.
    busy: Duration,
}

/// 20 clients, each with an input every 50 ms; the last on a line of
/// `round_trip` ms that loses `loss` % of what goes to it, whose game acks
/// only the newest 6 seqs and does so with its inputs, goes its own way
/// and is resynced.
fn busy_resync(round_trip: u64, loss: u32) -> Busy {
    const PLAYERS: usize = 20;
    let far = PLAYERS - 1;
    let all: Vec<usize> = (0..PLAYERS).collect();
    let mut bench = Bench::new(PLAYERS, MAP_BATTERIES, Settings::default());
    bench.join_all();
    play(&mut bench, &all, 4_000, |_| false);
    let mut one_way = vec![0; PLAYERS];
    one_way[far] = round_trip / 2;
    bench.one_way = one_way;
    bench.clients[far].ack_depth = Some(6);
    bench.clients[far].slow_acks = true;
    bench.clients[far].lose = Some(lossy(0x9E37_79B9 ^ round_trip as u32, loss));
    // Its round trip is measured, and its clock put right.
    play(&mut bench, &all, 12_000, |_| false);
    let from = bench.clients[far].clock();
    bench.clients[far].sim.salt_from = Some((from, 7));
    assert!(
        play(&mut bench, &all, 20_000, |bench| bench.game().resyncs(far) == 1),
        "not resynced: {}",
        bench.stages()
    );
    let begun = bench.ms;
    let busy_before = bench.server_busy;
    let (_, _, checks_before) = bench.game().gate_stats(far);
    let mut worlds = Vec::new();
    let mut was = b' ';
    let mut took = None;
    while bench.ms < begun + 10_000 {
        let stage = bench.stages().as_bytes()[far];
        if stage == b's' && was != b's' {
            worlds.push((bench.game().sync_time(far).unwrap(), bench.clock()));
        }
        was = stage;
        if took.is_none()
            && bench.stages().bytes().all(|stage| stage == b'L')
            && bench.clients[far].model.syncs >= 2
        {
            took = Some(bench.ms - begun);
        }
        step(&mut bench, &all);
    }
    let busy = bench.server_busy - busy_before;
    let (_, most, checks) = bench.game().gate_stats(far);
    bench.assert_no_faults();
    Busy {
        worlds,
        took: took.unwrap_or_else(|| panic!("the resync did not end: {}", bench.stages())),
        most,
        checks: checks - checks_before,
        busy,
    }
}

fn check_busy(round_trip: u64, loss: u32) {
    let started = Instant::now();
    let busy = busy_resync(round_trip, loss);
    eprintln!(
        "20 clients, {round_trip} ms, {loss} % lost: worlds {:?}, in play after {} ms, gate \
         entries at most {}, {} looks; server polls {} ms of wall time for 10 s of match \
         (test {} ms)",
        busy.worlds,
        busy.took,
        busy.most,
        busy.checks,
        busy.busy.as_millis(),
        started.elapsed().as_millis()
    );
    assert!(!busy.worlds.is_empty());
    // Every world goes within the game's 1.2 s rings of its snapshot's
    // time (RESYNC_LOAD_BY_MS), whatever the gate still waited for.
    for (time, clock) in &busy.worlds {
        assert!(clock - time <= 1_200, "{:?}", busy.worlds);
    }
    // The resync ends, and soon.
    assert!(busy.took < 8_000, "{} ms", busy.took);
    // The gate is cheap: its entries are a replay and a round trip of
    // relays at most (no O(n^2) growth), and it is looked at every 15 ms
    // at most (10 s / 15 ms = 667), not on every poll (2000 in 10 s).
    assert!(busy.most <= 700, "{} entries", busy.most);
    assert!(busy.checks <= 10_000 / 15 + 1, "{} looks", busy.checks);
}

#[test]
fn a_resync_in_a_busy_match_on_a_lossy_line_of_100_ms_is_bounded_and_cheap() {
    check_busy(100, 8);
}

#[test]
fn a_resync_in_a_busy_match_on_a_lossy_line_of_300_ms_is_bounded_and_cheap() {
    check_busy(300, 8);
}

/// Datagrams to `place` with any of `ids` in them are lost while the flag
/// is set.
fn hold(bench: &mut Bench, place: usize, ids: &'static [u8]) -> Arc<AtomicBool> {
    let holding = Arc::new(AtomicBool::new(false));
    let held = holding.clone();
    bench.clients[place].lose = Some(Box::new(move |datagram| {
        held.load(Ordering::Relaxed)
            && wire::parse(Table::Match, Direction::ToClient, datagram, 1).is_ok_and(|parsed| {
                parsed
                    .messages
                    .iter()
                    .any(|message| ids.contains(&message.id))
            })
    }));
    holding
}

#[test]
fn a_roster_change_lost_after_the_third_redo_still_arrives() {
    let mut bench = Bench::new(4, MAP_BATTERIES, Settings::default());
    let playing = [0, 1];
    bench.join_all();
    play(&mut bench, &playing, 4_000, |_| false);
    let far = 3;
    // Its replay is never acked: the datagrams with relays to it are lost.
    let relays = hold(&mut bench, far, &[id::INPUT_RELAY, id::PLAYER_LEFT]);
    let from = bench.clients[far].clock();
    bench.clients[far].sim.salt_from = Some((from, 7));
    assert!(play(&mut bench, &playing, 20_000, |bench| {
        bench.game().resyncs(far) == 1
    }));
    // Its three redos are used up, and no newer world is given.
    bench
        .server
        .games
        .get_mut(&bench.match_id)
        .unwrap()
        .set_replay_redos(far, 3);
    relays.store(true, Ordering::Relaxed);
    for client in &mut bench.clients[..3] {
        client.no_uploads = true;
    }
    assert!(play(&mut bench, &playing, 5_000, |bench| {
        bench.game().resync_state(far) == (true, true) && bench.stages().as_bytes()[far] == b't'
    }));
    let generation = bench.game().generation(far);
    // Player 2 leaves now: its leave, a roster change past any snapshot,
    // goes to the resynced client and its copies are lost for 4 s, past
    // the replay's 3 s ack wait.
    bench.clients[2].leave();
    assert!(play(&mut bench, &playing, 2_000, |bench| {
        bench.stages().as_bytes()[2] == b'g'
    }));
    play(&mut bench, &playing, 4_000, |_| false);
    // Nothing more is done again: the world of the same generation went
    // without what it lacked, and the resync ended.
    assert_eq!(bench.game().generation(far), generation);
    assert!(!bench.game().resync_state(far).0);
    let snapshots = bench.clients[far].snapshots();
    assert_eq!(snapshots.last().map(|world| Some(world.0)), Some(generation));
    for client in &mut bench.clients[..3] {
        client.no_uploads = false;
    }
    let left = |bench: &Bench| {
        bench.clients[far]
            .got(|message| match message {
                ServerMessage::PlayerLeft { player: 2, .. } => Some(()),
                _ => None,
            })
            .len()
    };
    assert_eq!(left(&bench), 0);
    // The line comes back: the leave arrives, kept until acked, and the
    // resync ends.
    relays.store(false, Ordering::Relaxed);
    assert!(
        play(&mut bench, &playing, 10_000, |bench| {
            left(bench) >= 1 && bench.stages().as_bytes()[far] == b'L'
        }),
        "{} left {}",
        bench.stages(),
        left(&bench)
    );
    bench.assert_no_faults();
}

#[test]
fn a_replay_copy_up_to_the_world_is_forgotten_when_the_world_goes() {
    let mut bench = Bench::new(3, MAP_BATTERIES, Settings::default());
    let playing = [0, 1];
    bench.join_all();
    play(&mut bench, &playing, 4_000, |_| false);
    let far = 2;
    // Its replay is lost while held: the world goes by the gate's limit.
    let relays = hold(&mut bench, far, &[id::INPUT_RELAY]);
    let from = bench.clients[far].clock();
    bench.clients[far].sim.salt_from = Some((from, 7));
    assert!(play(&mut bench, &playing, 20_000, |bench| {
        bench.game().resyncs(far) == 1
    }));
    relays.store(true, Ordering::Relaxed);
    assert!(play(&mut bench, &playing, 10_000, |bench| {
        bench.stages().as_bytes()[far] == b's'
    }));
    // The world went with its replay not acked, within a second or so of
    // the replay ...
    let (pending, most, _) = bench.game().gate_stats(far);
    assert_eq!(pending, 0);
    assert!(most > 0);
    let time = bench.game().sync_time(far).unwrap();
    relays.store(false, Ordering::Relaxed);
    let before = bench.clients[far].taken.len();
    assert!(play(&mut bench, &playing, 10_000, |bench| {
        bench.stages().as_bytes()[far] == b'L'
    }));
    // ... and no relay up to its time reached the game after it.
    let late: Vec<u32> = bench.clients[far].taken[before..]
        .iter()
        .filter_map(|message| match message {
            ServerMessage::InputRelay { input, .. } => input.time.resolve(time + 1_000),
            _ => None,
        })
        .filter(|at| *at <= time)
        .collect();
    assert!(late.is_empty(), "{late:?} up to {time}");
    bench.assert_no_faults();
}
