//! Who is put in a match, on what map and at what level
//! (`docs/notes/matchmaking.md`).

use session::v026g::KitSlot;
use session::v026g::matchmaking::{self, count_map_play, map_plays};

use super::*;

/// A dictionary item by its name.
fn dict_id(name: &str) -> u16 {
    Dictionary::shared()
        .items
        .iter()
        .find(|item| item.name == name)
        .unwrap_or_else(|| panic!("no item {name}"))
        .dict_id
}

/// The account's second kit gets an AK-74M (level 10) with rounds: a kit
/// that can be played, though it is not the one played.
fn arm_second_kit(account: &mut Account) {
    let rifle = account.add_item(dict_id("ak_74m"), FULL_CONDITION);
    let rounds = account.add_item(dict_id("ammo_5.45x39_7n10"), 60);
    account.kits[1].slots[7] = Some(KitSlot {
        item_id: rifle,
        count: 1,
    });
    account.kits[1].slots[8] = Some(KitSlot {
        item_id: rounds,
        count: 60,
    });
}

fn in_queue(decoded: &[Decoded]) -> Option<(u32, u16, [u16; 20])> {
    match queue_state(decoded) {
        Some(ClientState::InQueue {
            place,
            search_step,
            search_properties,
            ..
        }) => Some((place, search_step.get(), search_properties)),
        _ => None,
    }
}

#[test]
fn a_match_has_its_players_mean_level_and_their_least_played_map() {
    // No gather window: only the levels decide when the two meet.
    let mut world = World::new(Duration::ZERO);
    let alice = world.connect("Alice");
    let bob = world.connect("Bob");
    // Alice's best kit is of level 10; Bob's of level 0. Between them they
    // have played every map but Mamayev Kurgan (level_06).
    world.change(alice, |account| {
        arm_second_kit(account);
        for map in [1, 2, 3, 4, 5, 7, 8] {
            count_map_play(account, map);
        }
    });
    world.change(bob, |account| {
        for map in [1, 1, 2, 3] {
            count_map_play(account, map);
        }
    });
    assert_eq!(matchmaking::level(&world.account(alice), 0).min, 10);
    assert_eq!(matchmaking::level(&world.account(bob), 0).min, 0);

    // Each plays its first kit, the starter's.
    world.ask(alice, request::play(1));
    world.ask(bob, request::play(1));
    world.advance(Duration::from_secs(2));
    // Both wait: 0 is 10 levels under Alice's 10..=11, and 10 is 9 over
    // Bob's 0..=1. The search screen is told the step and who waits where.
    let (place, step, waiting) = in_queue(&world.take(alice)).expect("Alice waits");
    assert_eq!((place, step), (1, 4));
    let mut expected = [0_u16; 20];
    expected[0] = 1;
    expected[10] = 1;
    assert_eq!(waiting, expected);
    assert!(world.host.made.lock().unwrap().is_empty());
    world.advance(Duration::from_secs(30));
    assert!(sent_to_match(&world.take(bob)).is_none());
    assert!(world.host.made.lock().unwrap().is_empty());

    // Bob's range reaches 10 after 9 steps of 4 s.
    world.advance(Duration::from_secs(6));
    let (alice_match, alice_team) = sent_to_match(&world.take(alice)).expect("Alice's match");
    let (bob_match, bob_team) = sent_to_match(&world.take(bob)).expect("Bob's match");
    assert_eq!(alice_match, bob_match);
    assert_ne!(alice_team, bob_team);

    let made = world.host.made.lock().unwrap().clone();
    assert_eq!(made.len(), 1);
    let setup = &made[0].1;
    // (10 + 0) / 2: the row of `match_levels_dict` the rewards take.
    assert_eq!(setup.match_level, 5);
    let map = maps::map_mode(setup.map_id).expect("a map of the dictionary");
    assert!(map.is_playable());
    assert_eq!(map.level, 6);
    assert!(world.core.settings.maps.contains(&setup.map_id));
    // The roster is the two of them and no more: the client counts its 80 %
    // of it.
    assert_eq!(setup.players.len(), 2);
    drop(made);

    // It is one more match on that map for each.
    assert_eq!(map_plays(&world.account(alice), 6), 1);
    assert_eq!(map_plays(&world.account(bob), 6), 1);
    assert_eq!(map_plays(&world.account(bob), 1), 2);
    world.assert_clean();
}

#[test]
fn players_of_one_level_meet_as_before_levels_counted() {
    // New accounts are all of level 0: with no window, the match is made
    // as soon as the second presses Play.
    let mut world = World::new(Duration::ZERO);
    let alice = world.connect("Alice");
    let bob = world.connect("Bob");
    world.ask(alice, request::play(1));
    world.ask(bob, request::play(1));
    world.advance(Duration::from_secs(2));
    assert!(sent_to_match(&world.take(alice)).is_some());
    assert!(sent_to_match(&world.take(bob)).is_some());
    let made = world.host.made.lock().unwrap().clone();
    assert_eq!(made[0].1.match_level, 0);
    world.assert_clean();
}

#[test]
fn a_match_takes_two_sides_of_the_size_set_and_the_rest_wait() {
    let mut world = World::new(Duration::from_secs(600));
    world.core.settings.team_size = 2;
    let players: Vec<usize> = ["Alice", "Bob", "Carol", "Dave", "Erin"]
        .into_iter()
        .map(|name| world.connect(name))
        .collect();
    for &player in &players {
        world.ask(player, request::play(1));
    }
    world.advance(Duration::from_secs(2));
    // Four fill a match of two a side at once, long before the window.
    let mut teams = [0, 0];
    for &player in &players[..4] {
        let (_, team) = sent_to_match(&world.take(player)).expect("a match");
        teams[usize::from(team)] += 1;
    }
    assert_eq!(teams, [2, 2]);
    let made = world.host.made.lock().unwrap().clone();
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].1.players.len(), 4);
    drop(made);
    // The fifth waits, alone.
    let (place, _, waiting) = in_queue(&world.take(players[4])).expect("Erin waits");
    assert_eq!(place, 1);
    assert_eq!(waiting.iter().sum::<u16>(), 1);
    world.assert_clean();
}

#[test]
fn every_map_comes_up_before_one_comes_up_again() {
    // Two players who play match after match: eight matches, eight maps.
    let mut world = World::new(Duration::ZERO);
    let alice = world.connect("Alice");
    let bob = world.connect("Bob");
    let mut seen = Vec::new();
    for round in 0..8 {
        world.ask(alice, request::play(1));
        world.ask(bob, request::play(1));
        world.advance(Duration::from_secs(2));
        let (match_id, _) = sent_to_match(&world.take(alice)).expect("a match");
        world.take(bob);
        let made = world.host.made.lock().unwrap().clone();
        assert_eq!(made.len(), round + 1);
        seen.push(maps::map_mode(made[round].1.map_id).unwrap().level);
        // The match server ends it.
        world.host.open.lock().unwrap().remove(&match_id);
        world.inbox.match_finished(MatchFinished { match_id });
        world.advance(Duration::from_secs(2));
        world.take(alice);
        world.take(bob);
    }
    seen.sort_unstable();
    assert_eq!(seen, [1, 2, 3, 4, 5, 6, 7, 8]);
    world.assert_clean();
}

#[test]
fn the_side_size_and_the_search_step_are_settings() {
    use crate::v026g::{parse_search_step, parse_team_size};

    let settings = Settings::local("127.0.0.1:0");
    // "Two teams of up to 8 players"; every level in range within the
    // default gather window of a minute (14 steps from 0..=1 to 15).
    assert_eq!(settings.team_size, 8);
    assert_eq!(settings.search_step.get(), 4);
    assert!(u64::from(settings.search_step.get()) * 14 <= session::DEFAULT_GATHER_WINDOW.as_secs());

    assert_eq!(parse_team_size(" 5 "), Some(5));
    assert_eq!(parse_team_size("10"), Some(10));
    // The client's roster has 20 places; a side of nobody is no side.
    assert_eq!(parse_team_size("11"), None);
    assert_eq!(parse_team_size("0"), None);
    assert_eq!(parse_team_size("eight"), None);
    assert_eq!(parse_search_step("60").map(|step| step.get()), Some(60));
    // The client divides by it.
    assert_eq!(parse_search_step("0"), None);
    assert_eq!(parse_search_step("3601"), None);
    assert_eq!(parse_search_step(""), None);
}
