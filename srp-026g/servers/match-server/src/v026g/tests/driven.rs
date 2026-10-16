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

/// A match of one player against the server's bots ("Player vs Bots") goes
/// on while its player uploads the worlds it is asked for; one that stops
/// uploading for `Timing::pvb_stall` ends it, as one everyone left, and
/// the result says it was a match against bots.
#[test]
fn a_match_against_bots_whose_player_stops_uploading_is_over() {
    let mut settings = Settings::default();
    settings.timing.pvb_stall = std::time::Duration::from_secs(12);
    let mut bench = Bench::vs_bots(4, MAP_TDM, settings, &[1, 2, 3]);
    bench.join(0);
    assert!(bench.until(3_000, |bench| bench.clients[0].entered().len() == 4));
    // Its uploads keep it going well past the stall time.
    bench.run_long(30_000);
    assert!(bench.results.try_recv().is_err());
    assert_eq!(bench.game().rule_state(), state::IN_PROCESS);

    // Its game stops answering: over within the stall time and a request.
    bench.clients[0].no_uploads = true;
    let mut result = None;
    for _ in 0..25 {
        bench.run_long(1_000);
        if let Ok(over) = bench.results.try_recv() {
            result = Some(over);
            break;
        }
    }
    let result = result.expect("the match is over");
    assert_eq!(result.end, MatchEnd::Deserted);
    assert!(result.vs_bots);
    assert!(bench.until(10_000, |bench| bench.clients[0].told_to_leave));
}

/// The stall rule is only for a match against bots: the same match of
/// the server's bots for testing (`SRP_SERVER_BOTS`) goes on.
#[test]
fn a_match_with_bots_for_testing_has_no_stall_rule() {
    let mut settings = Settings::default();
    settings.timing.pvb_stall = std::time::Duration::from_secs(12);
    let mut bench = Bench::growing_with_driven(4, MAP_TDM, settings, &[1, 2, 3]);
    bench.join(0);
    assert!(bench.until(3_000, |bench| bench.clients[0].entered().len() == 4));
    bench.run_long(5_000);
    bench.clients[0].no_uploads = true;
    bench.run_long(30_000);
    assert!(bench.results.try_recv().is_err());
    assert_eq!(bench.game().rule_state(), state::IN_PROCESS);
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

/// The tick of the place's roster change (`entered`, else its leave).
fn change_of(bench: &Bench, place: usize, entered: bool) -> Option<u32> {
    bench
        .game()
        .roster_events()
        .iter()
        .find(|(_, who, change)| *who == place && *change == entered)
        .map(|(at, _, _)| *at)
}

/// A match of growing rosters, team 0's player at place 0 and the
/// server's bots at 1, 2 and 3 (teams 1, 0, 1: two a side), in play with
/// the player in and the bots entered, and taking late joiners.
fn playing_with_bots(map_id: u8, settings: Settings) -> Bench {
    let mut bench = Bench::growing_with_driven(4, map_id, settings, &[1, 2, 3]);
    bench.join(0);
    assert!(bench.until(2_000, |bench| bench.clients[0].entered().len() == 4));
    bench.run_long(25_000);
    assert_eq!(bench.game().rule_state(), state::IN_PROCESS);
    assert!(bench.until(2_000, |bench| bench.game().admitting()));
    bench
}

/// A late joiner on `team` comes into the match: its place, once every
/// client has its entry (the bots' places have none).
fn joins_late(bench: &mut Bench, team: u8) -> usize {
    assert!(bench.until(5_000, |bench| bench.game().admitting()));
    let place = bench.reserve_on(team);
    bench.join(place);
    assert!(
        bench.until(8_000, |bench| bench
            .clients
            .iter()
            .enumerate()
            .filter(|(index, _)| !(1..=3).contains(index))
            .all(|(_, client)| client.entered().contains(&(place as u8)))),
        "place {place}"
    );
    place
}

#[test]
fn a_servers_bot_gives_its_place_up_to_a_late_joiner_whose_side_needs_it() {
    let mut bench = playing_with_bots(MAP_TDM, Settings::default());
    let live = |bench: &Bench| {
        bench
            .registry
            .live_match(bench.match_id, bench.now())
            .expect("the match")
    };
    assert_eq!(live(&bench).giving_way, 0b1110);

    // Two a side: with a joiner on team 1 its side would be the larger, so
    // a bot of team 1 leaves (the last by place: with no view of the bots'
    // lives that is all there is to go by), a tick after the joiner is in.
    let first = joins_late(&mut bench, 1);
    assert!(bench.until(1_000, |bench| bench.clients[0].left() == [3]));
    let entered = change_of(&bench, first, true).expect("its entry");
    assert_eq!(change_of(&bench, 3, false), Some(entered + TICK_MS));
    // Every client hears it, the joiner too, and the lobby sees it gone (the
    // registry is a second behind at most).
    assert!(bench.until(1_000, |bench| { bench.clients[first].left() == [3] }));
    bench.run(1_100);
    assert_eq!(live(&bench).present, 0b1_0111);
    assert_eq!(live(&bench).giving_way, 0b0110);
    // It makes no input past its leave; the bot that stays does.
    let (gone, stays) = (
        bench.clients[0].relays_of(3).len(),
        bench.clients[0].relays_of(1).len(),
    );
    bench.run_long(12_000);
    assert_eq!(bench.clients[0].relays_of(3).len(), gone);
    assert!(bench.clients[0].relays_of(1).len() > stays);

    // Two a side again: a joiner on team 0 takes bot 2's place.
    let second = joins_late(&mut bench, 0);
    assert!(bench.until(1_000, |bench| bench.clients[0].left() == [3, 2]));
    assert!(change_of(&bench, 2, false) > change_of(&bench, second, true));
    // Team 0 has no bot left: the next one there makes it three against
    // two, and one on team 1 then fills the smaller side: bot 1 stays.
    let third = joins_late(&mut bench, 0);
    let fourth = joins_late(&mut bench, 1);
    bench.run_long(5_000);
    assert_eq!(bench.clients[0].left(), [3, 2]);
    assert_eq!(live(&bench).giving_way, 0b0010);
    assert_eq!(bench.game().bots_owing(), 0);
    assert_eq!(
        live(&bench).players,
        [3, 3],
        "the player, {second} and {third} against bot 1, {first} and {fourth}"
    );

    // The match goes on past the time a match of bots alone would be given
    // up: no bot's leave ends it.
    bench.run_long(60_000);
    assert_eq!(bench.game().rule_state(), state::IN_PROCESS);
    assert!(bench.results.try_recv().is_err());
    assert_eq!(bench.game().resyncs(0), 0);

    // Its end says who left: the two bots, and nobody else.
    let at = bench.clock() / TICK_MS * TICK_MS + 200;
    let standing = Some((
        at,
        Tdm {
            tickets: [58, 0],
            alive_players_mask: 0b0110_0001,
            active_players_mask: 0b1111_0011,
        },
    ));
    for client in &mut bench.clients {
        client.sim.tdm_from = standing;
    }
    assert!(bench.until(12_000, |bench| bench.clients[0].told_to_leave));
    let result = bench.results.try_recv().expect("a result");
    assert_eq!(result.end, MatchEnd::ModeRule);
    let left: Vec<usize> = (0..result.players.len())
        .filter(|&place| result.players[place].left)
        .collect();
    assert_eq!(left, [2, 3]);
    assert!(result.players[3].driven && result.players[3].entered);
    bench.assert_no_faults();
}

#[test]
fn with_giving_way_off_no_bot_leaves_for_a_late_joiner() {
    let settings = Settings {
        bots_give_way: false,
        ..Settings::default()
    };
    let mut bench = playing_with_bots(MAP_TDM, settings);
    let live = bench.registry.live_match(bench.match_id, bench.now());
    assert_eq!(live.map(|live| live.giving_way), Some(0));
    joins_late(&mut bench, 1);
    bench.run_long(5_000);
    assert!(bench.clients[0].left().is_empty());
    let live = bench.registry.live_match(bench.match_id, bench.now());
    assert_eq!(live.map(|live| live.players), Some([2, 3]));
    bench.assert_no_faults();
}

#[test]
fn a_part_of_a_group_enters_alone_when_the_bots_keep_the_sides_even() {
    // The player and bot 2 against bots 1 and 3; a group of two comes, one
    // for each side (places 4 and 5), and the one for team 1 never does.
    let mut bench = playing_with_bots(MAP_TDM, Settings::default());
    let first = bench.clients.len();
    let group: Vec<_> = (first..first + 2)
        .map(|place| setup(first + 2, MAP_TDM).players.remove(place))
        .collect();
    assert_eq!(
        group
            .iter()
            .map(|player| player.profile.team)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    bench
        .registry
        .join_running_group(bench.match_id, group)
        .expect("the match takes the group");
    for place in first..first + 2 {
        bench.clients.push(super::client::Client::growing(
            super::session_of(place),
            first + 2,
            MAP_TDM,
        ));
        bench.addrs.push(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            40_000 + place as u16,
        )));
    }
    bench.join(first);
    bench.run_long(61_000);
    // Counted with the bots the sides are two against two, and bot 2 gives
    // way: the one that came enters, and the sides stay even. Counted by
    // the clients alone (one against none) it would have waited its time
    // out.
    assert!(!bench.clients[first].disconnected);
    assert!(bench.clients[0].entered().contains(&(first as u8)));
    assert!(!bench.clients[0].entered().contains(&(first as u8 + 1)));
    assert_eq!(bench.clients[0].left(), [2]);
    bench.assert_no_faults();
}

#[test]
fn a_bot_of_a_side_out_of_respawns_leaves_only_once_dead_and_the_dead_one_first() {
    use survarium::v026g::match_msgs::motion::Tail;
    use survarium::v026g::match_msgs::players::PlayerState;

    let settings = Settings {
        maps: Some(walking_map()),
        ..Settings::default()
    };
    let mut bench = Bench::growing_with_driven(4, MAP_TDM, settings, &[1, 2, 3]);
    // Team 1's bots stand alive in the worlds, 40 m apart.
    let state = PlayerState {
        current_index: 0,
        target_index: 0,
        items: vec![1, 2, 3],
        alive: true,
        has_to_die: false,
        look_pitch: 0.0,
        rotation_y: 0.0,
        actions_mask: 0,
        effects: 0,
    };
    let tail = |x: f32| Tail {
        time_ms: 0,
        speed: 0.0,
        health: 1.0,
        inserted_time_ms: 100,
        physics_position: Some([x, 0.9, 0.0]),
        crouching: false,
        sprinting: false,
        recent_hits: Vec::new(),
    };
    let bodies = |client: &mut crate::v026g::test_client::Client, alive: [bool; 2]| {
        for (place, x, alive) in [(1, -20.0, alive[0]), (3, 20.0, alive[1])] {
            client.sim.player_states[place] = Some(PlayerState {
                alive,
                ..state.clone()
            });
            client.sim.player_tails[place] = Some(tail(x));
        }
    };
    bodies(&mut bench.clients[0], [true, true]);
    bench.join(0);
    assert!(bench.until(2_000, |bench| bench.clients[0].entered().len() == 4));
    bench.run_long(25_000);
    assert!(bench.until(2_000, |bench| bench.game().admitting()));

    // Team 1 is out of tickets: its living bots may not leave.
    let out = Some((bench.clock() / TICK_MS * TICK_MS + 100, [true, false]));
    bench.clients[0].sim.team_respawn_from = out;
    bench.run_long(6_000);
    assert!(bench.until(5_000, |bench| bench.game().admitting()));
    let place = bench.reserve_on(1);
    bodies(&mut bench.clients[place], [true, true]);
    bench.clients[place].sim.team_respawn_from = out;
    bench.join(place);
    assert!(bench.until(8_000, |bench| {
        bench.clients[0].entered().contains(&(place as u8))
    }));
    bench.run_long(5_000);
    assert!(bench.clients[0].left().is_empty());
    assert_eq!(bench.game().bots_owing(), 1);
    // The bot owing its place is not offered to the lobby again.
    let live = bench.registry.live_match(bench.match_id, bench.now());
    assert_eq!(live.map(|live| live.giving_way.count_ones()), Some(2));

    // Bot 1 dies: it leaves, not bot 3 (the last by place).
    for client in [0, place] {
        bodies(&mut bench.clients[client], [false, true]);
    }
    assert!(bench.until(10_000, |bench| bench.clients[0].left() == [1]));
    assert_eq!(bench.game().bots_owing(), 0);
    bench.run_long(5_000);
    assert_eq!(bench.clients[0].left(), [1]);
    assert_eq!(bench.game().rule_state(), state::IN_PROCESS);
}

/// A 60 x 60 m floor for the map of [`MAP_TDM`] with two stations to walk
/// to, written where `SRP_MAPDATA_DIR` would point; the store with its
/// walk grid built.
fn walking_map() -> std::sync::Arc<crate::v026g::bots::maps::Maps> {
    use crate::v026g::bots::maps::{MapState, Maps};
    use mapdata::{Library, MapData, Placement, Vec3, WalkerBehaviour};

    let project = survarium::v026g::maps::map_mode(MAP_TDM).unwrap().project;
    let floor = Library {
        name: "floor".into(),
        vertices: vec![
            Vec3::new(-30.0, 0.0, -30.0),
            Vec3::new(30.0, 0.0, -30.0),
            Vec3::new(30.0, 0.0, 30.0),
            Vec3::new(-30.0, 0.0, 30.0),
        ],
        triangles: vec![[0, 2, 1], [0, 3, 2]],
        walker: vec![WalkerBehaviour::Walk; 2],
        primitives: 0,
    };
    let map = MapData {
        project: project.into(),
        levelmap_min: Vec3::new(-30.0, 0.0, -30.0),
        levelmap_max: Vec3::new(30.0, 1.0, 30.0),
        stations: vec![Vec3::new(20.0, 0.0, 20.0), Vec3::new(20.0, 0.0, -20.0)],
        libraries: vec![floor],
        placements: vec![Placement {
            library: 0,
            position: Vec3::ZERO,
            rotation: Vec3::ZERO,
            cgroup: 10,
            cmask: 20,
        }],
        ..MapData::default()
    };
    let dir = std::env::temp_dir().join(format!("srp-walking-map-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    map.write(&dir.join(mapdata::format::file_name(project)))
        .unwrap();
    let maps = Maps::new(Some(dir));
    let started = std::time::Instant::now();
    while !matches!(maps.get(project), MapState::Ready(_)) {
        assert!(started.elapsed().as_secs() < 20, "{:?}", maps.get(project));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    maps
}

#[test]
fn with_the_map_the_servers_bot_sees_its_body_in_the_worlds_and_walks_smoothly() {
    use survarium::v026g::match_msgs::motion::Tail;
    use survarium::v026g::match_msgs::players::PlayerState;

    let settings = Settings {
        maps: Some(walking_map()),
        ..Settings::default()
    };
    let mut bench = Bench::with_driven(2, MAP_TDM, settings, &[1]);
    // The real client's worlds have the bot's body standing at (-20, 0, 0)
    // (the physics frame mirrors z), facing +z.
    let sim = &mut bench.clients[0].sim;
    sim.player_states[1] = Some(PlayerState {
        current_index: 0,
        target_index: 0,
        items: vec![1, 2, 3],
        alive: true,
        has_to_die: false,
        look_pitch: 0.0,
        rotation_y: 0.0,
        actions_mask: 0,
        effects: 0,
    });
    sim.player_tails[1] = Some(Tail {
        time_ms: 0,
        speed: 0.0,
        health: 1.0,
        inserted_time_ms: 100,
        physics_position: Some([-20.0, 0.9, 0.0]),
        crouching: false,
        sprinting: false,
        recent_hits: Vec::new(),
    });
    bench.join(0);
    assert!(bench.until(2_000, |bench| bench.clients[0].entered() == [0, 1]));
    // The bots' tactics are built on a thread of their own, in real time.
    let started = std::time::Instant::now();
    while !bench.game().bots_have_map() {
        assert!(started.elapsed().as_secs() < 20, "no tactics");
        std::thread::sleep(std::time::Duration::from_millis(2));
        bench.step(5);
    }

    // Once a world shows its body and play has begun (after the 20 s
    // countdown) it heads for a station: forward, and turns that ease in,
    // none more than the top turning speed allows.
    assert!(bench.until(30_000, |bench| {
        bench.clients[0]
            .relays_of(1)
            .iter()
            .any(|input| input.actions & key::FORWARD != 0)
    }));
    bench.run_long(3_000);
    let relays = bench.clients[0].relays_of(1);
    assert_bot_inputs(&relays);
    let yaw = |input: &PlayerInput| {
        crate::v026g::drive::half_value(u16::from_le_bytes([input.rotation[0], input.rotation[1]]))
    };
    assert!(
        relays.iter().filter(|input| yaw(input) != 0.0).count() >= 3,
        "{relays:?}"
    );
    assert!(
        relays
            .iter()
            .all(|input| yaw(input).abs() <= 5.0 * 0.05 + 1e-3)
    );

    // The worlds keep showing the body where it stood: it is stuck, and
    // steps back to get out.
    assert!(bench.until(20_000, |bench| {
        bench.clients[0]
            .relays_of(1)
            .iter()
            .any(|input| input.actions & key::BACK != 0)
    }));
    // While it walks the client is asked for a world every 2 s (5 s
    // without bots).
    let asked = |bench: &Bench| {
        bench.clients[0]
            .got(|message| match message {
                ServerMessage::UploadRequest => Some(()),
                _ => None,
            })
            .len()
    };
    let before = asked(&bench);
    bench.run_long(10_000);
    let during = asked(&bench) - before;
    assert!((4..=7).contains(&during), "{during} requests in 10 s");
    assert_eq!(bench.game().resyncs(0), 0);
    bench.assert_no_faults();
}
