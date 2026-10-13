//! A game older than the server asks for (`patch_check.rs`): it is told to
//! update, and with `refuse` its Play is refused.

use super::*;
use crate::v026g::PatchCheck;
use survarium::v026g::lobby::requests::id;
use survarium::v026g::match_msgs::late_join::PATCH_REVISION as LATE_JOIN_REVISION;

/// The game's own "You're using an outdated version of the game.".
const OUTDATED: &str = "st_login_menu_status_invalid_version";

/// A lobby that asks for patch revision 4.
fn checking(check: PatchCheck) -> World {
    let mut world = World::new(Duration::ZERO);
    world.core.settings.patch_check = check;
    world.core.settings.min_patch_revision = 4;
    world.core.settings.update_hint = "Update: start Skhid.bat".to_owned();
    world
}

/// A client whose game signed in with `revision`, with the general chat open
/// if `listens`.
fn game_of(world: &mut World, name: &str, revision: u8, listens: bool) -> usize {
    let (account, _) = signed_in(world.store, name);
    let session = world.store.create_session_with_revision(account, revision);
    let client = world.connect_session((account, session));
    world.take(client);
    if listens {
        world.ask(client, request::subscribe());
    }
    client
}

/// The update hints among the lines of the general chat (the lines about
/// the search, `queue_chat`, are of the same sender).
fn hints(decoded: &[Decoded]) -> Vec<String> {
    decoded
        .iter()
        .filter_map(|message| match message {
            Decoded::ChatText {
                sender_type: 5,
                sender_id: 0,
                channel: channel::GENERAL,
                body,
                ..
            } if body.starts_with("Update") => Some(body.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn nobody_is_checked_unless_the_server_is_told_to() {
    let settings = Settings::local("127.0.0.1:0");
    assert_eq!(settings.patch_check, PatchCheck::Off);
    assert_eq!(settings.min_patch_revision, 4);
    assert_eq!(settings.outdated_text, OUTDATED);

    let mut world = World::new(Duration::ZERO);
    let old = game_of(&mut world, "Old", 0, true);
    assert!(hints(&world.take(old)).is_empty());
    world.ask(old, request::play(1));
    let replies = world.take(old);
    assert_eq!(denial(&replies, id::PLAY), None);
    assert_eq!(world.store.matchmaking().queued_players(), 1);
}

#[test]
fn an_older_game_is_refused_in_the_games_words_and_told_what_to_do() {
    let mut world = checking(PatchCheck::Refuse);
    let old = game_of(&mut world, "Old", 3, true);
    // Told once it opens the general chat.
    assert_eq!(hints(&world.take(old)), ["Update: start Skhid.bat"]);

    world.ask(old, request::play(1));
    let replies = world.take(old);
    assert_eq!(denial(&replies, id::PLAY).as_deref(), Some(OUTDATED));
    assert_eq!(world.store.matchmaking().queued_players(), 0);
    // Not said twice in half a minute, and again after it.
    assert!(hints(&replies).is_empty());
    world.advance(Duration::from_secs(31));
    world.take(old);
    world.ask(old, request::play(1));
    let replies = world.take(old);
    assert_eq!(denial(&replies, id::PLAY).as_deref(), Some(OUTDATED));
    assert_eq!(hints(&replies), ["Update: start Skhid.bat"]);
    world.assert_clean();
}

#[test]
fn a_game_that_is_new_enough_plays_and_is_told_nothing() {
    let mut world = checking(PatchCheck::Refuse);
    let current = game_of(&mut world, "Current", 4, true);
    let other = game_of(&mut world, "Other", 4, true);
    for client in [current, other] {
        assert!(hints(&world.take(client)).is_empty());
        world.ask(client, request::play(1));
        assert_eq!(denial(&world.take(client), id::PLAY), None);
    }
    // The two of them are a match.
    assert_eq!(world.store.matchmaking().match_count(), 1);

    // A newer one plays too, in the queue of its kind.
    let newer = game_of(&mut world, "Newer", 5, true);
    assert!(hints(&world.take(newer)).is_empty());
    world.ask(newer, request::play(1));
    assert_eq!(denial(&world.take(newer), id::PLAY), None);
    assert_eq!(world.store.matchmaking().queued_players(), 1);

    // The experimental late-join exe is over any least revision.
    let late = game_of(&mut world, "Late", LATE_JOIN_REVISION, true);
    world.take(late);
    world.ask(late, request::play(1));
    let replies = world.take(late);
    assert_eq!(denial(&replies, id::PLAY), None);
    assert!(hints(&replies).is_empty());
}

#[test]
fn a_game_that_does_not_listen_to_the_chat_still_gets_the_message_box() {
    let mut world = checking(PatchCheck::Refuse);
    let old = game_of(&mut world, "Old", 0, false);
    world.ask(old, request::play(1));
    let replies = world.take(old);
    assert_eq!(denial(&replies, id::PLAY).as_deref(), Some(OUTDATED));
    assert!(hints(&replies).is_empty());
}

#[test]
fn warn_tells_an_older_game_and_lets_it_play() {
    let mut world = checking(PatchCheck::Warn);
    let old = game_of(&mut world, "Old", 2, true);
    assert_eq!(hints(&world.take(old)), ["Update: start Skhid.bat"]);
    world.ask(old, request::play(1));
    assert_eq!(denial(&world.take(old), id::PLAY), None);
    assert_eq!(world.store.matchmaking().queued_players(), 1);
}

#[test]
fn clan_team_play_is_refused_the_same_way() {
    let mut world = checking(PatchCheck::Refuse);
    let old = game_of(&mut world, "Old", 0, false);
    world.ask(old, (id::CLAN_PLAY, 1_u32.to_le_bytes().to_vec()));
    assert_eq!(
        denial(&world.take(old), id::CLAN_PLAY).as_deref(),
        Some(OUTDATED)
    );
}

#[test]
fn the_message_box_can_be_a_sentence_and_the_hint_can_be_left_out() {
    let mut world = checking(PatchCheck::Refuse);
    world.core.settings.outdated_text = "Update the game: start Skhid.bat".to_owned();
    world.core.settings.update_hint = String::new();
    let old = game_of(&mut world, "Old", 0, true);
    assert!(hints(&world.take(old)).is_empty());
    world.ask(old, request::play(1));
    let replies = world.take(old);
    assert_eq!(
        denial(&replies, id::PLAY).as_deref(),
        Some("Update the game: start Skhid.bat")
    );
    assert!(hints(&replies).is_empty());
}

#[test]
fn a_test_bot_is_never_checked() {
    let mut world = checking(PatchCheck::Refuse);
    world.accounts.set_bots(&["Bot".to_owned()]);
    let bot = game_of(&mut world, "Bot", 0, true);
    assert!(hints(&world.take(bot)).is_empty());
    world.ask(bot, request::play(1));
    assert_eq!(denial(&world.take(bot), id::PLAY), None);
    assert_eq!(world.store.matchmaking().queued_players(), 1);
}

#[test]
fn the_setting_reads_off_warn_and_refuse() {
    for (text, check) in [
        ("off", PatchCheck::Off),
        (" Warn ", PatchCheck::Warn),
        ("refuse", PatchCheck::Refuse),
        ("on", PatchCheck::Refuse),
    ] {
        assert_eq!(PatchCheck::parse(text), Some(check), "{text}");
    }
    assert_eq!(PatchCheck::parse("maybe"), None);
    // The default hint is one chat line, and names the file to start.
    let hint = crate::v026g::patch_check::DEFAULT_UPDATE_HINT;
    assert!(hint.contains("Skhid.bat") && hint.len() <= 255, "{hint}");
}
