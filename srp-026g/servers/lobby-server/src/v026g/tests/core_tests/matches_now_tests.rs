//! The matches being played now, for the "matches now" window of srp's
//! changed lobby movie: asked with the ranking page's own request (`3B`)
//! and a start no ranking page has, answered with a `91` whose rows carry
//! the list (`lobby::matches_now`). A stock game never asks, and what it
//! does ask is answered as ever.

use session::QueuePool;
use session::v026g::matches_now::{Phase, SCORE_AGE};
use survarium::v026g::lobby::requests::ClientMessage;
use survarium::v026g::lobby::{JOIN_KEY_MAX, JOIN_START, MATCHES_NOW_ROWS, MATCHES_START};
use survarium::v026g::match_msgs::late_join::PATCH_REVISION;

use super::*;
use crate::v026g::join::Listing;
use crate::v026g::{LiveView, OpenMatch};

/// Team Deathmatch and batteries on Rudnya (level 1).
const RUDNYA_TDM: u8 = 117;
const RUDNYA_BATTERIES: u8 = 61;

/// A match in play the lobby did not make: four against three, 88 tickets
/// to 78 in a world four seconds old, played for 6 min 20 s, of match
/// level 3.
pub(super) fn playing(match_id: u32) -> OpenMatch {
    OpenMatch {
        match_id,
        map_id: RUDNYA_TDM,
        phase: Phase::Playing,
        players: [4, 3],
        standing: Some(([88, 78], Duration::from_secs(4))),
        played: Some(Duration::from_secs(6 * 60 + 20)),
        match_level: 3,
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
pub(super) fn growing(world: &mut World, name: &str) -> usize {
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
            [("M2;0".to_owned(), MATCHES_START)]
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
    world.core.listing = Listing::new(0);
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
            match_level: 7,
        },
        OpenMatch {
            match_id: 0x54,
            map_id: RUDNYA_TDM,
            phase: Phase::Gathering,
            players: [5, 5],
            standing: None,
            played: None,
            match_level: 0,
        },
        stale,
    ];
    // The header counts the four that are open; each row is a place on.
    // The level, the mode, the phase, the teams, the score, the seconds
    // played, whether it takes the asker (none of these is one the lobby
    // made, so none does, and none says why: code 0), the match's listing
    // key (given as the lobby first lists it, one for each match listed),
    // and its match level.
    assert_eq!(
        asked(&mut world, alice, 32),
        [
            ("M2;4".to_owned(), MATCHES_START),
            ("1;4;2;4;3;88;78;380;0;1;3".to_owned(), MATCHES_START + 1),
            ("1;2;1;2;2;-;-;-;0;2;7".to_owned(), MATCHES_START + 2),
            ("1;4;0;5;5;-;-;-;0;3;0".to_owned(), MATCHES_START + 3),
            ("1;4;2;4;3;-;-;380;0;4;3".to_owned(), MATCHES_START + 4),
        ]
    );
    // A score as old as it may be is still told; the keys stay.
    world.host.listed.locked()[4].standing = Some(([40, 71], SCORE_AGE));
    assert_eq!(rows(&mut world, alice, 32)[3], "1;4;2;4;3;40;71;380;0;4;3");

    // No more rows than asked for, the oldest matches; the header still
    // counts them all.
    assert_eq!(
        asked(&mut world, alice, 2),
        [
            ("M2;4".to_owned(), MATCHES_START),
            ("1;4;2;4;3;88;78;380;0;1;3".to_owned(), MATCHES_START + 1),
            ("1;2;1;2;2;-;-;-;0;2;7".to_owned(), MATCHES_START + 2),
        ]
    );
    assert_eq!(
        asked(&mut world, alice, 0),
        [("M2;4".to_owned(), MATCHES_START)]
    );
    // A match that went keeps no key; the others keep theirs.
    world.host.listed.locked().remove(0);
    assert_eq!(rows(&mut world, alice, 1), ["1;2;1;2;2;-;-;-;0;2;7"]);
    assert_eq!(world.core.listing.match_of(1), None);
    assert_eq!(world.core.listing.match_of(2), Some(0x53));
    // And never more than the answer holds, whatever is asked for. Every
    // match open gets its key, listed in the rows or not.
    *world.host.listed.locked() = (0..40).map(playing).collect();
    let all = asked(&mut world, alice, 255);
    assert_eq!(all.len(), 1 + MATCHES_NOW_ROWS);
    assert_eq!(all[0], ("M2;40".to_owned(), MATCHES_START));
    assert_eq!(
        all[MATCHES_NOW_ROWS],
        (
            "1;4;2;4;3;88;78;380;0;36;3".to_owned(),
            MATCHES_START + MATCHES_NOW_ROWS as u32
        )
    );
    assert_eq!(world.core.listing.match_of(44), Some(39));
    world.assert_clean();
}

/// The places of a running match by team, and what each counts for beside
/// being there: `strength[t]` for the first place of team `t`, 0 for the
/// others.
fn set_sides(world: &mut World, id: u32, strength: [i32; 2]) {
    let running = world.core.matches.get_mut(&id).unwrap();
    let mut first = [true; 2];
    running.strengths = running
        .seats
        .iter()
        .map(|&(_, team)| {
            let team = usize::from(team);
            if std::mem::take(&mut first[team]) {
                strength[team]
            } else {
                0
            }
        })
        .collect();
}

/// What the asker counts for besides being there, as the queue counts it.
fn counts(world: &World, index: usize) -> i32 {
    let account = world.ids[index].0;
    let level = world.core.level_of(account, session::v026g::quests::now());
    world.core.strength_of(account, level)
}

/// The match rows' `join` code and `key` the lobby answers the asker now
/// (1: it may join; any other: why not). The request is handled at once,
/// with no round of the clock: nothing else of the lobby runs in between
/// (no pass puts a waiting player into a match).
pub(super) fn joins_now(world: &mut World, index: usize) -> Vec<(u8, u32)> {
    let rows = answered(world, index, MATCHES_START, 32);
    let [list] = &rows[..] else {
        panic!("not one list: {rows:?}");
    };
    list[1..]
        .iter()
        .map(|(name, _)| {
            let fields: Vec<&str> = name.split(';').collect();
            assert_eq!(fields.len(), 11, "{name}");
            (fields[8].parse().unwrap(), fields[9].parse().unwrap())
        })
        .collect()
}

/// The `91`s the lobby answers a `3B` with `start` at once, as the client
/// reads them: the rows of each, as `(name, place)`.
pub(super) fn answered(
    world: &mut World,
    index: usize,
    start: u32,
    count: u8,
) -> Vec<Vec<(String, u32)>> {
    let addr = world.clients[index].0;
    let before = world.core.links[&addr].pushed.len();
    let now = world.now;
    world
        .core
        .handle(addr, ClientMessage::QueryRanking { start, count }, now);
    let mut client = survarium::v026g::lobby::client_model::Client::signed_in();
    world.core.links[&addr].pushed[before..]
        .iter()
        .map(|message| match client.receive(message).unwrap() {
            Decoded::EloList(rows) => rows.into_iter().map(|row| (row.0, row.1)).collect(),
            other => panic!("not a ranking list: {other:?}"),
        })
        .collect()
}

/// A match of growing rosters of the players `names` sign in with, all
/// sent to it, which takes players: its ticket's id, the lobby's id, and
/// the players. Three are two against one, seven four against three.
pub(super) fn match_of(world: &mut World, names: &[&str]) -> (u32, u32, Vec<usize>) {
    // They gather until all have pressed Play.
    world
        .store
        .matchmaking()
        .set_growing_rules(Duration::from_secs(120), 1.0);
    let players: Vec<usize> = names.iter().map(|name| growing(world, name)).collect();
    for &player in &players {
        world.ask(player, request::play(1));
    }
    world
        .store
        .matchmaking()
        .set_growing_rules(Duration::ZERO, 1.0);
    world.advance(Duration::from_secs(2));
    let ticket = sent_to_match(&world.take(players[0])).unwrap().0;
    for &player in &players[1..] {
        assert_eq!(sent_to_match(&world.take(player)).unwrap().0, ticket);
    }
    let (&id, running) = world
        .core
        .matches
        .iter()
        .find(|(_, running)| running.ticket.match_id == ticket)
        .unwrap();
    assert_eq!(running.pool, QueuePool::GrowingRoster);
    assert_eq!(running.seats.len(), names.len());
    world.host.admitting.locked().insert(ticket);
    *world.host.listed.locked() = vec![OpenMatch {
        players: [0, 1]
            .map(|team| running.seats.iter().filter(|seat| seat.1 == team).count() as u8),
        ..playing(ticket)
    }];
    (ticket, id, players)
}

/// The team with more players of a match of an odd count, and the other.
pub(super) fn larger_and_smaller(world: &World, id: u32) -> (u8, u8) {
    let seats = &world.core.matches[&id].seats;
    let on_0 = seats.iter().filter(|seat| seat.1 == 0).count();
    let larger = u8::from(on_0 * 2 < seats.len());
    (larger, 1 - larger)
}

/// The owner's rule: a match's JOIN is on for a player only when that
/// player would be put into it at once by the balance rule, and only to
/// make a match that is uneven by value more even, counting what the
/// player itself counts for. Each asker gets its own rows.
#[test]
fn the_join_of_a_row_is_on_only_where_the_asker_makes_an_uneven_match_more_even() {
    let mut world = World::new(Duration::ZERO);
    world.core.listing = Listing::new(0);
    let names = ["P1", "P2", "P3", "P4", "P5", "P6", "P7"];
    let (ticket, id, players) = match_of(&mut world, &names);
    let carol = growing(&mut world, "Carol");
    let (_, three) = larger_and_smaller(&world, id);
    let player = world.core.settings.balance.player_value();
    // A new account counts for little beside being there.
    let carol_counts = counts(&world, carol);
    assert!((0..player / 4).contains(&carol_counts), "{carol_counts}");

    // Four against three, all alike: the three are a player short, and
    // Carol among them makes it four against four that count nearly
    // alike: on, under the key the match got when it was made.
    set_sides(&mut world, id, [0, 0]);
    assert_eq!(joins_now(&mut world, carol), [(1, 1)]);
    // Asking changed nothing: nobody was put anywhere.
    assert_eq!(world.host.made.locked()[0].1.players.len(), 7);
    assert_eq!(world.core.matches[&id].seats.len(), 7);
    assert_eq!(world.store.matchmaking().queued_players(), 0);

    // Three against four that count alike: one of the three counts for a
    // player more. Not uneven by value, so nobody joins it, on either side:
    // "teams are already balanced" (2).
    let mut strength = [0; 2];
    strength[usize::from(three)] = player;
    set_sides(&mut world, id, strength);
    assert_eq!(joins_now(&mut world, carol), [(2, 1)]);
    // Less than that: still uneven, and Carol makes it more even.
    strength[usize::from(three)] = player / 4;
    set_sides(&mut world, id, strength);
    assert_eq!(joins_now(&mut world, carol), [(1, 1)]);

    // An asker so strong that the sides would count further apart with it
    // than without: off. The same match, the same moment, a weaker asker:
    // on. (Its balance rating, 4 gear levels a point when trusted.)
    set_sides(&mut world, id, [0, 0]);
    let strong = growing(&mut world, "Strong");
    world.account(strong).balance = session::v026g::balance::Balance {
        rating: 5_000,
        matches: 1_000,
    };
    assert!(
        counts(&world, strong) >= player,
        "{}",
        counts(&world, strong)
    );
    assert_eq!(joins_now(&mut world, strong), [(2, 1)]);
    world.account(strong).balance = session::v026g::balance::Balance {
        rating: 1_000,
        matches: 1_000,
    };
    assert!(counts(&world, strong) < player);
    assert_eq!(joins_now(&mut world, strong), [(1, 1)]);
    assert_eq!(joins_now(&mut world, carol), [(1, 1)]);

    // The match server does not admit now: off for everyone, and why is
    // its word on the match. In play with time left: a joiner before is on
    // its way in (4). Its time near the end (6). Not in play yet (5). It
    // says nothing (0).
    let cutoff = session::matchmaking_policy::JOIN_CUTOFF;
    world.host.admitting.locked().remove(&ticket);
    let in_play = LiveView {
        present: 0x7F,
        played: Some(Duration::from_secs(300)),
        time_left: Some(Duration::from_secs(600)),
        ..LiveView::default()
    };
    world.host.live.locked().insert(ticket, in_play);
    assert_eq!(joins_now(&mut world, carol), [(4, 1)]);
    let near_end = LiveView {
        time_left: Some(cutoff),
        ..in_play
    };
    world.host.live.locked().insert(ticket, near_end);
    assert_eq!(joins_now(&mut world, carol), [(6, 1)]);
    let countdown = LiveView {
        played: None,
        ..in_play
    };
    world.host.live.locked().insert(ticket, countdown);
    assert_eq!(joins_now(&mut world, carol), [(5, 1)]);
    world.host.live.locked().remove(&ticket);
    assert_eq!(joins_now(&mut world, carol), [(0, 1)]);
    world.host.admitting.locked().insert(ticket);
    // Too near its end: the cutoff of a late join (6).
    world.host.remaining.locked().insert(ticket, cutoff);
    assert_eq!(joins_now(&mut world, carol), [(6, 1)]);
    world.host.remaining.locked().remove(&ticket);
    // The side Carol would go to has all the players a side may have: "no
    // free place" (3).
    world.core.settings.team_size = 3;
    assert_eq!(joins_now(&mut world, carol), [(3, 1)]);
    world.core.settings.team_size = 10;
    assert_eq!(joins_now(&mut world, carol), [(1, 1)]);

    // A player in the queue: off, "leave the search first" (13). (It
    // pressed Play while the match did not admit, so nothing put it in.)
    // Its own reason comes first, before any of the match: also while the
    // match does not admit, and while its sides count alike.
    world.host.admitting.locked().remove(&ticket);
    world.ask(carol, request::play(1));
    assert_eq!(world.store.matchmaking().queued_players(), 1);
    assert_eq!(joins_now(&mut world, carol), [(13, 1)]);
    world.host.admitting.locked().insert(ticket);
    assert_eq!(joins_now(&mut world, carol), [(13, 1)]);
    strength[usize::from(three)] = player;
    set_sides(&mut world, id, strength);
    assert_eq!(joins_now(&mut world, carol), [(13, 1)]);
    set_sides(&mut world, id, [0, 0]);
    world.host.admitting.locked().remove(&ticket);
    world.ask(carol, request::leave(1, false));
    assert_eq!(world.store.matchmaking().queued_players(), 0);
    world.host.admitting.locked().insert(ticket);
    assert_eq!(joins_now(&mut world, carol), [(1, 1)]);

    // A player of the match: off, "you are already in a match" (8). And
    // once it has left it by its own word and is back in the lobby: still
    // off, a place is never given twice (11).
    assert_eq!(joins_now(&mut world, players[0]), [(8, 1)]);
    let left = world.ids[players[0]].0;
    assert!(world.store.matchmaking().give_up_seat(left).is_some());
    world.ask(players[0], request::leave(1, true));
    assert!(
        world
            .core
            .player(world.clients[players[0]].0)
            .unwrap()
            .in_match
            .is_none()
    );
    assert_eq!(world.store.matchmaking().pool_of_account(left), None);
    assert_eq!(joins_now(&mut world, players[0]), [(11, 1)]);

    // A game too old to join a match being played: off (10).
    let old = world.connect("Old");
    assert_eq!(joins_now(&mut world, old), [(10, 1)]);
    // A game the server refuses Play for its revision: off; one it only
    // warns: on.
    world.core.settings.min_patch_revision = PATCH_REVISION + 1;
    world.core.settings.patch_check = crate::v026g::PatchCheck::Refuse;
    assert_eq!(joins_now(&mut world, carol), [(10, 1)]);
    world.core.settings.patch_check = crate::v026g::PatchCheck::Warn;
    assert_eq!(joins_now(&mut world, carol), [(1, 1)]);
    world.core.settings.patch_check = crate::v026g::PatchCheck::Off;

    // In a squad with another player: off for both (9).
    let alice = growing(&mut world, "Alice");
    let bob = growing(&mut world, "Bob");
    assert_eq!(joins_now(&mut world, alice), [(1, 1)]);
    form_squad(&mut world, alice, bob);
    assert!(world.store.squads().of(world.ids[alice].0).is_some());
    assert_eq!(joins_now(&mut world, alice), [(9, 1)]);
    assert_eq!(joins_now(&mut world, bob), [(9, 1)]);

    // A match of fixed teams takes nobody (7); a player in a squad is told
    // of its squad first, its own reason before the match's.
    world.core.matches.get_mut(&id).unwrap().fixed_teams = true;
    assert_eq!(joins_now(&mut world, carol), [(7, 1)]);
    assert_eq!(joins_now(&mut world, alice), [(9, 1)]);
    world.assert_clean();
}

#[test]
fn a_match_of_games_that_cannot_grow_takes_nobody() {
    let mut world = World::new(Duration::ZERO);
    world.core.listing = Listing::new(0);
    let alice = world.connect("Alice");
    let bob = world.connect("Bob");
    let carol = world.connect("Carol");
    let (ticket, id) = match_of_two(&mut world, alice, bob);
    assert_eq!(world.core.matches[&id].pool, QueuePool::Standard);
    *world.host.listed.locked() = vec![OpenMatch {
        players: [1, 1],
        ..playing(ticket)
    }];
    // Whatever the match server says of it: a game of growing rosters is
    // told the match takes no new players (7).
    world.host.admitting.locked().insert(ticket);
    let dave = growing(&mut world, "Dave");
    assert_eq!(rows(&mut world, dave, 32), ["1;4;2;1;1;88;78;380;7;1;3"]);
    // A stock game is told of its game first (10).
    assert_eq!(rows(&mut world, carol, 32), ["1;4;2;1;1;88;78;380;10;1;3"]);
    // A player in the queue may ask too, and stays in it; it is told of
    // the queue first (13).
    world.ask(carol, request::play(1));
    assert_eq!(world.store.matchmaking().queued_players(), 1);
    assert_eq!(rows(&mut world, carol, 32), ["1;4;2;1;1;88;78;380;13;1;3"]);
    assert_eq!(world.store.matchmaking().queued_players(), 1);
    // Its JOIN is answered "busy": it is in the queue (13; the match
    // would take nobody late either). Nothing is held, and the player
    // still waits.
    assert_eq!(
        answered(&mut world, carol, JOIN_START + 1, 1),
        [[("J1;busy;1;13".to_owned(), JOIN_START)]]
    );
    assert_eq!(world.store.matchmaking().queued_players(), 1);
    assert!(world.core.holds.is_empty());
    world.settle();
    world.assert_clean();
}

#[test]
fn a_reserved_start_the_server_does_not_know_gets_no_answer() {
    let mut world = World::new(Duration::from_secs(60));
    let alice = world.connect("Alice");
    *world.host.listed.locked() = vec![playing(0x51)];
    world.take(alice);
    // Past the list and before the JOIN starts; the JOIN start itself
    // (key 0 is no key); past the last key.
    for start in [
        MATCHES_START + 1,
        MATCHES_START + 32,
        JOIN_START - 1,
        JOIN_START,
        JOIN_START + JOIN_KEY_MAX + 1,
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
    // does none of that: one `91`, and the count is as it was. Nor does a
    // JOIN (of a key no match has: one `91` that says "gone").
    world.core.player_mut(addr).unwrap().ranked_sent = 77;
    let sent = pushed(&world);
    let now = world.now;
    for (start, count) in [
        (MATCHES_START, 32),
        (MATCHES_START, 0),
        (MATCHES_START + 1, 32),
        (JOIN_START + 1, 1),
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
        [0x91, 0x91, 0x91]
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

/// Near a planned maintenance (30 minutes or less) the rows say per match
/// whether it ends a minute or more before the maintenance starts: those
/// take the asker on their weaker side by value (1) where the balance rule
/// would not (2), the others say "ends after the maintenance" (12). A match
/// the lobby did not make says no reason (0) as ever. The match level is
/// each row's last field.
#[test]
fn near_a_maintenance_the_rows_say_which_matches_end_before_it() {
    let mut world = World::new(Duration::ZERO);
    world.core.listing = Listing::new(0);
    let names = ["P1", "P2", "P3", "P4", "P5", "P6", "P7"];
    let (ticket, id, _) = match_of(&mut world, &names);
    let carol = growing(&mut world, "Carol");
    let (four, three) = larger_and_smaller(&world, id);
    // The three count for three players more: the four are the weaker.
    let player = world.core.settings.balance.player_value();
    let mut strength = [0; 2];
    strength[usize::from(three)] = 3 * player;
    set_sides(&mut world, id, strength);
    world.host.listed.locked().push(playing(0x51));
    let teams = |world: &World| world.host.listed.locked()[0].players;
    let [team0, team1] = teams(&world);
    assert_eq!(team0 + team1, 7);
    let row = |code: u8| format!("1;4;2;{team0};{team1};88;78;380;{code};1;3");
    let other = "1;4;2;4;3;88;78;380;0;2;3";

    assert_eq!(rows(&mut world, carol, 32), [row(2).as_str(), other]);
    let unix = session::v026g::quests::now();
    // Planned in 20 minutes; the match ends in 10 (the test host's
    // default): JOIN on.
    world.store.set_maintenance(Some(unix + 20 * 60));
    assert_eq!(rows(&mut world, carol, 32), [row(1).as_str(), other]);
    // It ends in 25 minutes: after the maintenance.
    world
        .host
        .remaining
        .locked()
        .insert(ticket, Duration::from_secs(25 * 60));
    assert_eq!(rows(&mut world, carol, 32), [row(12).as_str(), other]);
    // Planned in 45 minutes: Play is on, the balance rule as ever.
    world.store.set_maintenance(Some(unix + 45 * 60));
    assert_eq!(rows(&mut world, carol, 32), [row(2).as_str(), other]);
    // In 20 again, the match ending in 10: Carol's JOIN and Play put her
    // on the four's side, five against three.
    world.store.set_maintenance(Some(unix + 20 * 60));
    world.host.remaining.locked().remove(&ticket);
    assert_eq!(
        answered(&mut world, carol, JOIN_START + 1, 1),
        [[("J1;ok;1;1".to_owned(), JOIN_START)]]
    );
    world.ask(carol, request::play(1));
    assert_eq!(sent_to_match(&world.take(carol)), Some((ticket, four)));
    let seats = &world.core.matches[&id].seats;
    assert_eq!(seats.iter().filter(|seat| seat.1 == four).count(), 5);
    world.store.set_maintenance(None);
    world.assert_clean();
}
