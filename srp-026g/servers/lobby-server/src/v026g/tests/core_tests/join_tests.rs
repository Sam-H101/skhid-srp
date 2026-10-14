//! JOIN, the button of a match in the "matches now" window (`join`): what
//! a JOIN request is answered, the place it holds, several players
//! pressing it at once, and the Play that takes the place. The rows' flag
//! is `matches_now_tests`.

use survarium::v026g::lobby::{JOIN_KEY_MAX, JOIN_START};

use super::matches_now_tests::{answered, growing, joins_now, larger_and_smaller, match_of};
use super::*;
use crate::v026g::join::{HOLD, JOIN_REFUSED, Listing};

/// The asker presses JOIN of the match with `key`; the one row of the one
/// `91` that answers it, which must be at the JOIN start. Handled at once,
/// as `joins_now` asks.
fn join(world: &mut World, index: usize, key: u32) -> String {
    let answers = answered(world, index, JOIN_START + key, 1);
    let [rows] = &answers[..] else {
        panic!("not one answer: {answers:?}");
    };
    let [(name, place)] = &rows[..] else {
        panic!("not one row: {rows:?}");
    };
    assert_eq!(*place, JOIN_START);
    name.clone()
}

/// What each place of the match counts for beside being there:
/// `larger_side` for the first place of the side with more players, 0 for
/// every other.
fn larger_side_counts(world: &mut World, id: u32, larger_side: i32) {
    let (larger, _) = larger_and_smaller(world, id);
    let running = world.core.matches.get_mut(&id).unwrap();
    let mut first = true;
    running.strengths = running
        .seats
        .iter()
        .map(|&(_, team)| {
            if team == larger && std::mem::take(&mut first) {
                larger_side
            } else {
                0
            }
        })
        .collect();
}

/// A lobby whose keys start at 1, with a match of two against one that
/// takes players (`P1` to `P3`), alike but for the first of the two, who
/// counts for `larger_side`: its ticket's id and the lobby's id.
fn two_against_one(world: &mut World, larger_side: i32) -> (u32, u32) {
    world.core.listing = Listing::new(0);
    let (ticket, id, _) = match_of(world, &["P1", "P2", "P3"]);
    larger_side_counts(world, id, larger_side);
    (ticket, id)
}

/// Two against one that count alike: the one counts for `player` more.
fn even_by_value(world: &mut World, id: u32, smaller: u8, player: i32) {
    let running = world.core.matches.get_mut(&id).unwrap();
    running.strengths = running
        .seats
        .iter()
        .map(|seat| if seat.1 == smaller { player } else { 0 })
        .collect();
}

/// The client was sent to `ticket` on `team`, and nothing refused it.
fn went_to(world: &mut World, index: usize, ticket: u32, team: u8) {
    let decoded = world.take(index);
    assert_eq!(denial(&decoded, 0x2F), None);
    assert_eq!(sent_to_match(&decoded), Some((ticket, team)), "{decoded:?}");
}

#[test]
fn a_join_is_answered_ok_full_gone_or_busy() {
    let mut world = World::new(Duration::ZERO);
    // Two against one, all alike: the one is a player short.
    let (ticket, id) = two_against_one(&mut world, 0);
    let carol = growing(&mut world, "Carol");
    let erin = growing(&mut world, "Erin");
    let standard = world.connect("Standard");
    let player = world.core.settings.balance.player_value();

    // A key no match has: gone, also the last there can be.
    assert_eq!(join(&mut world, carol, 2), "J1;gone;2");
    assert_eq!(
        join(&mut world, carol, JOIN_KEY_MAX),
        format!("J1;gone;{JOIN_KEY_MAX}")
    );
    // A game that cannot join a match being played: busy.
    assert_eq!(join(&mut world, standard, 1), "J1;busy;1");
    // The match server admits nobody now: gone.
    world.host.admitting.locked().remove(&ticket);
    assert_eq!(join(&mut world, carol, 1), "J1;gone;1");
    world.host.admitting.locked().insert(ticket);
    // Even by value (the one counts for a player more): full.
    let (_, smaller) = larger_and_smaller(&world, id);
    even_by_value(&mut world, id, smaller, player);
    assert_eq!(join(&mut world, carol, 1), "J1;full;1");
    assert!(world.core.holds.is_empty());
    larger_side_counts(&mut world, id, 0);

    // Uneven by value: ok, and a place is held for Carol for ten seconds.
    let now = world.now;
    assert_eq!(join(&mut world, carol, 1), "J1;ok;1");
    let hold = world.core.holds[&world.ids[carol].0];
    assert_eq!((hold.match_id, hold.key), (ticket, 1));
    assert_eq!(hold.until, now + HOLD);
    assert_eq!(HOLD, Duration::from_secs(10));
    // Holding it, Carol still sees her row on; another player does not,
    // and its JOIN is full: the place is hers.
    assert_eq!(joins_now(&mut world, carol), [(true, 1)]);
    assert_eq!(joins_now(&mut world, erin), [(false, 1)]);
    assert_eq!(join(&mut world, erin, 1), "J1;full;1");
    // Nobody was put anywhere by it.
    assert_eq!(world.host.made.locked()[0].1.players.len(), 3);
    assert_eq!(world.store.matchmaking().queued_players(), 0);
    // Carol's JOIN of another key gives her place up, granted or not.
    assert_eq!(join(&mut world, carol, 7), "J1;gone;7");
    assert!(world.core.holds.is_empty());
    assert_eq!(join(&mut world, erin, 1), "J1;ok;1");
    // Carol, while Erin holds it: full; her own again after Erin's
    // second JOIN of the same match (which replaces Erin's place).
    assert_eq!(join(&mut world, carol, 1), "J1;full;1");
    assert_eq!(join(&mut world, erin, 1), "J1;ok;1");
    assert_eq!(world.core.holds.len(), 1);
    world.settle();
    world.assert_clean();
}

/// Several players press JOIN of one match at once: the lobby reads one
/// message at a time, so one gets the place and the others are told so.
/// The match server takes one joiner at a time (a joiner on its way in
/// closes its admission), so while that place is held nobody else gets
/// one; once the first is in and the match admits again, the next is
/// asked anew: taken where the match is still uneven, not where the first
/// made it even.
#[test]
fn joins_pressed_at_once_give_one_place_and_the_next_is_decided_anew() {
    // (What the first of the two counts for, the askers, whether the match
    // takes the second asker once the first is in.)
    for (larger_side, askers, room_for_two) in [
        // Two against one, all alike: the first asker makes it two against
        // two that count alike.
        (0, 2, false),
        // The two count for fifty gear levels more: room for two players.
        (5_000, 3, true),
    ] {
        let mut world = World::new(Duration::ZERO);
        let (ticket, id) = two_against_one(&mut world, larger_side);
        let (_, smaller) = larger_and_smaller(&world, id);
        let names = ["Carol", "Dave", "Erin"];
        let askers: Vec<usize> = names[..askers]
            .iter()
            .map(|name| growing(&mut world, name))
            .collect();
        for &asker in &askers {
            assert_eq!(joins_now(&mut world, asker), [(true, 1)]);
        }
        world.settle();
        for &asker in &askers {
            world.take(asker);
        }
        // All in the same round.
        for &asker in &askers {
            let (op, body) = request::ranking(JOIN_START + 1, 1);
            world.client(asker).send(op, &body);
        }
        world.settle();
        let answers: Vec<String> = askers
            .iter()
            .map(|&asker| {
                let decoded = world.take(asker);
                let [Decoded::EloList(rows)] = &decoded[..] else {
                    panic!("{decoded:?}");
                };
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].1, JOIN_START);
                rows[0].0.clone()
            })
            .collect();
        let mut expected = vec!["J1;full;1".to_owned(); askers.len()];
        expected[0] = "J1;ok;1".to_owned();
        assert_eq!(answers, expected);
        assert_eq!(world.core.holds.len(), 1);

        // The first presses Play: into that match at once, on the side
        // that was short.
        world.ask(askers[0], request::play(1));
        went_to(&mut world, askers[0], ticket, smaller);
        assert!(world.core.holds.is_empty());
        // While it is on its way in the match server admits nobody.
        assert_eq!(join(&mut world, askers[1], 1), "J1;gone;1");
        // It is in; the match admits again.
        world.host.admitting.locked().insert(ticket);
        let second = join(&mut world, askers[1], 1);
        if room_for_two {
            assert_eq!(second, "J1;ok;1");
            world.ask(askers[1], request::play(1));
            went_to(&mut world, askers[1], ticket, smaller);
            assert_eq!(world.host.made.locked()[0].1.players.len(), 5);
        } else {
            assert_eq!(second, "J1;full;1");
            assert_eq!(world.host.made.locked()[0].1.players.len(), 4);
        }
        assert_eq!(world.host.made.locked().len(), 1);
        assert_eq!(world.store.matchmaking().queued_players(), 0);
        world.assert_clean();
    }
}

/// The Play of a player with a held place: into that match at once, by
/// the queue's own late-join steps, with what the lobby keeps of the match
/// and the log's "late join" line as for any late joiner.
#[test]
fn play_with_a_held_place_goes_into_that_match_at_once_and_only_there() {
    let mut world = World::new(Duration::ZERO);
    let (ticket, id) = two_against_one(&mut world, 0);
    let (_, smaller) = larger_and_smaller(&world, id);
    // Another match of growing rosters that takes players, and that a
    // player of the queue would be put into rather than the first: more
    // of it is left to play.
    world.host.admitting.locked().remove(&ticket);
    let (other, _, _) = match_of(&mut world, &["Q1", "Q2"]);
    world.host.admitting.locked().insert(ticket);
    world
        .host
        .remaining
        .locked()
        .insert(other, Duration::from_secs(1500));
    let carol = growing(&mut world, "Carol");
    assert_eq!(join(&mut world, carol, 1), "J1;ok;1");
    world.ask(carol, request::play(1));
    went_to(&mut world, carol, ticket, smaller);
    // The match server was asked for that match alone, and made no new
    // one; the queue never had her.
    let made = world.host.made.locked().clone();
    assert_eq!(made.len(), 2);
    assert_eq!(made[0].1.players.len(), 4);
    assert_eq!(made[1].1.players.len(), 2);
    let joined = &made[0].1.players[3];
    assert_eq!((joined.account_id, joined.session_id), world.ids[carol]);
    assert_eq!((joined.profile.team, joined.profile.squad), (smaller, 0));
    assert_eq!(world.store.matchmaking().queued_players(), 0);
    assert_eq!(
        world.store.matchmaking().seat(world.ids[carol].1),
        Some((id, 3))
    );
    // The lobby's roster of the match has her, as any late joiner.
    let running = &world.core.matches[&id];
    assert_eq!(running.seats[3], (world.ids[carol].1, smaller));
    assert_eq!(running.strengths.len(), 4);
    assert_eq!(running.names[3], "Carol");
    let addr = world.clients[carol].0;
    assert_eq!(world.core.player(addr).unwrap().in_match, Some(id));
    assert!(world.core.holds.is_empty());
    world.assert_clean();
}

/// The place was held, and by the Play the match no longer takes the
/// player: refused with the game's text, and the player is not queued
/// (not into another match, not into a new one). The hold is used up: the
/// next Play is an ordinary one.
#[test]
fn play_with_a_held_place_the_match_no_longer_takes_is_refused_and_not_queued() {
    let mut world = World::new(Duration::ZERO);
    let (ticket, id) = two_against_one(&mut world, 0);
    let (_, smaller) = larger_and_smaller(&world, id);
    let carol = growing(&mut world, "Carol");
    let refused = |world: &mut World, carol: usize| {
        world.take(carol);
        world.ask(carol, request::play(1));
        let decoded = world.take(carol);
        assert_eq!(denial(&decoded, 0x2F).as_deref(), Some(JOIN_REFUSED));
        assert_eq!(sent_to_match(&decoded), None);
        assert!(
            !decoded
                .iter()
                .any(|message| matches!(message, Decoded::ClientState { .. })),
            "{decoded:?}"
        );
        assert_eq!(world.store.matchmaking().queued_players(), 0);
        assert!(world.core.holds.is_empty());
        let addr = world.clients[carol].0;
        let player = world.core.player(addr).unwrap();
        assert!(player.queued_since.is_none() && player.in_match.is_none());
    };
    assert_eq!(JOIN_REFUSED, "st_cant_change_ready_state");

    // The match stopped admitting.
    assert_eq!(join(&mut world, carol, 1), "J1;ok;1");
    world.host.admitting.locked().remove(&ticket);
    refused(&mut world, carol);
    world.host.admitting.locked().insert(ticket);
    // The sides came to count alike (a lead, a player who came back).
    assert_eq!(join(&mut world, carol, 1), "J1;ok;1");
    let player = world.core.settings.balance.player_value();
    even_by_value(&mut world, id, smaller, player);
    refused(&mut world, carol);
    larger_side_counts(&mut world, id, 0);
    // The match server would not take her after all.
    assert_eq!(join(&mut world, carol, 1), "J1;ok;1");
    world.host.rejecting.locked().insert(ticket);
    refused(&mut world, carol);
    world.host.rejecting.locked().remove(&ticket);
    assert_eq!(world.host.made.locked()[0].1.players.len(), 3);
    // The match is over.
    assert_eq!(join(&mut world, carol, 1), "J1;ok;1");
    world.host.open.locked().remove(&ticket);
    refused(&mut world, carol);
    assert!(world.core.matches.is_empty());
    assert_eq!(world.host.made.locked().len(), 1);

    // With no place held, Play is the ordinary one: Carol waits in the
    // queue.
    world.ask(carol, request::play(1));
    let decoded = world.take(carol);
    assert_eq!(denial(&decoded, 0x2F), None);
    assert!(matches!(
        queue_state(&decoded),
        Some(ClientState::InQueue { .. })
    ));
    assert_eq!(world.store.matchmaking().queued_players(), 1);
    world.assert_clean();
}

/// A held place lasts ten seconds. While it lasts the queue's late joins
/// do not give it away; after, it is anyone's, and its player's Play is an
/// ordinary one.
#[test]
fn a_held_place_is_kept_from_the_queue_for_ten_seconds() {
    let mut world = World::new(Duration::ZERO);
    let (ticket, id) = two_against_one(&mut world, 0);
    let (_, smaller) = larger_and_smaller(&world, id);
    let carol = growing(&mut world, "Carol");
    let dave = growing(&mut world, "Dave");
    let erin = growing(&mut world, "Erin");
    assert_eq!(join(&mut world, carol, 1), "J1;ok;1");
    // Dave presses the ordinary Play: the queue would put him into the
    // match at once (it is a player short), but the place is Carol's.
    world.ask(dave, request::play(1));
    world.advance(Duration::from_secs(5));
    assert_eq!(sent_to_match(&world.take(dave)), None);
    assert_eq!(world.store.matchmaking().queued_players(), 1);
    assert_eq!(world.host.made.locked()[0].1.players.len(), 3);
    assert_eq!(join(&mut world, erin, 1), "J1;full;1");
    // Ten seconds on, Carol has not pressed Play: the place is free, and
    // the queue gives it to Dave.
    world.advance(HOLD);
    assert!(world.core.holds.is_empty());
    world.advance(Duration::from_secs(1));
    assert_eq!(sent_to_match(&world.take(dave)), Some((ticket, smaller)));
    assert_eq!(world.store.matchmaking().queued_players(), 0);
    // Carol's Play now is the ordinary one: she waits in the queue (the
    // match server admits nobody while Dave is on his way in).
    world.ask(carol, request::play(1));
    let decoded = world.take(carol);
    assert_eq!(denial(&decoded, 0x2F), None);
    assert_eq!(sent_to_match(&decoded), None);
    assert!(matches!(
        queue_state(&decoded),
        Some(ClientState::InQueue { .. })
    ));
    world.assert_clean();
}

/// A held place that runs out while nobody asks: the next asker gets it.
#[test]
fn a_held_place_that_ran_out_is_the_next_askers() {
    let mut world = World::new(Duration::ZERO);
    let (ticket, id) = two_against_one(&mut world, 0);
    let (_, smaller) = larger_and_smaller(&world, id);
    let carol = growing(&mut world, "Carol");
    let dave = growing(&mut world, "Dave");
    assert_eq!(join(&mut world, carol, 1), "J1;ok;1");
    assert_eq!(join(&mut world, dave, 1), "J1;full;1");
    world.now += HOLD;
    assert_eq!(joins_now(&mut world, dave), [(true, 1)]);
    assert_eq!(join(&mut world, dave, 1), "J1;ok;1");
    world.ask(dave, request::play(1));
    went_to(&mut world, dave, ticket, smaller);
    world.assert_clean();
}

/// Play of a player who never pressed JOIN, while others hold places: as
/// it always was, for a stock game and for one of growing rosters.
#[test]
fn a_play_with_no_held_place_is_the_ordinary_one() {
    let mut world = World::new(Duration::ZERO);
    let (ticket, id) = two_against_one(&mut world, 0);
    let (_, smaller) = larger_and_smaller(&world, id);
    // A player alone goes into an even match at once (no wait for
    // another): the late joins' own rule, set short for the test.
    world.core.settings.late_join_patience = Duration::ZERO;
    let carol = growing(&mut world, "Carol");
    let stock = world.connect("Stock");
    let dave = growing(&mut world, "Dave");
    assert_eq!(join(&mut world, carol, 1), "J1;ok;1");
    // A stock game: the standard queue.
    world.ask(stock, request::play(1));
    let decoded = world.take(stock);
    assert!(matches!(
        queue_state(&decoded),
        Some(ClientState::InQueue { .. })
    ));
    assert_eq!(world.store.matchmaking().queued_players(), 1);
    // Carol's place is used; Dave, alone in the growing queue, then goes
    // the ordinary way: into the match once it admits again, though it is
    // even by then.
    world.ask(carol, request::play(1));
    went_to(&mut world, carol, ticket, smaller);
    world.ask(dave, request::play(1));
    assert_eq!(sent_to_match(&world.take(dave)), None);
    world.host.admitting.locked().insert(ticket);
    world.advance(Duration::from_secs(2));
    assert_eq!(
        sent_to_match(&world.take(dave)).map(|sent| sent.0),
        Some(ticket)
    );
    assert_eq!(world.host.made.locked()[0].1.players.len(), 5);
    world.assert_clean();
}
