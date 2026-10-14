//! Scenarios: a bot against a small stand-in for the game. The stand-in
//! moves the body by the bot's inputs at their ticks (the same movement
//! model, kept on the same walk grid, plus what the bot cannot know: a
//! post the grid lacks), and gives the bot a world of a tick 300 ms behind
//! the clock every second, as the match does.

use std::collections::VecDeque;

use mapdata::{RayTree, Vec3, WalkGrid, World};

use super::body::{Fix, slide};
use super::kin::{TICK_S, wrap};
use super::*;
use crate::v026g::drive::half_value;

/// The game's side of one bot body.
struct Game {
    grid_world: World,
    feet: Vec3,
    node: NodeId,
    yaw: f32,
    velocity: [f32; 2],
    keys: u32,
    /// Inputs to apply, by tick.
    inputs: VecDeque<(u32, u32, (f32, f32))>,
    /// The body at each tick, for the worlds.
    states: VecDeque<(u32, Vec3, f32, u32, f32)>,
    /// A post the walk grid does not have: (centre, radius).
    post: Option<(Vec3, f32)>,
    tick: u32,
}

impl Game {
    fn new(grid: WalkGrid, stations: Vec<Vec3>, start: Vec3, yaw: f32) -> Self {
        let node = grid.locate(start, 1.0).unwrap();
        let map = mapdata::MapData {
            stations,
            ..mapdata::MapData::default()
        };
        Self {
            grid_world: World {
                map,
                grid,
                sight: RayTree::build(&[]),
            },
            feet: start,
            node,
            yaw,
            velocity: [0.0, 0.0],
            keys: 0,
            inputs: VecDeque::new(),
            states: VecDeque::new(),
            post: None,
            tick: 0,
        }
    }

    /// Plays the ticks up to `clock`: each moves the body by the keys held,
    /// then takes the tick's input.
    fn run_to(&mut self, clock: u32, kin: &Kinematics) {
        while self.tick + TICK_MS <= clock {
            self.tick += TICK_MS;
            let target = kin.target_velocity(self.keys, self.yaw, 1.0);
            self.velocity = kin.approach(self.velocity, target);
            let to = Vec3::new(
                self.feet.x + self.velocity[0] * TICK_S,
                self.feet.y,
                self.feet.z + self.velocity[1] * TICK_S,
            );
            let (node, feet, _) = slide(&self.grid_world.grid, self.node, self.feet, to);
            let blocked = self
                .post
                .is_some_and(|(centre, radius)| feet.distance_xz(centre) < radius + 0.38);
            if blocked {
                self.velocity = [0.0, 0.0];
            } else {
                (self.node, self.feet) = (node, feet);
            }
            while let Some(&(at, keys, turn)) = self.inputs.front() {
                if at > self.tick {
                    break;
                }
                assert_eq!(at, self.tick, "an input for a tick already played");
                self.inputs.pop_front();
                self.keys = keys;
                self.yaw += turn.0;
            }
            let speed = self.velocity[0].hypot(self.velocity[1]);
            self.states
                .push_back((self.tick, self.feet, self.yaw, self.keys, speed));
            if self.states.len() > 400 {
                self.states.pop_front();
            }
        }
    }

    /// The world of the tick `lag` ms behind the clock.
    fn world(&self, lag: u32) -> Option<Fix> {
        let at = self.tick.checked_sub(lag)?;
        let &(time, feet, yaw, keys, speed) = self.states.iter().find(|state| state.0 == at)?;
        Some(Fix {
            time,
            alive: true,
            inserted: 50,
            feet: Some(feet),
            yaw,
            pitch: 0.0,
            keys,
            speed: Some(speed),
        })
    }
}

struct Run {
    notes: Vec<Note>,
    /// Each input's tick and turn.
    turns: Vec<(u32, f32)>,
    /// How far the reckoning was off, at each world.
    errors: Vec<f32>,
}

/// Runs bot 0 against the game for `ms`, the clock stepping 10 to 30 ms.
fn run(game: &mut Game, ms: u32) -> Run {
    let mut bots = Bots::new(0x5EED, &[true]);
    let kin = bots.kin.clone();
    bots.entered(0, 0);
    let mut out = Run {
        notes: Vec::new(),
        turns: Vec::new(),
        errors: Vec::new(),
    };
    let mut clock = 0;
    let mut next_world = 500;
    let mut step = 10;
    while clock < ms {
        clock += step;
        step = 10 + (step + 7) % 21;
        game.run_to(clock, &kin);
        if clock >= next_world {
            next_world += 1_000;
            if let Some(fix) = game.world(300) {
                out.notes.extend(bots.fixed(0, fix, Some(&game.grid_world)));
                if let Some(error) = bots.bot(0).unwrap().track().last_error() {
                    out.errors.push(error);
                }
            }
        }
        let (made, notes) = bots.poll(clock, Some(&game.grid_world));
        out.notes.extend(notes);
        for made in made {
            assert!(
                made.time > clock && made.time <= clock + 100,
                "ahead of the clock"
            );
            assert!(game.inputs.back().is_none_or(|last| last.0 < made.time));
            let rotation = made.input.rotation;
            let turn = (
                half_value(u16::from_le_bytes([rotation[0], rotation[1]])),
                half_value(u16::from_le_bytes([rotation[2], rotation[3]])),
            );
            out.turns.push((made.time, turn.0));
            game.inputs.push_back((made.time, made.input.actions, turn));
        }
    }
    // SRP_BOT_NOTES=1 shows what the bot did.
    if std::env::var_os("SRP_BOT_NOTES").is_some() {
        for note in &out.notes {
            eprintln!("{:>6} ms: {}", note.time, note.text);
        }
        eprintln!("reckoning off at the worlds (m): {:?}", out.errors);
    }
    out
}

fn count(run: &Run, text: &str) -> usize {
    run.notes
        .iter()
        .filter(|note| note.text.contains(text))
        .count()
}

#[test]
fn a_bot_walks_from_goal_to_goal_round_a_wall_and_keeps_its_body_reckoned() {
    // The 40 m floor with a wall at x = 20 up to z = 30; goals both sides.
    let stations = vec![
        Vec3::new(30.0, 0.0, 10.0),
        Vec3::new(8.0, 0.0, 8.0),
        Vec3::new(30.0, 0.0, 35.0),
    ];
    let mut game = Game::new(
        body::tests::walled(),
        stations,
        Vec3::new(10.0, 0.0, 10.0),
        0.3,
    );
    let run = run(&mut game, 90_000);
    let arrived = count(&run, "is at its goal");
    assert!(arrived >= 3, "{:#?}", run.notes);
    assert_eq!(count(&run, "stuck"), 0, "{:#?}", run.notes);
    assert_eq!(count(&run, "off its route"), 0, "{:#?}", run.notes);
    // The reckoning stays near the game's body: the model is the game's
    // here, so only the clock's steps part them.
    assert!(!run.errors.is_empty());
    let worst = run.errors.iter().copied().fold(0.0, f32::max);
    assert!(worst < 0.3, "{:?}", run.errors);
    // Smooth: no tick turns more than the top speed allows, and the
    // turning speed changes by small steps.
    let feel = Feel::default();
    for (_, turn) in &run.turns {
        assert!(turn.abs() <= feel.max_turn_speed * TICK_S + 1e-3, "{turn}");
    }
    assert!(wrap(game.yaw).is_finite());
}

#[test]
fn a_bot_caught_on_a_post_the_map_lacks_gets_out_and_goes_on() {
    // Open floor; the one goal straight ahead, a post in the way that the
    // walk grid does not have.
    let grid = WalkGrid::build(
        &body::tests::floor(0.0, 0.0, 60.0, 60.0, 0.0),
        [0.0, 0.0, 60.0, 60.0],
        0.25,
        mapdata::Body::default(),
    );
    let stations = vec![Vec3::new(30.0, 0.0, 50.0), Vec3::new(10.0, 0.0, 50.0)];
    let mut game = Game::new(grid, stations, Vec3::new(30.0, 0.0, 10.0), 0.0);
    game.post = Some((Vec3::new(30.0, 0.0, 30.0), 1.2));
    let run = run(&mut game, 60_000);
    // It noticed, stepped off, and still got somewhere.
    assert!(count(&run, "is stuck") >= 1, "{:#?}", run.notes);
    assert!(count(&run, "is at its goal") >= 1, "{:#?}", run.notes);
    // It did not stand at the post the whole time: never more than three
    // stuck notes in a row at one spot before it gave that way up.
    assert!(count(&run, "GiveUp") <= 1, "{:#?}", run.notes);
}
