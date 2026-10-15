//! A game older than the server takes (`patch_check.rs`): its sessions from
//! before are refused in the lobby. (The sign-in itself is
//! refused by the login server: `login_server::v026g`'s tests.)

use super::*;
use crate::v026g::patch_check::parse_min_patch_revision;
use survarium::v026g::lobby::requests::id;
use survarium::v026g::match_msgs::late_join::PATCH_REVISION as LATE_JOIN_REVISION;

/// A lobby that asks for patch revision 4.
fn checking() -> World {
    let mut world = World::new(Duration::ZERO);
    world.core.settings.min_patch_revision = 4;
    world
}

/// A client whose game signed in with `revision`: a session the store holds,
/// as one restored after a restart is.
fn game_of(world: &mut World, name: &str, revision: u8) -> usize {
    let (account, _) = signed_in(world.store, name);
    let session = world.store.create_session_with_revision(account, revision);
    let client = world.connect_session((account, session));
    world.take(client);
    client
}

#[test]
fn nobody_is_checked_unless_the_server_is_told_to() {
    let settings = Settings::local("127.0.0.1:0");
    assert_eq!(settings.min_patch_revision, 0);

    let mut world = World::new(Duration::ZERO);
    let old = game_of(&mut world, "Old", 0);
    assert!(world.client(old).model.is_signed_in());
    world.ask(old, request::play(1));
    assert_eq!(denial(&world.take(old), id::PLAY), None);
    assert_eq!(world.store.matchmaking().queued_players(), 1);
}

#[test]
fn an_older_games_session_from_before_is_refused_and_ended() {
    let mut world = checking();
    let (account, _) = signed_in(world.store, "Old");
    let session = world.store.create_session_with_revision(account, 3);
    let old = world.connect_session((account, session));
    // Refused as an ended session is (`FD`): not one push, so none of the
    // items its game may lack (the price list, the inventory, the account).
    assert!(world.client(old).disconnected);
    assert!(world.client(old).decoded.is_empty());
    assert_eq!(world.store.players_online(), 0);
    // The session is over: its next try is refused as well, and a restart
    // would not bring it back.
    assert_eq!(world.store.client_revision(session), None);
    let again = world.connect_session((account, session));
    assert!(world.client(again).disconnected);
    assert!(world.client(again).decoded.is_empty());
    world.advance(Duration::from_secs(3));
    assert!(world.core.links.is_empty());
    world.assert_clean();
}

#[test]
fn a_game_that_is_new_enough_signs_in_and_plays() {
    let mut world = checking();
    let current = game_of(&mut world, "Current", 4);
    let other = game_of(&mut world, "Other", 4);
    for client in [current, other] {
        assert!(world.client(client).model.is_signed_in());
        world.ask(client, request::play(1));
        assert_eq!(denial(&world.take(client), id::PLAY), None);
    }
    // The two of them are a match.
    assert_eq!(world.store.matchmaking().match_count(), 1);

    // A newer one plays too, in the queue of its kind.
    let newer = game_of(&mut world, "Newer", 5);
    world.ask(newer, request::play(1));
    assert_eq!(denial(&world.take(newer), id::PLAY), None);
    assert_eq!(world.store.matchmaking().queued_players(), 1);

    // The experimental late-join exe is over any least revision.
    let late = game_of(&mut world, "Late", LATE_JOIN_REVISION);
    world.ask(late, request::play(1));
    assert_eq!(denial(&world.take(late), id::PLAY), None);
    world.assert_clean();
}

#[test]
fn a_test_bot_is_never_checked() {
    let mut world = checking();
    world.accounts.set_bots(&["Bot".to_owned()]);
    let bot = game_of(&mut world, "Bot", 0);
    assert!(world.client(bot).model.is_signed_in());
    world.ask(bot, request::play(1));
    assert_eq!(denial(&world.take(bot), id::PLAY), None);
    assert_eq!(world.store.matchmaking().queued_players(), 1);
}

#[test]
fn the_least_revision_is_a_number_or_none() {
    for (text, min) in [
        (None, 0),
        (Some(""), 0),
        (Some(" "), 0),
        (Some("0"), 0),
        (Some(" 12 "), 12),
    ] {
        assert_eq!(parse_min_patch_revision(text), Ok(min), "{text:?}");
    }
    // A value it can't read stops srp, rather than let older games in.
    for text in ["refuse", "-1", "256", "12a"] {
        assert!(parse_min_patch_revision(Some(text)).is_err(), "{text}");
    }
}
