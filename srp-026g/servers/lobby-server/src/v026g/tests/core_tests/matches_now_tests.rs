//! The matches being played now, for the "matches now" window of srp's
//! changed lobby movie: asked with the ranking page's own request (`3B`)
//! and a start no ranking page has, answered with a `91` whose rows carry
//! the list (`lobby::matches_now`). A stock game never asks, and what it
//! does ask is answered as ever.

use session::QueuePool;
use session::v026g::matches_now::{Phase, SCORE_AGE};
use survarium::v026g::lobby::requests::ClientMessage;
use survarium::v026g::lobby::{MATCHES_NOW_ROWS, MATCHES_START};
use survarium::v026g::match_msgs::late_join::PATCH_REVISION;

use super::*;
use crate::v026g::{LiveView, OpenMatch};

/// Team Deathmatch and batteries on Rudnya (level 1).
const RUDNYA_TDM: u8 = 117;
const RUDNYA_BATTERIES: u8 = 61;

/// A match in play the lobby did not make: four against three, 88 tickets
/// to 78 in a world four seconds old, played for 6 min 20 s.
fn playing(match_id: u32) -> OpenMatch {
    OpenMatch {
        match_id,
        map_id: RUDNYA_TDM,
        phase: Phase::Playing,
        players: [4, 3],
        standing: Some(([88, 78], Duration::from_secs(4))),
        played: Some(Duration::from_secs(6 * 60 + 20)),
    }
}

/// The window asks; the rows of the one `91` that answers it, as `(name,
/// place)`. Nothing else is sent for the request.
fn asked(world: &mut World, index: usize, count: u8) -> Vec<(String, u32)> {
    world.take(index);
    world.ask(index, request::ranking(MATCHES_START, count));
    let decoded = world.take(index);
    let [Decoded::EloList(rows)] = &decoded[..] else {
        panic!("not one ranking list: {decoded:?}");
    };
    rows.iter()
        .map(|row| {
            assert_eq!((row.2, row.3), (0, 0), "a rating in {row:?}");
            (row.0.clone(), row.1)
        })
        .collect()
}

/// The matches' rows of an answer, without the header.
fn rows(world: &mut World, index: usize, count: u8) -> Vec<String> {
    let mut all = asked(world, index, count);
    all.remove(0);
    all.into_iter().map(|row| row.0).collect()
}

/// A game that plays in growing rosters signs in.
fn growing(world: &mut World, name: &str) -> usize {
    let (account, _) = signed_in(world.store, name);
    let session = world
        .store
        .create_session_with_revision(account, PATCH_REVISION);
    world.connect_session((account, session))
}

/// Alice and Bob are sent to a match of the pool their games play in;
/// returns the ticket's id and the lobby's.
fn match_of_two(world: &mut World, alice: usize, bob: usize) -> (u32, u32) {
    world.ask(alice, request::play(1));
    world.ask(bob, request::play(1));
    world.advance(Duration::from_secs(2));
    let ticket = sent_to_match(&world.take(alice)).unwrap().0;
    assert_eq!(sent_to_match(&world.take(bob)).unwrap().0, ticket);
    let (&id, _) = world
        .core
        .matches
        .iter()
        .find(|(_, running)| running.ticket.match_id == ticket)
        .unwrap();
    (ticket, id)
}

#[test]
fn with_no_match_the_list_is_its_header_alone() {
    let mut world = World::new(Duration::from_secs(60));
    let alice = world.connect("Alice");
    // The header is always there: an empty list is not no answer.
    for count in [0, 1, 32, 255] {
        assert_eq!(
            asked(&mut world, alice, count),
            [("M1;0".to_owned(), MATCHES_START)]
        );
    }
    world.assert_clean();
}

#[test]
fn the_list_says_how_far_each_match_is_and_names_nobody() {
    let map = |id| maps::map_mode(id).unwrap();
    assert_eq!((map(RUDNYA_TDM).level, map(RUDNYA_TDM).mode), (1, 4));
    assert_eq!(
        (map(RUDNYA_BATTERIES).level, map(RUDNYA_BATTERIES).mode),
        (1, 2)
    );
    let mut world = World::new(Duration::from_secs(60));
    let alice = world.connect("Alice");
    // The match server's matches, the oldest first: one in play, one that
    // has its result and lingers, one whose countdown runs, one still
    // gathering, and one in play whose score could not be renewed.
    let mut stale = playing(0x55);
    stale.standing = Some(([40, 71], SCORE_AGE + Duration::from_secs(1)));
    *world.host.listed.locked() = vec![
        playing(0x51),
        OpenMatch {
            phase: Phase::Over,
            ..playing(0x52)
        },
        OpenMatch {
            match_id: 0x53,
            map_id: RUDNYA_BATTERIES,
            phase: Phase::Countdown,
            players: [2, 2],
            // No battery yet: nothing to tell, whatever the world says.
            standing: Some(([0, 0], Duration::ZERO)),
            played: None,
        },
        OpenMatch {
            match_id: 0x54,
            map_id: RUDNYA_TDM,
            phase: Phase::Gathering,
            players: [5, 5],
            standing: None,
            played: None,
        },
        stale,
    ];
    // The header counts the four that are open; each row is a place on.
    // The level, the mode, the phase, the teams, the score, the seconds
    // played, and whether it takes a player: none of these is one the lobby
    // made, so none does.
    assert_eq!(
        asked(&mut world, alice, 32),
        [
            ("M1;4".to_owned(), MATCHES_START),
            ("1;4;2;4;3;88;78;380;0".to_owned(), MATCHES_START + 1),
            ("1;2;1;2;2;-;-;-;0".to_owned(), MATCHES_START + 2),
            ("1;4;0;5;5;-;-;-;0".to_owned(), MATCHES_START + 3),
            ("1;4;2;4;3;-;-;380;0".to_owned(), MATCHES_START + 4),
        ]
    );
    // A score as old as it may be is still told.
    world.host.listed.locked()[4].standing = Some(([40, 71], SCORE_AGE));
    assert_eq!(rows(&mut world, alice, 32)[3], "1;4;2;4;3;40;71;380;0");

    // No more rows than asked for, the oldest matches; the header still
    // counts them all.
    assert_eq!(
        asked(&mut world, alice, 2),
        [
            ("M1;4".to_owned(), MATCHES_START),
            ("1;4;2;4;3;88;78;380;0".to_owned(), MATCHES_START + 1),
            ("1;2;1;2;2;-;-;-;0".to_owned(), MATCHES_START + 2),
        ]
    );
    assert_eq!(
        asked(&mut world, alice, 0),
        [("M1;4".to_owned(), MATCHES_START)]
    );
    // And never more than the answer holds, whatever is asked for.
    *world.host.listed.locked() = (0..40).map(playing).collect();
    let all = asked(&mut world, alice, 255);
    assert_eq!(all.len(), 1 + MATCHES_NOW_ROWS);
    assert_eq!(all[0], ("M1;40".to_owned(), MATCHES_START));
    assert_eq!(
        all[MATCHES_NOW_ROWS],
        (
            "1;4;2;4;3;88;78;380;0".to_owned(),
            MATCHES_START + MATCHES_NOW_ROWS as u32
        )
    );
    world.assert_clean();
}

#[test]
fn a_match_takes_a_player_when_the_late_join_rule_would_put_one_in() {
    let mut world = World::new(Duration::ZERO);
    world
        .store
        .matchmaking()
        .set_growing_rules(Duration::ZERO, 1.0);
    let alice = growing(&mut world, "Alice");
    let bob = growing(&mut world, "Bob");
    let carol = growing(&mut world, "Carol");
    let (ticket, id) = match_of_two(&mut world, alice, bob);
    assert_eq!(world.core.matches[&id].pool, QueuePool::GrowingRoster);
    *world.host.listed.locked() = vec![OpenMatch {
        players: [1, 1],
        ..playing(ticket)
    }];
    let join = |world: &mut World| {
        let rows = rows(world, carol, 32);
        assert_eq!(rows.len(), 1);
        match rows[0].as_str() {
            "1;4;2;1;1;88;78;380;1" => true,
            "1;4;2;1;1;88;78;380;0" => false,
            other => panic!("{other}"),
        }
    };
    // The match server does not admit to it (it is not in play yet, or a
    // joiner before is still on its way in).
    assert!(!join(&mut world));
    // It does: one against one takes a third. The player of the list is
    // one whose wait for a better moment is over: an even match takes
    // nobody alone before that, and the list does not say "wait".
    world.host.admitting.locked().insert(ticket);
    assert_eq!(
        world.core.settings.late_join_patience,
        Duration::from_secs(60)
    );
    assert!(join(&mut world));
    // Asking changed nothing: nobody was put anywhere.
    assert_eq!(world.host.made.locked()[0].1.players.len(), 2);
    assert_eq!(world.core.matches[&id].seats.len(), 2);
    assert_eq!(world.store.matchmaking().queued_players(), 0);

    // Too near its end: the cutoff of a late join.
    let cutoff = session::matchmaking_policy::JOIN_CUTOFF;
    world.host.remaining.locked().insert(ticket, cutoff);
    assert!(!join(&mut world));
    world
        .host
        .remaining
        .locked()
        .insert(ticket, cutoff + Duration::from_millis(1));
    assert!(join(&mut world));

    // Both sides have all the players a side may have.
    world.core.settings.team_size = 1;
    assert!(!join(&mut world));
    world.core.settings.team_size = 10;
    assert!(join(&mut world));

    // Every roster place was given, though only two of the twenty are
    // still in the match: a place is never given twice.
    let seats = world.core.matches[&id].seats.clone();
    let running = world.core.matches.get_mut(&id).unwrap();
    running.seats.resize(session::MAX_PLAYERS_PER_MATCH, (0, 0));
    world.host.live.locked().insert(
        ticket,
        LiveView {
            present: 0b11,
            ..LiveView::default()
        },
    );
    assert!(!join(&mut world));
    world.core.matches.get_mut(&id).unwrap().seats = seats;
    assert!(join(&mut world));

    // A match of fixed teams takes nobody.
    world.core.matches.get_mut(&id).unwrap().fixed_teams = true;
    assert!(!join(&mut world));
    world.assert_clean();
}

#[test]
fn a_match_of_games_that_cannot_grow_takes_nobody() {
    let mut world = World::new(Duration::ZERO);
    let alice = world.connect("Alice");
    let bob = world.connect("Bob");
    let carol = world.connect("Carol");
    let (ticket, id) = match_of_two(&mut world, alice, bob);
    assert_eq!(world.core.matches[&id].pool, QueuePool::Standard);
    *world.host.listed.locked() = vec![OpenMatch {
        players: [1, 1],
        ..playing(ticket)
    }];
    // Whatever the match server says of it.
    world.host.admitting.locked().insert(ticket);
    assert_eq!(rows(&mut world, carol, 32), ["1;4;2;1;1;88;78;380;0"]);
    // A player in the queue may ask too, and stays in it.
    world.ask(carol, request::play(1));
    assert_eq!(world.store.matchmaking().queued_players(), 1);
    assert_eq!(rows(&mut world, carol, 32), ["1;4;2;1;1;88;78;380;0"]);
    assert_eq!(world.store.matchmaking().queued_players(), 1);
    world.assert_clean();
}

#[test]
fn a_reserved_start_the_server_does_not_know_gets_no_answer() {
    let mut world = World::new(Duration::from_secs(60));
    let alice = world.connect("Alice");
    *world.host.listed.locked() = vec![playing(0x51)];
    world.take(alice);
    for start in [
        MATCHES_START + 1,
        MATCHES_START + 32,
        3_000_000_000,
        u32::MAX,
    ] {
        for count in [0, 1, 32, 255] {
            world.ask(alice, request::ranking(start, count));
            assert_eq!(world.take(alice), [], "start {start}, count {count}");
        }
    }
    // The link is as good as before.
    assert_eq!(asked(&mut world, alice, 32).len(), 2);
    world.assert_clean();
}

#[test]
fn the_ranking_page_is_answered_as_ever_and_the_list_touches_nothing_of_it() {
    let mut world = World::new(Duration::from_secs(60));
    let alice = world.connect("Alice");
    let bob = world.connect("Bob");
    world.account(bob).rating += 100;
    *world.host.listed.locked() = vec![playing(0x51)];
    let addr = world.clients[alice].0;
    let pushed = |world: &World| world.core.links[&addr].pushed.clone();
    let ranked_sent = |world: &World| world.core.player(addr).unwrap().ranked_sent;

    // The ranking's first page, and a page further down.
    let first_page = |world: &mut World| {
        world.take(alice);
        world.ask(alice, request::ranking(0, 10));
        world.take(alice)
    };
    let before = first_page(&mut world);
    assert!(matches!(
        &before[..],
        [
            Decoded::BaseInfo { ranked_players: 2, .. },
            Decoded::Tutorial { show: false, .. },
            Decoded::PlayerElo { place: 2, .. },
            Decoded::EloList(rows)
        ] if rows.len() == 2 && rows[0].0 == "Bob" && rows[0].1 == 1 && rows[1].0 == "Alice"
    ));
    world.ask(alice, request::ranking(1, 1));
    let second = world.take(alice);
    assert!(matches!(
        &second[..],
        [Decoded::EloList(rows)] if rows.len() == 1 && rows[0].0 == "Alice" && rows[0].1 == 2
    ));

    // The list, asked for with the count the page has of the ranking made
    // wrong on purpose: the first page of the ranking would put it right
    // and send the count, the account's own row and the rows. The list
    // does none of that: one `91`, and the count is as it was.
    world.core.player_mut(addr).unwrap().ranked_sent = 77;
    let sent = pushed(&world);
    let now = world.now;
    for (start, count) in [
        (MATCHES_START, 32),
        (MATCHES_START, 0),
        (MATCHES_START + 1, 32),
        (u32::MAX, 255),
    ] {
        world
            .core
            .handle(addr, ClientMessage::QueryRanking { start, count }, now);
    }
    assert_eq!(ranked_sent(&world), 77);
    let answers = pushed(&world)[sent.len()..].to_vec();
    assert_eq!(
        answers
            .iter()
            .map(|message| message.id_byte())
            .collect::<Vec<_>>(),
        [0x91, 0x91]
    );
    // The last start of all that is a ranking page's is one: no row is
    // that far down, and the answer is the empty list it always was.
    world.core.handle(
        addr,
        ClientMessage::QueryRanking {
            start: MATCHES_START - 1,
            count: 32,
        },
        now,
    );
    assert_eq!(ranked_sent(&world), 77);
    let last = pushed(&world).pop().unwrap();
    assert_eq!((last.id_byte(), last.body()), (0x91, &[0_u8][..]));
    // And the first page does put the count right.
    world.core.handle(
        addr,
        ClientMessage::QueryRanking {
            start: 0,
            count: 10,
        },
        now,
    );
    assert_eq!(ranked_sent(&world), 2);
    world.settle();

    // The page after it all: what it was before.
    assert_eq!(first_page(&mut world), before);
    world.ask(alice, request::ranking(1, 1));
    assert_eq!(world.take(alice), second);
    world.assert_clean();
}
