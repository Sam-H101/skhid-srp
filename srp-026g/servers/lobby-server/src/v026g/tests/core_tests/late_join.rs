use super::*;
use session::{QueuePool, STRENGTH_SCALE};
use survarium::v026g::match_msgs::late_join::{FIRST_REVISION, PATCH_REVISION};

use crate::v026g::LiveView;

fn growing(world: &mut World, name: &str) -> usize {
    let (account, _) = signed_in(world.store, name);
    let session = world
        .store
        .create_session_with_revision(account, PATCH_REVISION);
    world.connect_session((account, session))
}

/// A match of two being played that takes players. A player alone goes
/// into it at once: the tests of the wait set the patience themselves.
fn running(world: &mut World) -> u32 {
    world.core.settings.late_join_patience = Duration::ZERO;
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

/// The team of each roster place of the one running match, and its id in
/// the lobby's list.
fn seats(world: &World) -> (u32, Vec<u8>) {
    let (&id, running) = world.core.matches.iter().next().unwrap();
    (id, running.seats.iter().map(|seat| seat.1).collect())
}

fn joins(world: &mut World, name: &str, ticket: u32) -> Option<u8> {
    let newcomer = growing(world, name);
    world.ask(newcomer, request::play(1));
    world.advance(Duration::from_secs(2));
    let sent = sent_to_match(&world.take(newcomer))?;
    assert_eq!(sent.0, ticket);
    let made = world.host.made.locked();
    Some(made[0].1.players.last().unwrap().profile.team)
}

#[test]
fn a_late_player_goes_to_the_weaker_side_without_changing_the_reward_level() {
    let mut world = World::new(Duration::ZERO);
    let ticket = running(&mut world);
    let (id, teams) = seats(&world);
    // The second place counts for twenty gear levels more than the first.
    world.core.matches.get_mut(&id).unwrap().strengths = vec![0, 20 * STRENGTH_SCALE];
    let reward_level = world.host.made.locked()[0].1.match_level;
    assert_eq!(joins(&mut world, "LevelJoiner", ticket), Some(teams[0]));
    assert_eq!(world.host.made.locked()[0].1.match_level, reward_level);
    let running = &world.core.matches[&id];
    assert_eq!(running.seats.len(), 3);
    assert_eq!(running.strengths.len(), 3);
    world.assert_clean();
}

#[test]
fn a_player_who_left_is_not_counted_and_the_next_one_takes_its_side() {
    let mut world = World::new(Duration::ZERO);
    let ticket = running(&mut world);
    let (id, teams) = seats(&world);
    // The first place is by far the weaker side: a player goes there...
    world.core.matches.get_mut(&id).unwrap().strengths = vec![0, 20 * STRENGTH_SCALE];
    // ...unless the second place's player has left the match: then that
    // side has nobody.
    world.host.live.locked().insert(
        ticket,
        LiveView {
            present: 0b01,
            ..LiveView::default()
        },
    );
    assert_eq!(joins(&mut world, "Replacement", ticket), Some(teams[1]));
    assert_eq!(world.core.matches[&id].seats.len(), 3);
    world.assert_clean();
}

#[test]
fn a_player_alone_goes_to_the_side_that_trails() {
    for leader in 0..2_usize {
        let mut world = World::new(Duration::ZERO);
        let ticket = running(&mut world);
        // Patient again: an even match takes nobody alone.
        world.core.settings.late_join_patience = Duration::from_secs(60);
        let (id, teams) = seats(&world);
        world.core.matches.get_mut(&id).unwrap().map_id =
            survarium::v026g::maps::playable_in(survarium::v026g::maps::mode::BATTERIES)
                .next()
                .unwrap()
                .id;
        let mut points = [0, 0];
        points[leader] = 3;
        world.host.live.locked().insert(
            ticket,
            LiveView {
                present: 0b11,
                points: Some(points),
                kills: [0, 0],
                played: Some(Duration::from_secs(200)),
            },
        );
        let team = joins(&mut world, "Helper", ticket).expect("three batteries are a lead");
        assert_eq!(usize::from(team), 1 - leader);
        assert!(teams.contains(&team));
        world.assert_clean();
    }
}

#[test]
fn a_player_alone_waits_for_another_before_it_makes_an_even_match_uneven() {
    let mut world = World::new(Duration::ZERO);
    let ticket = running(&mut world);
    world.core.settings.late_join_patience = Duration::from_secs(60);
    let alone = growing(&mut world, "Alone");
    world.ask(alone, request::play(1));
    world.advance(Duration::from_secs(30));
    assert!(sent_to_match(&world.take(alone)).is_none());
    assert_eq!(world.host.made.locked()[0].1.players.len(), 2);
    // Another comes: one to each side, at once.
    let other = growing(&mut world, "Another");
    world.ask(other, request::play(1));
    world.advance(Duration::from_secs(2));
    assert_eq!(sent_to_match(&world.take(alone)).unwrap().0, ticket);
    assert_eq!(sent_to_match(&world.take(other)).unwrap().0, ticket);
    let made = world.host.made.locked();
    assert_ne!(
        made[0].1.players[2].profile.team,
        made[0].1.players[3].profile.team
    );
    drop(made);
    world.assert_clean();
}

#[test]
fn a_player_alone_goes_in_once_it_has_waited_long_enough() {
    let mut world = World::new(Duration::ZERO);
    let ticket = running(&mut world);
    world.core.settings.late_join_patience = Duration::from_secs(60);
    let alone = growing(&mut world, "Alone");
    world.ask(alone, request::play(1));
    world.advance(Duration::from_secs(58));
    assert!(sent_to_match(&world.take(alone)).is_none());
    world.host.admitting.locked().insert(ticket);
    world.advance(Duration::from_secs(4));
    assert_eq!(sent_to_match(&world.take(alone)).unwrap().0, ticket);
    world.assert_clean();
}

#[test]
fn the_side_that_trails_by_a_dramatic_lead_gets_two_players() {
    use survarium::v026g::maps::{self, mode};
    // Five batteries, or thirteen tickets: two and a half players.
    for (game_mode, lead, both) in [
        (mode::BATTERIES, 4, false),
        (mode::BATTERIES, 5, true),
        (mode::TEAM_DEATHMATCH, 12, false),
        (mode::TEAM_DEATHMATCH, 13, true),
    ] {
        let mut world = World::new(Duration::ZERO);
        let ticket = running(&mut world);
        world.core.settings.late_join_patience = Duration::from_secs(60);
        // The lead is read by the mode of the match's map.
        let (id, _) = seats(&world);
        world.core.matches.get_mut(&id).unwrap().map_id =
            maps::playable_in(game_mode).next().unwrap().id;
        world.host.live.locked().insert(
            ticket,
            LiveView {
                present: 0b11,
                points: Some([lead, 0]),
                kills: [0, 0],
                played: Some(Duration::from_secs(400)),
            },
        );
        // Both at once: held back from the match until both wait, and
        // not a match of their own.
        world.host.admitting.locked().remove(&ticket);
        world
            .store
            .matchmaking()
            .set_growing_rules(Duration::from_secs(120), 1.0);
        let first = growing(&mut world, "HelperA");
        let second = growing(&mut world, "HelperB");
        world.ask(first, request::play(1));
        world.ask(second, request::play(1));
        world.host.admitting.locked().insert(ticket);
        world.advance(Duration::from_secs(2));
        assert_eq!(sent_to_match(&world.take(first)).unwrap().0, ticket);
        assert_eq!(sent_to_match(&world.take(second)).unwrap().0, ticket);
        let made = world.host.made.locked();
        let teams = [
            made[0].1.players[2].profile.team,
            made[0].1.players[3].profile.team,
        ];
        assert_eq!(
            teams == [1, 1],
            both,
            "mode {game_mode}, a lead of {lead}: {teams:?}"
        );
        drop(made);
        world.assert_clean();
    }
}

#[test]
fn a_released_revision_plays_in_the_growing_queue_and_an_older_one_does_not() {
    let mut world = World::new(Duration::ZERO);
    world
        .store
        .matchmaking()
        .set_growing_rules(Duration::ZERO, 1.0);
    for (name, revision) in [("Five", FIRST_REVISION), ("Six", FIRST_REVISION + 1)] {
        let (account, _) = signed_in(world.store, name);
        let session = world.store.create_session_with_revision(account, revision);
        let client = world.connect_session((account, session));
        world.ask(client, request::play(1));
    }
    world.advance(Duration::from_secs(2));
    assert_eq!(world.host.made.locked()[0].1.pool, QueuePool::GrowingRoster);
    // Revision 4: the standard queue, where it needs its own kind.
    let (account, _) = signed_in(world.store, "Four");
    let session = world
        .store
        .create_session_with_revision(account, FIRST_REVISION - 1);
    let old = world.connect_session((account, session));
    world.ask(old, request::play(1));
    world.advance(Duration::from_secs(2));
    assert!(sent_to_match(&world.take(old)).is_none());
    assert_eq!(world.host.made.locked().len(), 1);
}

#[test]
fn the_log_gets_the_sides_of_a_match_being_played_each_minute() {
    let mut world = World::new(Duration::ZERO);
    let ticket = running(&mut world);
    let (id, _) = seats(&world);
    // Nothing is known of a match the host does not describe.
    world.advance(Duration::from_secs(2));
    assert_eq!(world.core.matches[&id].predicted_at, None);
    world.host.live.locked().insert(
        ticket,
        LiveView {
            present: 0b11,
            points: Some([2, 1]),
            kills: [7, 5],
            played: Some(Duration::from_secs(61)),
        },
    );
    world.advance(Duration::from_secs(2));
    let first = world.core.matches[&id].predicted_at.expect("a line");
    world.advance(Duration::from_secs(30));
    assert_eq!(world.core.matches[&id].predicted_at, Some(first));
    world.advance(Duration::from_secs(40));
    assert!(world.core.matches[&id].predicted_at.unwrap() > first);
}

#[test]
fn queued_strengths_refresh_before_a_running_match_takes_the_player() {
    let mut world = World::new(Duration::ZERO);
    let ticket = running(&mut world);
    let (id, _) = seats(&world);
    world.host.admitting.locked().remove(&ticket);
    let newcomer = growing(&mut world, "ChangedLevel");
    world.ask(newcomer, request::play(1));
    assert_eq!(
        world
            .store
            .matchmaking()
            .queued_groups(QueuePool::GrowingRoster)[0]
            .worth()
            .strength,
        0
    );
    // A rating change outside an inventory request must be seen by the next
    // queue poll too: the player then counts for its five gear levels.
    world.account(newcomer).rating = 15_000;
    assert_eq!(
        session::v026g::matchmaking::level(&world.account(newcomer), 0).min,
        5
    );
    world.host.admitting.locked().insert(ticket);
    world.advance(Duration::from_secs(2));
    assert_eq!(sent_to_match(&world.take(newcomer)).unwrap().0, ticket);
    assert_eq!(world.core.matches[&id].strengths[2], 5 * STRENGTH_SCALE);
    world.assert_clean();
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
