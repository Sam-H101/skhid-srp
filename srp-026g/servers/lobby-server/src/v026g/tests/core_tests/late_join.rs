use super::*;
use session::QueuePool;
use survarium::v026g::match_msgs::late_join::PATCH_REVISION;

fn growing(world: &mut World, name: &str) -> usize {
    let (account, _) = signed_in(world.store, name);
    let session = world
        .store
        .create_session_with_revision(account, PATCH_REVISION);
    world.connect_session((account, session))
}

fn running(world: &mut World) -> u32 {
    world
        .store
        .matchmaking()
        .set_growing_rules(Duration::ZERO, 1.0);
    let first = growing(world, "first");
    let second = growing(world, "second");
    world.ask(first, request::play(1));
    world.ask(second, request::play(1));
    world.advance(Duration::from_secs(2));
    let ticket = sent_to_match(&world.take(first)).unwrap().0;
    assert_eq!(sent_to_match(&world.take(second)).unwrap().0, ticket);
    assert_eq!(world.host.made.locked()[0].1.pool, QueuePool::GrowingRoster);
    world.host.admitting.locked().insert(ticket);
    ticket
}

fn another_match(world: &mut World, count: usize) -> u32 {
    world.host.admitting.locked().clear();
    world
        .store
        .matchmaking()
        .set_growing_rules(Duration::from_secs(120), 1.0);
    let mut clients = Vec::new();
    for index in 0..count {
        let client = growing(world, &format!("Other{index}"));
        world.ask(client, request::play(1));
        clients.push(client);
    }
    world
        .store
        .matchmaking()
        .set_growing_rules(Duration::ZERO, 1.0);
    world.advance(Duration::from_secs(2));
    let ticket = sent_to_match(&world.take(clients[0])).unwrap().0;
    for client in clients.into_iter().skip(1) {
        assert_eq!(sent_to_match(&world.take(client)).unwrap().0, ticket);
    }
    ticket
}

#[test]
fn changing_the_exponent_reverses_population_versus_remaining_time_preference() {
    for (exponent, prefer_fuller) in [(1.0, true), (0.5, false)] {
        let mut world = World::new(Duration::ZERO);
        let small = running(&mut world);
        let large = another_match(&mut world, 6);
        world
            .store
            .matchmaking()
            .set_growing_rules(Duration::ZERO, exponent);
        world.host.admitting.locked().extend([small, large]);
        world
            .host
            .remaining
            .locked()
            .insert(small, Duration::from_secs(720));
        world
            .host
            .remaining
            .locked()
            .insert(large, Duration::from_secs(360));
        let newcomer = growing(&mut world, "Weighted");
        world.ask(newcomer, request::play(1));
        world.advance(Duration::from_secs(2));
        assert_eq!(
            sent_to_match(&world.take(newcomer)).unwrap().0,
            if prefer_fuller { large } else { small }
        );
        assert_eq!(world.host.made.locked().len(), 2);
        world.assert_clean();
    }
}

#[test]
fn refused_best_match_leaves_the_whole_batch_available_for_another_match() {
    let mut world = World::new(Duration::ZERO);
    let fallback = running(&mut world);
    let preferred = another_match(&mut world, 6);
    world
        .store
        .matchmaking()
        .set_growing_rules(Duration::from_secs(120), 1.0);
    let alice = growing(&mut world, "Alice");
    let bob = growing(&mut world, "Bob");
    let solo = growing(&mut world, "Solo");
    form_squad(&mut world, alice, bob);
    world.ask(alice, request::play(1));
    world.ask(bob, request::play(1));
    world.ask(solo, request::play(1));
    assert_eq!(world.store.matchmaking().queued_players(), 3);
    world.host.admitting.locked().extend([fallback, preferred]);
    world
        .host
        .remaining
        .locked()
        .insert(preferred, Duration::from_secs(900));
    world.host.rejecting.locked().insert(preferred);
    world.advance(Duration::from_secs(2));
    for client in [alice, bob, solo] {
        assert_eq!(sent_to_match(&world.take(client)).unwrap().0, fallback);
    }
    let made = world.host.made.locked();
    assert_eq!(made[0].1.players.len(), 5);
    assert_eq!(made[1].1.players.len(), 6);
    assert_ne!(made[0].1.players[2].profile.squad, 0);
    assert_eq!(
        made[0].1.players[2].profile.squad,
        made[0].1.players[3].profile.squad
    );
    assert_eq!(world.store.matchmaking().queued_players(), 0);
    drop(made);
    world.assert_clean();
}

#[test]
fn cutoff_keeps_a_lone_joiner_queued_until_an_eligible_match_exists() {
    let mut world = World::new(Duration::ZERO);
    let ticket = running(&mut world);
    world
        .host
        .remaining
        .locked()
        .insert(ticket, Duration::from_secs(120));
    let newcomer = growing(&mut world, "Cutoff");
    world.ask(newcomer, request::play(1));
    world.advance(Duration::from_secs(2));
    assert!(sent_to_match(&world.take(newcomer)).is_none());
    assert_eq!(world.host.made.locked()[0].1.players.len(), 2);
    world
        .host
        .remaining
        .locked()
        .insert(ticket, Duration::from_millis(120_001));
    world.advance(Duration::from_secs(2));
    assert_eq!(sent_to_match(&world.take(newcomer)).unwrap().0, ticket);
    world.assert_clean();
}

#[test]
fn running_matches_are_ranked_by_score_before_starting_another_match() {
    let mut world = World::new(Duration::ZERO);
    let first = running(&mut world);
    world.host.admitting.locked().remove(&first);
    let a = growing(&mut world, "OtherA");
    let b = growing(&mut world, "OtherB");
    world.ask(a, request::play(1));
    world.ask(b, request::play(1));
    world.advance(Duration::from_secs(2));
    let second = sent_to_match(&world.take(a)).unwrap().0;
    assert_ne!(first, second);
    world.host.admitting.locked().extend([first, second]);
    world
        .host
        .remaining
        .locked()
        .insert(first, Duration::from_secs(180));
    world
        .host
        .remaining
        .locked()
        .insert(second, Duration::from_secs(600));
    let newcomer = growing(&mut world, "Scored");
    world.ask(newcomer, request::play(1));
    world.advance(Duration::from_secs(2));
    assert_eq!(sent_to_match(&world.take(newcomer)).unwrap().0, second);
    assert_eq!(world.host.made.locked().len(), 2);
    world.assert_clean();
}

#[test]
fn late_join_updates_team_averages_without_changing_the_reward_level() {
    let mut world = World::new(Duration::ZERO);
    let ticket = running(&mut world);
    let id = *world.core.matches.keys().next().unwrap();
    let running = world.core.matches.get_mut(&id).unwrap();
    running.teams[0].level_sum = 0;
    running.teams[1].level_sum = 100;
    let reward_level = world.host.made.locked()[0].1.match_level;
    world.host.admitting.locked().remove(&ticket);
    let newcomer = growing(&mut world, "LevelJoiner");
    world.ask(newcomer, request::play(1));
    let added = world
        .store
        .matchmaking()
        .queued_groups(QueuePool::GrowingRoster)[0]
        .population();
    assert!(added.level_sum < 50);
    world.host.admitting.locked().insert(ticket);
    world.advance(Duration::from_secs(2));
    let made = world.host.made.locked();
    assert_eq!(made[0].1.players[2].profile.team, 1);
    assert_eq!(made[0].1.match_level, reward_level);
    assert_eq!(
        world.core.matches[&id].teams[1].level_sum,
        100 + added.level_sum
    );
    assert_eq!(world.core.matches[&id].teams[1].players, 2);
    drop(made);
    world.assert_clean();
}

#[test]
fn departed_players_remain_in_population_and_level_counts() {
    let mut world = World::new(Duration::ZERO);
    running(&mut world);
    let id = *world.core.matches.keys().next().unwrap();
    let teams = world.core.matches[&id].teams;
    world.store.matchmaking().give_up_seat(world.ids[0].0);
    world.advance(Duration::from_secs(2));
    assert_eq!(world.core.matches[&id].teams, teams);
    assert_eq!(world.core.matches[&id].seats.len(), 2);
}

#[test]
fn play_sends_a_new_player_to_the_running_match_without_resending_existing_players() {
    let mut world = World::new(Duration::ZERO);
    let ticket = running(&mut world);
    let newcomer = growing(&mut world, "newcomer");
    world.ask(newcomer, request::play(1));
    world.advance(Duration::from_secs(2));
    assert_eq!(sent_to_match(&world.take(newcomer)).unwrap().0, ticket);
    assert!(sent_to_match(&world.take(0)).is_none());
    assert!(sent_to_match(&world.take(1)).is_none());
    let made = world.host.made.locked();
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].1.players.len(), 3);
    assert_eq!(made[0].1.players[2].account_id, world.ids[newcomer].0);
    let queue_match = world
        .store
        .matchmaking()
        .seat(world.ids[newcomer].1)
        .unwrap();
    assert_eq!(queue_match.1, 2);
    drop(made);
    world.assert_clean();
}

#[test]
fn ordinary_clients_form_a_separate_match_and_never_fill_a_growing_roster() {
    let mut world = World::new(Duration::ZERO);
    let ticket = running(&mut world);
    let alice = world.connect("Alice");
    world.ask(alice, request::play(1));
    world.advance(Duration::from_secs(2));
    assert!(sent_to_match(&world.take(alice)).is_none());
    assert_eq!(world.host.made.locked().len(), 1);
    let bob = world.connect("Bob");
    world.ask(bob, request::play(1));
    world.advance(Duration::from_secs(2));
    let other = sent_to_match(&world.take(alice)).unwrap().0;
    assert_ne!(other, ticket);
    assert_eq!(sent_to_match(&world.take(bob)).unwrap().0, other);
    assert_eq!(world.host.made.locked()[1].1.pool, QueuePool::Standard);
    world.assert_clean();
}

#[test]
fn a_squad_joins_one_team_as_one_reservation() {
    let mut world = World::new(Duration::ZERO);
    let ticket = running(&mut world);
    let alice = growing(&mut world, "Alice");
    let bob = growing(&mut world, "Bob");
    form_squad(&mut world, alice, bob);
    world.ask(alice, request::play(1));
    assert_eq!(world.host.made.locked()[0].1.players.len(), 2);
    world.ask(bob, request::play(1));
    world.advance(Duration::from_secs(1));
    assert_eq!(world.host.made.locked()[0].1.players.len(), 2);
    let opponent = growing(&mut world, "Opponent");
    world.ask(opponent, request::play(1));
    world.advance(Duration::from_secs(2));
    assert_eq!(sent_to_match(&world.take(opponent)).unwrap().0, ticket);
    let alice_match = sent_to_match(&world.take(alice)).unwrap();
    let bob_match = sent_to_match(&world.take(bob)).unwrap();
    assert_eq!(alice_match, bob_match);
    assert_eq!(alice_match.0, ticket);
    let made = world.host.made.locked();
    assert_eq!(made[0].1.players.len(), 5);
    let squad = made[0].1.players[2].profile.squad;
    assert_ne!(squad, 0);
    assert_eq!(made[0].1.players[3].profile.squad, squad);
    drop(made);
    world.assert_clean();
}

#[test]
fn a_squad_of_incompatible_clients_is_not_queued() {
    let mut world = World::new(Duration::ZERO);
    let alice = world.connect("Alice");
    let bob = growing(&mut world, "Bob");
    form_squad(&mut world, alice, bob);
    world.ask(alice, request::play(1));
    world.ask(bob, request::play(1));
    assert!(
        denial(
            &world.take(bob),
            survarium::v026g::lobby::requests::id::PLAY
        )
        .is_some()
    );
    assert_eq!(world.store.matchmaking().queued_players(), 0);
    assert!(world.host.made.locked().is_empty());
    world.assert_clean();
}

#[test]
fn a_busy_match_keeps_a_newcomer_queued_until_admission_reopens() {
    let mut world = World::new(Duration::ZERO);
    let ticket = running(&mut world);
    world.host.admitting.locked().remove(&ticket);
    let newcomer = growing(&mut world, "newcomer");
    world.ask(newcomer, request::play(1));
    world.advance(Duration::from_secs(2));
    assert!(sent_to_match(&world.take(newcomer)).is_none());
    assert_eq!(world.store.matchmaking().queued_players(), 1);
    assert_eq!(world.host.made.locked()[0].1.players.len(), 2);
    world.host.admitting.locked().insert(ticket);
    world.advance(Duration::from_secs(2));
    assert_eq!(sent_to_match(&world.take(newcomer)).unwrap().0, ticket);
    assert_eq!(world.store.matchmaking().queued_players(), 0);
    world.assert_clean();
}

#[test]
fn a_full_team_cannot_accept_a_squad_by_splitting_it() {
    let mut world = World::new(Duration::ZERO);
    world.core.settings.team_size = 2;
    let ticket = running(&mut world);
    let alice = growing(&mut world, "Alice");
    let bob = growing(&mut world, "Bob");
    form_squad(&mut world, alice, bob);
    world.ask(alice, request::play(1));
    world.ask(bob, request::play(1));
    world.advance(Duration::from_secs(2));
    assert!(sent_to_match(&world.take(alice)).is_none());
    assert!(sent_to_match(&world.take(bob)).is_none());
    assert_eq!(world.store.matchmaking().queued_players(), 2);
    assert_eq!(world.host.made.locked()[0].1.players.len(), 2);
    assert!(world.host.admitting.locked().contains(&ticket));
    world.assert_clean();
}

#[test]
fn changing_client_revision_cannot_reconnect_to_an_incompatible_match() {
    let mut world = World::new(Duration::ZERO);
    running(&mut world);
    let account = world.ids[0].0;
    let session = world.store.create_session(account);
    let replacement = world.connect_session((account, session));
    world.ask(replacement, request::play(1));
    let replies = world.take(replacement);
    assert!(denial(&replies, survarium::v026g::lobby::requests::id::PLAY).is_some());
    assert!(sent_to_match(&replies).is_none());
    assert_eq!(world.store.matchmaking().queued_players(), 0);
    world.assert_clean();
}
