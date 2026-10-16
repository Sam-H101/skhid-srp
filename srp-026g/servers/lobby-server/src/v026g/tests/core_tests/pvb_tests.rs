//! "Player vs Bots" (`pvb`): the status and the choice the changed lobby
//! movie asks for (the `B1`/`B2` rows), the Play that makes the match at
//! once with one player against the server's bots, and what the rule
//! refuses, each with its code.

use session::v026g::account::KitSlot;
use session::v026g::{matchmaking, quests};
use survarium::v026g::item_params::item_params;
use survarium::v026g::lobby::{JOIN_START, PVB_CHOOSE_BASE, PVB_STATUS_START, pvb_choice_start};

use super::matches_now_tests::{answered, growing, joins_now};
use super::*;
use crate::v026g::join::JOIN_REFUSED;
use crate::v026g::pvb::{self, Config, MAPS};

/// A lobby with the mode on, at most `max_matches` matches against bots.
fn world_with(max_matches: usize) -> World {
    let mut settings = Settings::local("127.0.0.1:0");
    settings.pvb = Config {
        enabled: true,
        max_matches,
        ..Config::default()
    };
    World::with_settings(Duration::from_secs(60), settings)
}

/// An ammo of `level` (`match_making_level`): in the Mosin's second ammo
/// slot of the first kit it makes the account's equipment level that.
fn ammo_of_level(level: u8) -> u16 {
    Dictionary::shared()
        .items
        .iter()
        .find(|item| {
            item.name.starts_with("ammo_")
                && item_params(item.dict_id)
                    .is_some_and(|params| params.match_making_level == level)
        })
        .unwrap_or_else(|| panic!("no ammo of level {level}"))
        .dict_id
}

/// The player's equipment level becomes `level`.
fn with_gear(world: &mut World, index: usize, level: u8) {
    world.change(index, |account| {
        let item_id = account.add_item(ammo_of_level(level), 30);
        account.kits[0].slots[9] = Some(KitSlot { item_id, count: 30 });
    });
    let gear = matchmaking::gear_level(&world.account(index), quests::now());
    assert_eq!(gear, level);
}

/// The one row of the one `91` a request at `start` is answered with.
fn one_row(world: &mut World, index: usize, start: u32) -> (String, u32) {
    let answers = answered(world, index, start, 1);
    let [rows] = &answers[..] else {
        panic!("not one answer: {answers:?}");
    };
    let [row] = &rows[..] else {
        panic!("not one row: {rows:?}");
    };
    row.clone()
}

/// The status: its header and its map rows, each checked at its place.
fn status(world: &mut World, index: usize) -> (String, Vec<String>) {
    let answers = answered(world, index, PVB_STATUS_START, 32);
    let [rows] = &answers[..] else {
        panic!("not one answer: {answers:?}");
    };
    for (i, (_, place)) in rows.iter().enumerate() {
        assert_eq!(*place, PVB_STATUS_START + i as u32);
    }
    let mut names = rows.iter().map(|(name, _)| name.clone());
    let header = names.next().expect("a header");
    (header, names.collect())
}

/// The player chooses `size` on `map_id`: the row's name, at the start
/// asked.
fn choose(world: &mut World, index: usize, size: u8, map_id: u8) -> String {
    let start = pvb_choice_start(size, map_id);
    let (name, place) = one_row(world, index, start);
    assert_eq!(place, start);
    name
}

/// The player chooses and presses Play: the match it was sent to.
fn play_vs_bots(world: &mut World, index: usize, size: u8, map_id: u8) -> (u32, u8) {
    assert_eq!(
        choose(world, index, size, map_id),
        format!("B2;ok;{size};{map_id}")
    );
    world.take(index);
    world.ask(index, request::play(1));
    world.advance(Duration::from_secs(1));
    let decoded = world.take(index);
    assert_eq!(denial(&decoded, 0x2F), None);
    sent_to_match(&decoded).expect("sent to its match")
}

/// The owner's example: a player of equipment level 5 chooses 8 v 8 on
/// Rudnya's batteries (61) and is in a match at once, alone: on team 0
/// with 7 of the server's bots against 8; fixed teams, the map chosen, a
/// match against bots for the match server.
#[test]
fn gear_5_gets_an_8_v_8_with_15_bots_on_the_map_chosen() {
    let mut world = world_with(5);
    let alice = growing(&mut world, "Alice");
    with_gear(&mut world, alice, 5);
    let (header, maps) = status(&mut world, alice);
    assert_eq!(header, "B1;0;5;5;6.8.10;8;61;0;5");
    // The owner's maps, each free for five matches.
    let listed: Vec<u8> = maps
        .iter()
        .map(|row| row.split(';').next().unwrap().parse().unwrap())
        .collect();
    assert_eq!(listed, MAPS);
    assert_eq!(maps[0], "61;1;2;5");
    assert_eq!(maps[8], "117;1;4;5");

    let (ticket, team) = play_vs_bots(&mut world, alice, 8, 61);
    assert_eq!(team, 0);
    let made = world.host.made.locked();
    let (id, setup) = made.last().unwrap();
    assert_eq!(*id, ticket);
    assert!(setup.vs_bots);
    assert_eq!(setup.map_id, 61);
    assert_eq!(setup.players.len(), 16);
    assert_eq!(setup.players[0].account_id, world.ids[alice].0);
    assert!(!setup.players[0].driven);
    let bots = &setup.players[1..];
    assert_eq!(bots.len(), 15);
    assert!(bots.iter().all(|bot| bot.driven && bot.account_id == 0));
    let on = |team: u8| {
        setup
            .players
            .iter()
            .filter(|player| player.profile.team == team)
            .count()
    };
    assert_eq!((on(0), on(1)), (8, 8));
    assert_eq!(
        bots.iter().filter(|bot| bot.profile.team == 0).count(),
        7,
        "the player's side: the player and 7 bots"
    );
    drop(made);
    let running = world
        .core
        .matches
        .values()
        .find(|running| running.ticket.match_id == ticket)
        .unwrap();
    assert!(running.vs_bots && running.fixed_teams);
    assert_eq!(running.map_id, 61);
    // The play counts towards no map rotation, and nobody waits.
    assert!(world.account(alice).map_plays.is_empty());
    assert_eq!(world.store.matchmaking().queued_players(), 0);
    // It is one of the five now.
    let (header, maps) = status(&mut world, alice);
    assert_eq!(header, "B1;4;5;5;6.8.10;8;61;1;5");
    assert!(maps.iter().all(|row| row.ends_with(";4")), "{maps:?}");
    world.assert_clean();
}

/// Each size: N - 1 bots with the player, N against it.
#[test]
fn every_size_has_its_bots_on_both_sides() {
    let mut world = world_with(5);
    for (n, size) in pvb::SIZES.into_iter().enumerate() {
        let player = growing(&mut world, &format!("P{n}"));
        let (ticket, _) = play_vs_bots(&mut world, player, size, 117);
        let made = world.host.made.locked();
        let (_, setup) = made.iter().find(|(id, _)| *id == ticket).unwrap();
        assert_eq!(setup.players.len(), 2 * usize::from(size));
        let side = |team: u8| {
            setup
                .players
                .iter()
                .filter(|player| player.profile.team == team)
                .count()
        };
        assert_eq!((side(0), side(1)), (usize::from(size), usize::from(size)));
        assert_eq!(setup.map_id, 117);
    }
}

/// Equipment level 6 is over the most (5): refused with code 1 and both
/// levels, in the status and the choice; the Play then queues as ever.
#[test]
fn gear_6_is_refused_with_code_1_and_its_level() {
    let mut world = world_with(5);
    let bob = growing(&mut world, "Bob");
    with_gear(&mut world, bob, 6);
    let (header, maps) = status(&mut world, bob);
    assert_eq!(header, "B1;1;6;5;6.8.10;8;61;0;5");
    assert_eq!(maps.len(), MAPS.len());
    assert_eq!(choose(&mut world, bob, 8, 61), "B2;no;1;6;5;0;5");
    assert!(world.core.pvb_holds.is_empty());
    world.ask(bob, request::play(1));
    assert_eq!(world.store.matchmaking().queued_players(), 1);
    assert!(world.host.made.locked().is_empty());
}

/// In a squad: code 2. The mode off: code 6. A size or map not offered
/// (London, another weather, a size of none): code 10. An old game: code
/// 5. Waiting in the queue: code 3.
#[test]
fn every_refusal_says_its_code() {
    let mut world = world_with(5);
    let alice = growing(&mut world, "Alice");
    let bob = growing(&mut world, "Bob");
    form_squad(&mut world, alice, bob);
    assert!(status(&mut world, alice).0.starts_with("B1;2;0;5;"));
    assert_eq!(choose(&mut world, alice, 8, 61), "B2;no;2;0;5;0;5");

    let carol = growing(&mut world, "Carol");
    for (size, map_id) in [(8, 181), (8, 62), (7, 61), (12, 61)] {
        assert_eq!(
            choose(&mut world, carol, size, map_id),
            "B2;no;10;0;5;0;5",
            "{size} on {map_id}"
        );
    }
    let standard = world.connect("Standard");
    assert!(status(&mut world, standard).0.starts_with("B1;5;"));
    world.ask(carol, request::play(1));
    assert!(status(&mut world, carol).0.starts_with("B1;3;"));
    assert_eq!(choose(&mut world, carol, 8, 61), "B2;no;3;0;5;0;5");

    world.core.settings.pvb.enabled = false;
    let dave = growing(&mut world, "Dave");
    let (header, maps) = status(&mut world, dave);
    assert_eq!(header, "B1;6;0;5;6.8.10;8;61;0;5");
    assert!(maps.iter().all(|row| row.ends_with(";0")));
    assert_eq!(choose(&mut world, dave, 8, 61), "B2;no;6;0;5;0;5");
    // A start past the choices is no choice of these.
    assert_eq!(pvb_choice_start(255, 255), PVB_CHOOSE_BASE + 255_255);
}

/// Five matches against bots run: the sixth is refused with code 7, the
/// numbers say 5 of 5, and no map is free; once one is over, there is room.
#[test]
fn the_sixth_bot_match_is_refused_with_code_7() {
    let mut world = world_with(5);
    let mut tickets = Vec::new();
    for (n, &map_id) in MAPS.iter().enumerate().take(5) {
        let player = growing(&mut world, &format!("P{n}"));
        tickets.push(play_vs_bots(&mut world, player, 6, map_id).0);
    }
    let sixth = growing(&mut world, "Sixth");
    let (header, maps) = status(&mut world, sixth);
    assert_eq!(header, "B1;7;0;5;6.8.10;8;61;5;5");
    assert!(maps.iter().all(|row| row.ends_with(";0")), "{maps:?}");
    assert_eq!(choose(&mut world, sixth, 6, 61), "B2;no;7;0;5;5;5");
    assert_eq!(world.host.made.locked().len(), 5);

    // A choice held counts too: with four running, one held leaves none.
    world.host.open.locked().remove(&tickets[0]);
    world.advance(Duration::from_secs(2));
    let seventh = growing(&mut world, "Seventh");
    assert_eq!(choose(&mut world, seventh, 6, 61), "B2;ok;6;61");
    assert!(
        status(&mut world, sixth)
            .0
            .starts_with("B1;7;0;5;6.8.10;8;61;5;5")
    );
    // Its choice given up (a choice not granted replaces it), the sixth
    // gets in.
    assert!(choose(&mut world, seventh, 7, 61).starts_with("B2;no;10;"));
    play_vs_bots(&mut world, sixth, 6, 61);
}

/// A Play whose choice ran out within 30 s is refused and never queued;
/// one later is an ordinary Play.
#[test]
fn a_play_after_its_choice_ran_out_is_not_queued() {
    let mut world = world_with(5);
    let alice = growing(&mut world, "Alice");
    assert_eq!(choose(&mut world, alice, 8, 61), "B2;ok;8;61");
    world.advance(pvb::HOLD + Duration::from_secs(1));
    world.take(alice);
    world.ask(alice, request::play(1));
    let decoded = world.take(alice);
    assert_eq!(denial(&decoded, 0x2F).as_deref(), Some(JOIN_REFUSED));
    assert_eq!(sent_to_match(&decoded), None);
    assert_eq!(world.store.matchmaking().queued_players(), 0);
    assert!(world.host.made.locked().is_empty());
    // The choice is used up: the next Play is the queue's.
    world.ask(alice, request::play(1));
    assert_eq!(world.store.matchmaking().queued_players(), 1);
    assert!(world.host.made.locked().is_empty());
    world.ask(alice, request::leave(1, false));
    assert_eq!(world.store.matchmaking().queued_players(), 0);

    // Past the 30 s, a choice that ran out is forgotten.
    assert_eq!(choose(&mut world, alice, 8, 61), "B2;ok;8;61");
    world.advance(pvb::HOLD + pvb::EXPIRED_REFUSED + Duration::from_secs(1));
    world.ask(alice, request::play(1));
    assert_eq!(world.store.matchmaking().queued_players(), 1);
}

/// A match against bots never takes a late joiner: the queue's late joins
/// pass it by, its row says code 7 and its JOIN is refused with it.
#[test]
fn a_running_bot_match_takes_no_late_joiner() {
    let mut world = world_with(5);
    world.core.listing = crate::v026g::join::Listing::new(0);
    let alice = growing(&mut world, "Alice");
    let (ticket, _) = play_vs_bots(&mut world, alice, 6, 61);
    // The match server would admit.
    world.host.admitting.locked().insert(ticket);
    *world.host.listed.locked() = vec![crate::v026g::OpenMatch {
        match_id: ticket,
        map_id: 61,
        phase: session::v026g::matches_now::Phase::Playing,
        players: [6, 6],
        standing: Some(([0, 0], Duration::ZERO)),
        played: Some(Duration::from_secs(60)),
        match_level: 0,
    }];
    let bob = growing(&mut world, "Bob");
    assert_eq!(joins_now(&mut world, bob), [(7, 1)]);
    assert_eq!(one_row(&mut world, bob, JOIN_START + 1).0, "J1;gone;1;7");
    world.ask(bob, request::play(1));
    world.advance(Duration::from_secs(3));
    assert_eq!(sent_to_match(&world.take(bob)), None);
    let made = world.host.made.locked();
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].1.players.len(), 12);
    drop(made);
    assert_eq!(world.store.matchmaking().queued_players(), 1);
}

/// The match's end pays the player at half (`rewards`, the
/// match server's flag carried over) and its bots nothing.
#[test]
fn a_bot_match_is_settled_as_one() {
    let mut world = world_with(5);
    let alice = growing(&mut world, "Alice");
    let (ticket, _) = play_vs_bots(&mut world, alice, 6, 61);
    world.end_match(ticket, 0);
    let report = world.results.find(ticket).expect("settled");
    assert_eq!(report.players.len(), 12);
    let player = report.player(world.ids[alice].0).unwrap();
    assert!(player.reward.won && !player.reward.daily_bonus);
    assert_eq!(player.reward.quest_money, 0);
    assert_eq!(player.reward.rating_after, player.reward.rating_before);
}
