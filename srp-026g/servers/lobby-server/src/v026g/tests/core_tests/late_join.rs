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
    world.advance(Duration::from_secs(2));
    let alice_match = sent_to_match(&world.take(alice)).unwrap();
    let bob_match = sent_to_match(&world.take(bob)).unwrap();
    assert_eq!(alice_match, bob_match);
    assert_eq!(alice_match.0, ticket);
    let made = world.host.made.locked();
    assert_eq!(made[0].1.players.len(), 4);
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
