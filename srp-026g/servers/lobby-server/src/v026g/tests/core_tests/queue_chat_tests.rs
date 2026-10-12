//! The lines in the general chat about the search (`queue_chat.rs`): who
//! joined it, who left it, and with whom a match starts.

use super::*;

/// The general-channel lines of an account-type sender with no name, in the
/// order they came.
fn lines(decoded: &[Decoded]) -> Vec<String> {
    decoded
        .iter()
        .filter_map(|message| match message {
            Decoded::ChatText {
                sender_type: 5,
                sender_id: 0,
                sender_name,
                channel: channel::GENERAL,
                body,
                ..
            } if sender_name.is_empty() => Some(body.clone()),
            _ => None,
        })
        .collect()
}

fn listening(world: &mut World, names: &[&str]) -> Vec<usize> {
    let clients: Vec<usize> = names.iter().map(|name| world.connect(name)).collect();
    for &client in &clients {
        world.ask(client, request::subscribe());
        world.take(client);
    }
    clients
}

#[test]
fn a_join_and_a_leave_are_said_to_everybody_who_listens() {
    let mut world = World::new(Duration::from_secs(60));
    let [alice, bob, carol] = listening(&mut world, &["Alice", "Bob", "Carol"])[..] else {
        unreachable!()
    };
    // Dave is online but never opened the general chat.
    let dave = world.connect("Dave");
    world.take(dave);

    world.ask(alice, request::play(1));
    for client in [alice, bob, carol] {
        assert_eq!(
            lines(&world.take(client)),
            ["Alice joined the search (1 waiting)"]
        );
    }
    assert!(lines(&world.take(dave)).is_empty());

    world.ask(alice, request::leave(1, false));
    for client in [alice, bob, carol] {
        assert_eq!(
            lines(&world.take(client)),
            ["Alice left the search (0 waiting)"]
        );
    }
    world.assert_clean();
}

#[test]
fn a_match_is_announced_to_everybody_with_whom_its_players_play() {
    let mut world = World::new(Duration::from_secs(60));
    let [alice, bob, carol] = listening(&mut world, &["Alice", "Bob", "Carol"])[..] else {
        unreachable!()
    };
    world.ask(alice, request::play(1));
    world.ask(bob, request::play(1));
    world.take(alice);
    world.take(bob);
    world.take(carol);
    world.advance(Duration::from_secs(62));

    let said = |decoded: &[Decoded]| lines(decoded);
    let (to_alice, to_bob) = (said(&world.take(alice)), said(&world.take(bob)));
    assert_eq!(to_alice, ["Match found: Alice vs Bob"]);
    assert_eq!(to_bob, to_alice);
    // Carol is not in it, and hears it too.
    assert_eq!(said(&world.take(carol)), to_alice);
    world.assert_clean();
}

#[test]
fn a_squad_is_one_line_and_its_leaving_too() {
    let mut world = World::new(Duration::from_secs(60));
    let [alice, bob, carol] = listening(&mut world, &["Alice", "Bob", "Carol"])[..] else {
        unreachable!()
    };
    form_squad(&mut world, alice, bob);
    world.take(alice);
    world.take(bob);
    world.take(carol);

    world.ask(alice, request::play(1));
    world.ask(bob, request::play(1));
    assert_eq!(
        lines(&world.take(carol)),
        ["Alice, Bob joined the search as a squad (2 waiting)"]
    );
    world.ask(bob, request::leave(1, false));
    assert_eq!(
        lines(&world.take(carol)),
        ["Bob left the search with their squad (0 waiting)"]
    );
}

#[test]
fn a_player_who_presses_play_in_a_loop_is_not_repeated() {
    let mut world = World::new(Duration::from_secs(60));
    let [alice, bob] = listening(&mut world, &["Alice", "Bob"])[..] else {
        unreachable!()
    };
    world.ask(alice, request::play(1));
    world.ask(alice, request::leave(1, false));
    // Ten seconds later: the join is not said again, and neither is its
    // leave.
    world.advance(Duration::from_secs(10));
    world.ask(alice, request::play(1));
    world.ask(alice, request::leave(1, false));
    assert_eq!(
        lines(&world.take(bob)),
        [
            "Alice joined the search (1 waiting)",
            "Alice left the search (0 waiting)"
        ]
    );
    // After 30 s it is said again.
    world.advance(Duration::from_secs(40));
    world.take(bob);
    world.ask(alice, request::play(1));
    assert_eq!(
        lines(&world.take(bob)),
        ["Alice joined the search (1 waiting)"]
    );
}

#[test]
fn the_lines_can_be_turned_off() {
    let mut world = World::new(Duration::from_secs(60));
    world.core.settings.queue_chat = false;
    let [alice, bob] = listening(&mut world, &["Alice", "Bob"])[..] else {
        unreachable!()
    };
    world.ask(alice, request::play(1));
    world.ask(bob, request::play(1));
    world.advance(Duration::from_secs(62));
    world.ask(alice, request::leave(1, false));
    assert!(lines(&world.take(alice)).is_empty());
    assert!(lines(&world.take(bob)).is_empty());
}
