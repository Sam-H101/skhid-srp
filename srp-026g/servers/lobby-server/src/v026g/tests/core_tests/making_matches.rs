//! Who is put in a match, on what map and at what level
//! (`resources/notes/v026g/matchmaking.md`).

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
fn a_match_has_its_players_mean_level_and_counts_its_map() {
    // No gather window: only the levels decide when the two meet.
    let mut world = World::new(Duration::ZERO);
    let alice = world.connect("Alice");
    let bob = world.connect("Bob");
    // Alice's best kit is of level 10; Bob's of level 0. Between them they
    // have played every map but Mamayev Kurgan (level_06), which is the
    // likeliest to be drawn but not certain (`matchmaking::choose_map`).
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
    let plays_before = |player| -> Vec<u32> {
        let account = world.account(player);
        (0..=9).map(|level| map_plays(&account, level)).collect()
    };
    let (alice_before, bob_before) = (plays_before(alice), plays_before(bob));

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
    assert!(world.core.settings.maps.contains(&setup.map_id));
    // The roster is the two of them and no more: the client counts its 80 %
    // of it.
    assert_eq!(setup.players.len(), 2);
    drop(made);

    // It is one more match on that map for each, and on no other.
    for (player, before) in [(alice, alice_before), (bob, bob_before)] {
        let account = world.account(player);
        for level in 0..=9 {
            let more = u32::from(level == map.level);
            assert_eq!(
                map_plays(&account, level),
                before[usize::from(level)] + more,
                "{level}"
            );
        }
    }
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
fn raising_equipment_after_queueing_restarts_search_at_the_new_level() {
    // Actual inventory requests, both in the kit used for Play and in
    // another playable kit, must replace the old search.
    for (kit_id, weapon_slot, ammo_slot) in [(1, 10, 11), (2, 7, 8)] {
        let mut world = World::new(Duration::ZERO);
        let alice = world.connect("Alice");
        let bob = world.connect("Bob");
        let rifle_dict = dict_id("ak_74m");
        let rounds_dict = dict_id("ammo_5.45x39_7n10");
        let (mut rifle, mut rounds) = (0, 0);
        world.change(alice, |account| {
            rifle = account.add_item(rifle_dict, FULL_CONDITION);
            rounds = account.add_item(rounds_dict, 60);
        });
        assert_eq!(matchmaking::level(&world.account(alice), 0).min, 0);
        world.ask(alice, request::play(1));
        let mut expected = [0_u16; 20];
        expected[0] = 1;
        assert_eq!(world.store.matchmaking().waiting_by_level::<20>(), expected);
        world.advance(Duration::from_secs(40));
        world.take(alice);

        world.ask(
            alice,
            request::move_item(kit_id, rifle, rifle_dict, INVENTORY_SLOT, weapon_slot, 1),
        );
        world.ask(
            alice,
            request::move_item(kit_id, rounds, rounds_dict, INVENTORY_SLOT, ammo_slot, 60),
        );
        assert_eq!(matchmaking::level(&world.account(alice), 0).min, 10);
        // The old widened range must not carry over to the new search.
        expected[0] = 0;
        expected[10] = 1;
        assert_eq!(world.store.matchmaking().waiting_by_level::<20>(), expected);
        assert!(world.host.made.lock().unwrap().is_empty());

        world.ask(bob, request::play(1));
        world.advance(Duration::from_secs(2));
        let decoded = world.take(alice);
        assert!(sent_to_match(&decoded).is_none());
        assert!(matches!(queue_state(&decoded), Some(ClientState::InQueue {
            current_wait_seconds: 0..=3, search_properties, ..
        }) if search_properties[0] == 1 && search_properties[10] == 1));
        assert!(world.host.made.lock().unwrap().is_empty());
        // They can still meet after the new range has widened normally.
        world.advance(Duration::from_secs(36));
        assert!(sent_to_match(&world.take(alice)).is_some());
        assert!(sent_to_match(&world.take(bob)).is_some());
        let made = world.host.made.lock().unwrap().clone();
        assert_eq!(made.len(), 1);
        let setup = &made[0].1;
        // Current equipment is used both for matching and for rewards.
        assert_eq!(setup.match_level, 5);
        let profile = &setup
            .players
            .iter()
            .find(|player| player.account_id == world.ids[alice].0)
            .unwrap()
            .profile;
        let kit = profile.kits.iter().find(|kit| kit.id == kit_id).unwrap();
        assert_ne!(
            kit.state,
            survarium::v026g::match_msgs::static_info::KIT_NOT_LOADED
        );
        let (item_index, _) = kit.slots[usize::from(weapon_slot)].unwrap();
        assert_eq!(profile.items[usize::from(item_index)].dict_id, rifle_dict);
        world.assert_clean();
    }
}

#[test]
fn a_squads_search_restarts_together_when_a_members_level_changes() {
    let mut world = World::new(Duration::from_secs(600));
    let alice = world.connect("Alice");
    let bob = world.connect("Bob");
    let carol = world.connect("Carol");
    let rifle_dict = dict_id("ak_74m");
    let rounds_dict = dict_id("ammo_5.45x39_7n10");
    let (mut rifle, mut rounds) = (0, 0);
    world.change(alice, |account| {
        rifle = account.add_item(rifle_dict, FULL_CONDITION);
        rounds = account.add_item(rounds_dict, 60);
    });
    form_squad(&mut world, alice, bob);
    for player in [alice, bob, carol] {
        world.ask(player, request::play(1));
    }
    world.advance(Duration::from_secs(10));
    for player in [alice, bob, carol] {
        world.take(player);
    }
    world.ask(
        alice,
        request::move_item(2, rifle, rifle_dict, INVENTORY_SLOT, 7, 1),
    );
    world.ask(
        alice,
        request::move_item(2, rounds, rounds_dict, INVENTORY_SLOT, 8, 60),
    );
    world.advance(Duration::from_secs(2));
    let mut expected = [0; 20];
    expected[0] = 2;
    expected[10] = 1;
    assert_eq!(world.store.matchmaking().waiting_by_level::<20>(), expected);
    for player in [alice, bob] {
        assert!(matches!(
            queue_state(&world.take(player)),
            Some(ClientState::InQueue {
                place: 2,
                current_wait_seconds: 0..=3,
                ..
            })
        ));
    }
    assert!(matches!(
        queue_state(&world.take(carol)),
        Some(ClientState::InQueue {
            place: 1,
            current_wait_seconds: 10..,
            ..
        })
    ));
    assert!(
        world
            .store
            .squads()
            .of(world.ids[alice].0)
            .unwrap()
            .all_ready()
    );

    // Lowering the level also replaces the search, without breaking up
    // the squad or requiring its members to press Play again.
    world.advance(Duration::from_secs(10));
    world.ask(
        alice,
        request::move_item(2, rifle, rifle_dict, 7, INVENTORY_SLOT, 1),
    );
    world.advance(Duration::from_secs(2));
    expected[0] = 3;
    expected[10] = 0;
    assert_eq!(world.store.matchmaking().waiting_by_level::<20>(), expected);
    for player in [alice, bob] {
        assert!(matches!(
            queue_state(&world.take(player)),
            Some(ClientState::InQueue {
                current_wait_seconds: 0..=3,
                ..
            })
        ));
    }
    assert!(world.host.made.lock().unwrap().is_empty());
    world.assert_clean();
}

#[test]
fn equipment_moves_that_keep_the_level_preserve_the_search_wait() {
    let mut world = World::new(Duration::ZERO);
    let alice = world.connect("Alice");
    world.ask(alice, request::play(1));
    world.advance(Duration::from_secs(10));
    let before = world
        .core
        .player(world.clients[alice].0)
        .unwrap()
        .queued_since;
    // Moving the starter weapon out and back keeps the search level at zero.
    world.ask(alice, request::move_item(1, 1, 18, 7, INVENTORY_SLOT, 1));
    world.ask(alice, request::move_item(1, 1, 18, INVENTORY_SLOT, 7, 1));
    // A rejected move must not restart search either.
    world.ask(alice, request::move_item(1, 1, 18, 7, 0, 1));
    world.advance(Duration::from_secs(2));
    assert_eq!(
        world
            .core
            .player(world.clients[alice].0)
            .unwrap()
            .queued_since,
        before
    );
    assert!(matches!(
        queue_state(&world.take(alice)),
        Some(ClientState::InQueue {
            current_wait_seconds: 10..,
            ..
        })
    ));
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
fn the_maps_come_up_by_their_players_plays() {
    // Two players who play match after match: the maps they played less are
    // drawn more often, so in 32 matches every map comes up, and none much
    // more than the others.
    let mut world = World::new(Duration::ZERO);
    world.core.rng = survarium::v026g::drops::Rng::new(26);
    let alice = world.connect("Alice");
    let bob = world.connect("Bob");
    let mut seen = Vec::new();
    for round in 0..32 {
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
    let mut levels: Vec<u8> = maps::playable().map(|map| map.level).collect();
    levels.sort_unstable();
    levels.dedup();
    let counts: Vec<usize> = levels
        .iter()
        .map(|&level| seen.iter().filter(|&&each| each == level).count())
        .collect();
    assert!(
        counts.iter().all(|&count| (1..=8).contains(&count)),
        "{levels:?}: {counts:?}"
    );
    world.assert_clean();
}

#[test]
fn the_side_size_and_the_search_step_are_settings() {
    use crate::v026g::{parse_search_step, parse_team_size};

    let settings = Settings::local("127.0.0.1:0");
    // Two teams of up to 10 (the client's roster of 20); every level in range within the
    // default gather window of a minute (14 steps from 0..=1 to 15).
    assert_eq!(settings.team_size, 10);
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

#[test]
fn the_map_skew_is_a_setting() {
    use crate::v026g::parse_map_skew;

    assert_eq!(Settings::local("127.0.0.1:0").map_skew, 1.0);
    assert_eq!(parse_map_skew(" 0.5 "), Some(0.5));
    assert_eq!(parse_map_skew("0"), Some(0.0));
    assert_eq!(parse_map_skew("4"), Some(4.0));
    assert_eq!(parse_map_skew("4.1"), None);
    assert_eq!(parse_map_skew("-1"), None);
    assert_eq!(parse_map_skew("NaN"), None);
    assert_eq!(parse_map_skew("much"), None);
}

#[test]
fn the_servers_bots_take_the_places_after_the_players() {
    // `SRP_SERVER_BOTS=3`: three of the match server's own bots go into
    // each match made, after its players.
    let settings = Settings {
        server_bots: 3,
        ..Settings::local("127.0.0.1:0")
    };
    let mut world = World::with_settings(Duration::ZERO, settings);
    let alice = world.connect("Alice");
    let bob = world.connect("Bob");
    world.ask(alice, request::play(1));
    world.ask(bob, request::play(1));
    world.advance(Duration::from_secs(4));
    let made = world.host.made.lock().unwrap().clone();
    assert_eq!(made.len(), 1);
    let (_, setup) = &made[0];
    assert_eq!(setup.players.len(), 5);
    // The players keep the first places.
    for player in &setup.players[..2] {
        assert!(!player.driven && player.session_id != 0 && player.account_id != 0);
    }
    // The bots: no session, no account, a kit with a weapon, on the sides
    // so that neither has more than one place over the other.
    let mut sizes = [0_usize; 2];
    for (n, player) in setup.players.iter().enumerate() {
        sizes[usize::from(player.profile.team)] += 1;
        if n < 2 {
            continue;
        }
        assert!(player.driven && !player.bot);
        assert_eq!((player.session_id, player.account_id), (0, 0));
        assert_eq!(player.profile.name, format!("Bot-{}", n - 1));
        let kit = &player.profile.kits[0];
        assert_ne!(
            kit.state,
            survarium::v026g::match_msgs::static_info::KIT_NOT_LOADED
        );
        assert!(kit.slots[7].is_some(), "a weapon in slot 7");
    }
    assert!(sizes[0].abs_diff(sizes[1]) <= 1, "{sizes:?}");
    // The queue's roster has the bots' places too: a place taken later
    // counts from past them.
    let queued = world.store.matchmaking().matches();
    assert_eq!(queued.len(), 1);
    let formed = world
        .store
        .matchmaking()
        .formed_match(queued[0].0)
        .expect("the match");
    assert_eq!(formed.seats.len(), 5);
    assert!(
        formed.seats[2..]
            .iter()
            .all(|seat| seat.player.session_id == 0)
    );
}
