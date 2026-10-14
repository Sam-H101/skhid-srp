//! The server's own bots (`MatchPlayer::driven`, `SRP_SERVER_BOTS`): places
//! with no client, entered and played by the match server
//! (`game/driven.rs`).

use survarium::v026g::match_msgs::rules::state;
use survarium::v026g::match_msgs::snapshot::Tdm;
use survarium::v026g::match_msgs::{PlayerInput, ServerMessage, TICK_MS};

use super::{Bench, MAP_TDM, setup};
use crate::v026g::drive::key;
use crate::v026g::{MatchEnd, MatchRegistry, Settings};

/// What a script's inputs must be: on the grid, with no bit a bot never
/// sends.
fn assert_bot_inputs(inputs: &[PlayerInput]) {
    for input in inputs {
        assert_eq!(input.time.ms, None, "on the 50 ms grid: {input:?}");
        assert_eq!(input.actions & key::NEVER, 0, "{input:?}");
    }
}

#[test]
fn the_servers_bot_enters_once_a_real_client_plays_and_its_inputs_reach_it() {
    let mut bench = Bench::with_driven(2, MAP_TDM, Settings::default(), &[1]);

    // Nobody simulates: the bot's place waits, with no client and no clock.
    bench.run(2_000);
    assert_eq!(bench.stages(), "");

    // The real client starts the match; the bot enters after it, ahead of
    // its clock, and the client hears it.
    bench.join(0);
    assert!(bench.until(2_000, |bench| bench.clients[0].entered() == [0, 1]));
    assert_eq!(bench.stages(), "L-");

    // Its script starts 2 s after its entry: the first input walks.
    assert!(bench.until(4_000, |bench| !bench.clients[0].relays_of(1).is_empty()));
    let first = bench.clients[0].relays_of(1)[0];
    assert_eq!(first.actions, key::FORWARD);
    assert_eq!(first.rotation, [0; 4]);

    // A whole run of the script: every input reaches the client, none of
    // them late (it would be moved or dropped, a fault of the client's
    // world), and the bot is no witness: one client vouches for the world.
    bench.run_long(30_000);
    let relays = bench.clients[0].relays_of(1);
    assert!(relays.len() >= 15, "{} inputs", relays.len());
    assert_bot_inputs(&relays);
    assert!(relays.iter().any(|input| input.actions == key::FIRE));
    assert!(relays.iter().any(|input| input.rotation != [0; 4]));
    assert_eq!(bench.game().rule_state(), state::IN_PROCESS);
    assert_eq!(bench.game().trusted_by(), 1);
    assert_eq!(bench.game().resyncs(0), 0);
    assert_eq!(bench.clients[0].model.syncs, 1);
    let moved = bench.clients[0].got(|message| match message {
        ServerMessage::MoveInput { .. } | ServerMessage::DiscardInput { .. } => Some(()),
        _ => None,
    });
    assert!(moved.is_empty());

    // The bot's team (1) runs out of tickets: the end is the real
    // client's world, and the bot's row says what it is.
    let at = bench.clock() / TICK_MS * TICK_MS + 100;
    bench.clients[0].sim.tdm_from = Some((
        at,
        Tdm {
            tickets: [58, 0],
            alive_players_mask: 0b01,
            active_players_mask: 0b11,
        },
    ));
    assert!(bench.until(12_000, |bench| bench.clients[0].told_to_leave));
    let result = bench.results.try_recv().expect("a result");
    assert_eq!(result.end, MatchEnd::ModeRule);
    assert_eq!(result.winner, Some(0));
    let bot = &result.players[1];
    assert!(bot.driven && bot.entered && !bot.left);
    assert_eq!((bot.session_id, bot.account_id), (0, 0));
    assert!(!result.players[0].driven);
    bench.assert_no_faults();
}

#[test]
fn a_client_that_comes_later_gets_the_bots_entry_and_inputs_from_the_history() {
    let mut bench = Bench::with_driven(3, MAP_TDM, Settings::default(), &[2]);
    bench.join(0);
    bench.until(2_000, |bench| bench.clients[0].entered() == [0, 2]);
    assert_eq!(bench.clients[0].entered(), [0, 2], "{}", bench.stages());
    // The bot has walked for a while before the second player comes.
    bench.run_long(6_000);
    assert!(!bench.clients[0].relays_of(2).is_empty());

    bench.join(1);
    // The bot's entry is in the world it starts from; its own comes after.
    assert!(bench.until(2_000, |bench| bench.clients[1].entered() == [1]));
    assert_eq!(bench.clients[0].entered(), [0, 2, 1]);
    bench.run_long(20_000);
    // From its start on it gets what the first client gets.
    let first = bench.clients[0].relays_of(2);
    let second = bench.clients[1].relays_of(2);
    assert!(!second.is_empty());
    assert!(first.ends_with(&second[second.len().saturating_sub(5)..]));
    assert_bot_inputs(&second);
    assert_eq!(bench.game().trusted_by(), 2);
    assert_eq!((bench.game().resyncs(0), bench.game().resyncs(1)), (0, 0));
    bench.assert_no_faults();
}

#[test]
fn a_match_left_to_the_servers_bot_is_over() {
    let mut bench = Bench::with_driven(2, MAP_TDM, Settings::default(), &[1]);
    bench.join(0);
    assert!(bench.until(2_000, |bench| bench.clients[0].entered() == [0, 1]));
    bench.run(3_000);
    // As in a match of real players only: the last one gone, it is over.
    bench.clients[0].leave();
    assert!(bench.until(5_000, |bench| bench.results.try_recv().is_ok()));
    assert!(bench.until(10_000, |bench| bench.registry.match_count() == 0));
    bench.assert_no_faults();
}

#[test]
fn a_match_only_the_servers_bots_are_in_is_never_started_and_is_given_up() {
    // No client ever comes: the match is never run, and given up as one
    // nobody came to.
    let mut bench = Bench::with_driven(2, MAP_TDM, Settings::default(), &[0, 1]);
    assert!(bench.until(150_000, |bench| bench.results.try_recv().is_ok()));
    assert_eq!(bench.stages(), "");
    assert_eq!(bench.registry.match_count(), 0);
}

#[test]
fn the_servers_bots_have_no_seats_and_share_session_zero() {
    let registry = MatchRegistry::new("127.0.0.1", 1236);
    let mut setup = setup(4, MAP_TDM);
    for place in [2, 3] {
        let player = &mut setup.players[place];
        (player.driven, player.session_id, player.account_id) = (true, 0, 0);
    }
    registry
        .create(setup)
        .expect("two bots of session 0 are no clash");
    assert!(registry.seat(0).is_none());
    // Nor can a bot be put into a match being played.
    let mut late = super::setup(1, MAP_TDM).players.remove(0);
    late.driven = true;
    assert!(registry.join_running(1, late).is_err());
}
