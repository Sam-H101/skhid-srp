//! "Watch replay" from the History of srp's changed lobby movie: the ask
//! (`3D` with `REPLAY_ASK`) and the polls (`3B` with `REPLAY_START`),
//! answered with the asker's own `R1` row and, when ready, the link (`9E`)
//! just before it. Crafted requests get the asker's own answer and no
//! token they are not given.

use std::path::PathBuf;

use session::v026g::replays::{ReplaySettings, Replays, Sidecar};
use survarium::v026g::lobby::{REPLAY_ASK, REPLAY_START};

use super::*;

const MATCH: u32 = 0x1F07_C8FB;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "srp-lobby-replay-{name}-{}-{}",
        std::process::id(),
        Replays::now_unix()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Replays in `root` with a made replay of `MATCH` that `players` played.
fn replays(root: &std::path::Path, players: &[u32]) -> &'static Replays {
    let settings = ReplaySettings {
        replay_dir: root.join("replays"),
        record_dir: root.join("recordings"),
        modes_dir: root.join("modes"),
        max_recording_bytes: 1 << 20,
        max_replay_bytes: 1 << 20,
    }
    .checked()
    .unwrap();
    let replays: &'static Replays = Box::leak(Box::new(Replays::new(Some(settings), 6)));
    std::fs::write(replays.sur_path(MATCH).unwrap(), b"0123456789ab").unwrap();
    let sidecar = Sidecar {
        match_id: MATCH,
        size: 12,
        sha256: "cd".repeat(32),
        revisions: vec![Some(14), Some(12)],
        account_ids: players.to_vec(),
        made_at: 0,
    };
    std::fs::write(replays.json_path(MATCH).unwrap(), sidecar.to_json()).unwrap();
    replays
}

/// A player whose game signed in as `revision`.
fn player(world: &mut World, name: &str, revision: u8) -> usize {
    let (account, _) = signed_in(world.store, name);
    let session = world.store.create_session_with_revision(account, revision);
    world.connect_session((account, session))
}

/// The movie's poll: what came back, as `(9E text, R1 row)`.
fn poll(world: &mut World, index: usize) -> (Option<String>, String) {
    world.take(index);
    world.ask(index, request::ranking(REPLAY_START, 1));
    let decoded = world.take(index);
    let (link, rows) = match &decoded[..] {
        [Decoded::EloList(rows)] => (None, rows),
        [Decoded::VideoLink { show, url }, Decoded::EloList(rows)] => {
            assert!(!show, "the link is never shown in the lobby");
            (Some(url.clone()), rows)
        }
        other => panic!("not an R1 answer: {other:?}"),
    };
    let [(name, place, 0, 0)] = &rows[..] else {
        panic!("not one row: {rows:?}");
    };
    assert_eq!(*place, REPLAY_START);
    (link, name.clone())
}

#[test]
fn replays_off_answer_off_and_ask_nothing() {
    let mut world = World::new(Duration::from_secs(60));
    let alice = player(&mut world, "Alice", 14);
    world.take(alice);
    world.ask(alice, request::player_details(REPLAY_ASK, MATCH));
    // The ask itself is not answered: the polls are.
    assert!(world.take(alice).is_empty());
    assert_eq!(poll(&mut world, alice), (None, "R1;off;00000000;0".into()));
    world.assert_clean();
}

#[test]
fn a_player_of_the_match_gets_the_link_before_a_ready_row() {
    let root = temp("ready");
    let mut world = World::new(Duration::from_secs(60));
    let alice = player(&mut world, "Alice", 14);
    let bob = player(&mut world, "Bob", 14);
    let carol = player(&mut world, "Carol", 12);
    let dave = player(&mut world, "Dave", 14);
    let desk = replays(&root, &[world.ids[alice].0, world.ids[carol].0]);
    world.core.replays = Some(desk);

    // Nobody asked yet: `none`, for the match 0.
    assert_eq!(poll(&mut world, alice), (None, "R1;none;00000000;0".into()));
    world.ask(alice, request::player_details(REPLAY_ASK, MATCH));
    assert!(world.take(alice).is_empty());
    let (link, row) = poll(&mut world, alice);
    assert_eq!(row, "R1;ready;1f07c8fb;12");
    let link = link.expect("a 9E before the ready row");
    let parts: Vec<&str> = link.split(' ').collect();
    assert_eq!(parts.len(), 5, "{link}");
    assert_eq!(
        (parts[0], parts[1], parts[3], parts[4]),
        ("replay", "1f07c8fb", "cd".repeat(32).as_str(), "12")
    );
    let token = parts[2];
    assert_eq!(token.len(), 32);
    assert!(link.chars().count() <= 245);
    // The token downloads that match, and only from the web page.
    assert_eq!(
        desk.take_download(token, false, world.now)
            .map(|download| download.match_id),
        Ok(MATCH)
    );

    // Bob did not play it: `not_yours`, no link; his polls are his own.
    world.ask(bob, request::player_details(REPLAY_ASK, MATCH));
    assert_eq!(
        poll(&mut world, bob),
        (None, "R1;not_yours;1f07c8fb;0".into())
    );
    // Carol played it with revision 12 (the patches' mount, no
    // replay link): her game cannot start a replay.
    world.ask(carol, request::player_details(REPLAY_ASK, MATCH));
    assert_eq!(
        poll(&mut world, carol),
        (None, "R1;old_game;1f07c8fb;0".into())
    );
    // Dave never asked: his crafted poll shows nobody's answer.
    assert_eq!(poll(&mut world, dave), (None, "R1;none;00000000;0".into()));
    // Another match, of no recording: gone.
    world.advance(Duration::from_secs(3));
    world.ask(alice, request::player_details(REPLAY_ASK, 0x1234));
    assert_eq!(poll(&mut world, alice), (None, "R1;gone;00001234;0".into()));
    // Asked again at once for another: busy (one ask in two seconds).
    world.ask(alice, request::player_details(REPLAY_ASK, MATCH));
    assert_eq!(poll(&mut world, alice), (None, "R1;busy;1f07c8fb;0".into()));
    // An ordinary row of a results screen is still that (none kept here).
    world.ask(alice, request::player_details(2, MATCH));
    assert!(world.take(alice).is_empty());
    world.assert_clean();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_preparing_row_carries_the_place_in_the_conversion_line() {
    let root = temp("line");
    let mut world = World::new(Duration::from_secs(60));
    let alice = player(&mut world, "Alice", 14);
    let bob = player(&mut world, "Bob", 14);
    let desk = replays(&root, &[]);
    world.core.replays = Some(desk);
    // Two kept recordings with no replay yet (MATCH has one).
    let record_dir = &desk.settings().unwrap().record_dir;
    for id in [0x0A, 0x0B] {
        std::fs::write(
            record_dir.join(format!("rec026g_20261015-120000_{id:08x}.srprec")),
            b"x",
        )
        .unwrap();
        desk.recording_kept(id);
    }
    world.ask(alice, request::player_details(REPLAY_ASK, 0x0A));
    assert_eq!(
        poll(&mut world, alice),
        (None, "R1;preparing;0000000a;1".into())
    );
    world.ask(bob, request::player_details(REPLAY_ASK, 0x0B));
    assert_eq!(
        poll(&mut world, bob),
        (None, "R1;preparing;0000000b;2".into())
    );
    // The first is taken up: still 1 for it, still 2 behind it.
    assert_eq!(desk.next_job(), Some(0x0A));
    assert_eq!(
        poll(&mut world, alice),
        (None, "R1;preparing;0000000a;1".into())
    );
    assert_eq!(
        poll(&mut world, bob),
        (None, "R1;preparing;0000000b;2".into())
    );
    // A `ready` row keeps the size there.
    world.advance(Duration::from_secs(3));
    world.ask(alice, request::player_details(REPLAY_ASK, MATCH));
    let (link, row) = poll(&mut world, alice);
    assert!(link.is_some());
    assert_eq!(row, "R1;ready;1f07c8fb;12");
    world.assert_clean();
    let _ = std::fs::remove_dir_all(&root);
}
