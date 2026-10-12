//! Test bots in a match (`MatchPlayer::bot`, `SRP_BOT_ACCOUNTS`): a
//! scripted client that simulates nothing is a player for the others and
//! no witness of the world (`game.rs`, "Bots").

use survarium::v026g::match_msgs::ServerMessage;
use survarium::v026g::match_msgs::rules::state;
use survarium::v026g::match_msgs::snapshot::Tdm;

use super::client::Client;
use super::{Bench, MAP_TDM, session_of};
use crate::v026g::{MatchEnd, Settings};

/// The bot's client connects and says it has loaded: it has no level to
/// load, so it is ready long before a real client.
fn bot_loads(bench: &mut Bench, place: usize) {
    bench.clients[place].connect();
    assert!(
        bench.until(1_000, |bench| bench.clients[place].model.options.is_some()),
        "the bot got no static info"
    );
    bench.clients[place].ready();
}

fn hash_reports_sent(client: &Client) -> bool {
    // An idle client has no world: `uploads` counts its 0xE0 bursts, and
    // it never passes the hash gate.
    client.uploads > 0
}

#[test]
fn one_real_client_and_a_bot_play_a_match_to_its_end() {
    let mut bench = Bench::with_bots(2, MAP_TDM, Settings::default(), &[1]);

    // The bot is there first and waits: nobody holds a world yet.
    bot_loads(&mut bench, 1);
    bench.run(2_000);
    assert_eq!(bench.stages(), "-w");
    assert!(!bench.game().has_clock());

    // The real client: its initial world starts the match; it is in, and
    // then the bot is started from a world and put in (0xE9) for a tick
    // ahead of the real client's clock.
    bench.join(0);
    assert!(bench.until(5_000, |bench| {
        bench.stages() == "LL" && bench.clients[1].model.is_synced()
    }));
    assert!(bench.until(1_000, |bench| bench.clients[0].entered() == [0, 1]));
    // The bot is handed the real player's entry with its world, then
    // enters itself.
    assert_eq!(bench.clients[1].entered(), [0, 1]);
    assert_eq!(bench.clients[1].model.local_index, Some(1));
    assert_eq!(
        bench.clients[1]
            .model
            .options
            .as_ref()
            .unwrap()
            .players
            .len(),
        2
    );
    let entered_at = bench.clients[0].got(|message| match message {
        ServerMessage::PlayerEntered { player: 1, .. } => Some(()),
        _ => None,
    });
    assert!(!entered_at.is_empty());

    // The countdown and play, by the server's copy of the rule; the real
    // client's worlds are trusted on its word alone (one voucher), the
    // bot is never asked for one and nobody is resynced.
    bench.run_long(30_000);
    assert_eq!(bench.game().rule_state(), state::IN_PROCESS);
    let (observed, standing) = bench.game().observed().expect("a trusted world");
    assert!(observed > 20_000, "the trusted world is at {observed} ms");
    assert_eq!(standing.unwrap().points, [100, 100]);
    assert_eq!(bench.game().trusted_by(), 1);
    assert!(bench.clients[0].upload_requests >= 3);
    assert_eq!(bench.clients[1].upload_requests, 0);
    assert!(!hash_reports_sent(&bench.clients[1]));
    assert_eq!((bench.game().resyncs(0), bench.game().resyncs(1)), (0, 0));
    assert_eq!(bench.clients[0].model.syncs, 1);
    assert_eq!(bench.clients[1].model.syncs, 1);
    assert!(!bench.game().hash_starved());
    assert!(bench.results.try_recv().is_err());

    // The real player's inputs reach the bot (it acks them); the bot
    // makes none.
    let input = bench.clients[0].input(100, 0x21);
    assert!(bench.until(1_000, |bench| bench.clients[1].relays_of(0) == [input]));
    assert!(bench.clients[0].relays_of(1).is_empty());

    // The bot's team (1) runs out of tickets with nobody alive: the end
    // is read from the real client's world, and both are told.
    let at = bench.clock() / 50 * 50 + 100;
    bench.clients[0].sim.tdm_from = Some((
        at,
        Tdm {
            tickets: [58, 0],
            alive_players_mask: 0b01,
            active_players_mask: 0b11,
        },
    ));
    assert!(bench.until(12_000, |bench| {
        bench.clients.iter().all(|client| client.told_to_leave)
    }));
    let result = bench.results.try_recv().expect("a result");
    assert_eq!(result.end, MatchEnd::ModeRule);
    assert_eq!(result.winner, Some(0));
    assert_eq!(result.team_points, [58, 0]);
    assert_eq!(result.trusted_by, 1);
    assert!(result.played_ms.is_some());
    // The bot's row: it was in to the end, and has the (empty) counters
    // the world held for its place.
    let bot = &result.players[1];
    assert!(bot.entered && bot.present_at_end && !bot.left);
    assert_eq!((bot.kills, bot.deaths, bot.score), (0, 0, 0));
    assert!(bot.statistics.is_some());
    bench.assert_no_faults();
}

#[test]
fn without_the_setting_a_client_that_reports_nothing_keeps_every_world_untrusted() {
    // The same two clients, the second place not marked: the rules are the
    // ones for two real clients. Two must agree, the idle one says
    // nothing, so no world after the first is ever trusted and the match
    // cannot end by its mode's rule.
    let mut bench = Bench::new(2, MAP_TDM, Settings::default());
    bench.clients[1] = Client::idle(session_of(1));
    bot_loads(&mut bench, 1);
    bench.join(0);
    assert!(bench.until(5_000, |bench| bench.stages() == "LL"));
    bench.run_long(30_000);
    let (observed, _) = bench.game().observed().expect("the initial world");
    assert_eq!(observed, super::client::INITIAL_MS);
    let at = bench.clock() / 50 * 50 + 100;
    bench.clients[0].sim.tdm_from = Some((
        at,
        Tdm {
            tickets: [58, 0],
            alive_players_mask: 0b01,
            active_players_mask: 0b11,
        },
    ));
    bench.run_long(15_000);
    assert!(bench.results.try_recv().is_err());
    assert!(!bench.clients[0].told_to_leave);
    // Nobody is resynced over it either: one report a tick is not judged.
    assert_eq!((bench.game().resyncs(0), bench.game().resyncs(1)), (0, 0));
    bench.assert_no_faults();
}

#[test]
fn a_bots_place_is_no_witness_whatever_its_client_sends() {
    // Two real clients and a place marked as a bot's whose client is a
    // whole one: patched, hashing every tick, and with a world of its own.
    let mut bench = Bench::with_bots(3, MAP_TDM, Settings::default(), &[2]);
    let (players, map_id) = (3, MAP_TDM);
    bench.clients[2] = Client::new(session_of(2), true, players, map_id);
    bench.join_all();
    bench.run(4_000);
    let from = bench.clients[2].clock();
    bench.clients[2].sim.salt_from = Some((from, 9));
    bench.run_long(30_000);
    // Its reports are not heard: it is not resynced (it would be, as a
    // real client: `a_client_out_of_step_is_resynced_as_the_next_generation`),
    // and it is never asked for its world.
    assert_eq!(bench.game().resyncs(2), 0);
    assert_eq!(bench.clients[2].upload_requests, 0);
    assert_eq!(bench.clients[2].model.syncs, 1);
    // For the two real clients the rule is the one without it: a world
    // needs both.
    assert_eq!(bench.game().trusted_by(), 2);
    assert_eq!((bench.game().resyncs(0), bench.game().resyncs(1)), (0, 0));

    // One real client's claim alone is still not believed.
    let at = bench.clock() / 50 * 50 + 100;
    bench.clients[0].sim.tdm_from = Some((
        at,
        Tdm {
            tickets: [58, 0],
            alive_players_mask: 0b001,
            active_players_mask: 0b111,
        },
    ));
    bench.run_long(8_000);
    assert!(bench.results.try_recv().is_err());
    let (_, standing) = bench.game().observed().expect("the last agreed world");
    assert_eq!(standing.unwrap().points, [100, 100]);
    bench.assert_no_faults();
}

#[test]
fn a_match_left_to_its_bots_is_over() {
    let mut bench = Bench::with_bots(2, MAP_TDM, Settings::default(), &[1]);
    bot_loads(&mut bench, 1);
    bench.join(0);
    assert!(bench.until(5_000, |bench| bench.stages() == "LL"));
    bench.run(3_000);
    // The real player leaves; the bot would stay to the time limit.
    bench.clients[0].leave();
    assert!(bench.until(1_000, |bench| bench.clients[1].left() == [0]));
    bench.run_long(30_000);
    assert!(bench.results.try_recv().is_err());
    assert!(bench.until(20_000, |bench| bench.clients[1].told_to_leave));
    let result = bench.results.try_recv().expect("a result");
    assert_eq!(result.end, MatchEnd::Deserted);
    assert!(result.players[1].present_at_end && !result.players[0].present_at_end);
    bench.assert_no_faults();
}

#[test]
fn a_match_only_bots_came_to_is_over() {
    // Two bots the queue put together with nobody else: no world, no
    // clock; they are told to go.
    let mut bench = Bench::with_bots(2, MAP_TDM, Settings::default(), &[0, 1]);
    bot_loads(&mut bench, 0);
    bot_loads(&mut bench, 1);
    bench.run_long(40_000);
    assert_eq!(bench.stages(), "ww");
    assert!(bench.until(10_000, |bench| bench.results.try_recv().is_ok()));
    assert!(bench.until(8_000, |bench| bench.registry.match_count() == 0));
    bench.assert_no_faults();
}
