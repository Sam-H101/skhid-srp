//! Soak runs (`player_bots_research.md` B8 point 5): whole matches of bots
//! against a stand-in for the game. Only a real client simulates a match,
//! so these runs stand in for it with a small game of their own:
//! - bodies move by the bots' inputs at their ticks with the movement
//!   model (`kin`), kept on the walk grid;
//! - a shot hits the first enemy whose chest lies within 0.45 m of the
//!   view's ray, with a clear line on the bullet mesh; five hits kill;
//! - the dead respawn at their side's spawns after 6.4 s (the radio);
//! - a use takes a battery within reach, puts it in at one's own
//!   container, or steals from the other's; a drop drops it;
//! - a world of every body, the statistics and the batteries comes every
//!   second, 300 ms behind the clock, as the match gives the bots.
//!
//! The stand-in is much simpler than the game (no recoil, no armour, no
//! stamina, no jumps): the runs check that bots of two teams find each
//! other, fight, die, come back, take and bring batteries and end a
//! match, with no input out of the rules; not how well.
//! `SRP_BOT_NOTES=1` prints what each bot did.

use std::collections::VecDeque;
use std::sync::Arc;

use mapdata::{BatterySpawner, Container, NodeId, Vec3, World};
use survarium::v026g::match_msgs::TICK_MS;

use super::body::{Fix, slide};
use super::kin::{Blend, Kinematics, TICK_S, forward, to_world};
use super::rng::Rng;
use super::trace::Trace;
use super::tuning::Tuning;
use super::view::{BatteryState, Clock, WorldFacts};
use super::*;
use crate::v026g::drive::{half_value, key};

/// A body of the stand-in game.
#[derive(Clone, Debug)]
struct Body {
    team: u8,
    feet: Vec3,
    node: Option<NodeId>,
    yaw: f32,
    pitch: f32,
    blend: Blend,
    keys: u32,
    health: f32,
    alive: bool,
    dead_at: u32,
    spawned: u32,
    carrying: Option<usize>,
    last_shot: u32,
    moved: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Lying {
    Free(Vec3),
    Carried(Place),
    Home(u8),
}

type Recorded = (u32, Vec<Fix>, Vec<f32>, WorldFacts);

/// The stand-in game.
struct Game {
    world: Arc<World>,
    kin: Kinematics,
    bodies: Vec<Body>,
    batteries: Vec<Lying>,
    containers: Vec<(u8, Vec3)>,
    stats: Vec<(u32, u32, u32)>,
    points: [u32; 2],
    inputs: Vec<VecDeque<(u32, u32, (f32, f32))>>,
    /// The worlds of the last two seconds.
    history: VecDeque<Recorded>,
    tick: u32,
    rng: Rng,
    shots: u32,
    hits: u32,
    delivered: [u32; 2],
    stolen: u32,
}

/// The soak map: the 80 x 40 m field (a wall across its middle up to
/// z = 25), side 0 spawning at x = 5, side 1 at x = 75, their containers
/// near their spawns and three batteries between.
fn soak_world() -> Arc<World> {
    super::tactics::tests::field_with(|map| {
        let container = |team: i32, id: i32, x: f32, z: f32| Container {
            position: Vec3::new(x, 0.0, z),
            team,
            id,
            use_shapes: Vec::new(),
        };
        map.containers = vec![container(0, 0, 12.0, 30.0), container(1, 1, 68.0, 30.0)];
        map.battery_spawners = [(40.0, 33.0), (25.0, 12.0), (55.0, 12.0)]
            .iter()
            .map(|&(x, z)| BatterySpawner {
                position: Vec3::new(x, 0.0, z),
                item: Vec3::new(x, 0.3, z),
                layer: "default".into(),
            })
            .collect();
    })
}

impl Game {
    fn new(world: Arc<World>, teams: &[u8], seed: u64) -> Self {
        let containers = world
            .map
            .containers
            .iter()
            .map(|c| (c.team as u8, c.position))
            .collect();
        let batteries = world
            .map
            .battery_spawners
            .iter()
            .map(|s| Lying::Free(s.item))
            .collect();
        let body = |team| Body {
            team,
            feet: Vec3::ZERO,
            node: None,
            yaw: 0.0,
            pitch: 0.0,
            blend: Blend::default(),
            keys: 0,
            health: 1.0,
            alive: false,
            dead_at: 0,
            spawned: 0,
            carrying: None,
            last_shot: 0,
            moved: 0.0,
        };
        let mut game = Self {
            kin: Kinematics::default(),
            bodies: teams.iter().map(|&team| body(team)).collect(),
            batteries,
            containers,
            stats: vec![(0, 0, 0); teams.len()],
            points: [0, 0],
            inputs: vec![VecDeque::new(); teams.len()],
            history: VecDeque::new(),
            tick: 0,
            rng: Rng::new(seed),
            shots: 0,
            hits: 0,
            delivered: [0, 0],
            stolen: 0,
            world,
        };
        for place in 0..teams.len() {
            game.spawn(place);
        }
        game
    }

    fn spawn(&mut self, place: Place) {
        let side = i32::from(self.bodies[place].team);
        let spawns: Vec<Vec3> = self
            .world
            .map
            .spawns
            .iter()
            .filter(|s| s.team == side)
            .map(|s| s.position)
            .collect();
        let at = spawns[self.rng.below(spawns.len())]
            + Vec3::new(self.rng.range(-2.0, 2.0), 0.0, self.rng.range(-2.0, 2.0));
        let node = self.world.grid.locate(at, 3.0);
        let feet = node.map_or(at, |n| self.world.grid.position(n));
        let tick = self.tick;
        let body = &mut self.bodies[place];
        body.feet = feet;
        body.node = node;
        // Facing the other side.
        body.yaw = if side == 0 {
            -std::f32::consts::FRAC_PI_2
        } else {
            std::f32::consts::FRAC_PI_2
        };
        body.pitch = 0.0;
        body.blend = Blend::default();
        body.health = 1.0;
        body.alive = true;
        body.spawned = tick;
        body.carrying = None;
    }

    fn eye(body: &Body) -> Vec3 {
        let up = if body.keys & key::CROUCH != 0 {
            0.95
        } else {
            1.6
        };
        body.feet + Vec3::new(0.0, up, 0.0)
    }

    fn chest(body: &Body) -> Vec3 {
        let up = if body.keys & key::CROUCH != 0 {
            0.8
        } else {
            1.3
        };
        body.feet + Vec3::new(0.0, up, 0.0)
    }

    /// One tick: each body takes its input (the world of the tick shows
    /// it, before the move), then moves; shots, uses, drops, deaths,
    /// respawns.
    fn step(&mut self) {
        self.tick += TICK_MS;
        let tick = self.tick;
        let n = self.bodies.len();
        let mut pressed = vec![0u32; n];
        for (place, queue) in self.inputs.iter_mut().enumerate() {
            while let Some(&(at, keys, turn)) = queue.front() {
                if at > tick {
                    break;
                }
                assert_eq!(at, tick, "an input for a tick already played");
                queue.pop_front();
                let body = &mut self.bodies[place];
                pressed[place] = keys & !body.keys;
                body.keys = keys;
                body.yaw += turn.0;
                body.pitch = (body.pitch + turn.1).clamp(-1.0, 1.0);
            }
        }
        self.record();
        for place in 0..n {
            if !self.bodies[place].alive {
                if tick >= self.bodies[place].dead_at + 6_400 {
                    self.spawn(place);
                }
                continue;
            }
            let body = &mut self.bodies[place];
            let level = if body.carrying.is_some() { 0.95 } else { 1.0 };
            let local = body.blend.tick(
                self.kin.target_local(body.keys, level),
                self.kin.blend_ticks,
            );
            let v = to_world(local, body.yaw);
            let to = Vec3::new(
                body.feet.x + v[0] * TICK_S,
                body.feet.y,
                body.feet.z + v[1] * TICK_S,
            );
            if let Some(node) = body.node {
                let (node, feet, _) = slide(&self.world.grid, node, body.feet, to);
                body.moved += feet.distance_xz(body.feet);
                body.node = Some(node);
                body.feet = feet;
            }
        }
        self.shoot(tick);
        self.use_and_drop(&pressed);
    }

    /// Shots: a hit on the nearest enemy chest within 0.45 m of the ray.
    fn shoot(&mut self, tick: u32) {
        for shooter in 0..self.bodies.len() {
            let body = &self.bodies[shooter];
            if !body.alive
                || body.keys & key::FIRE == 0
                || tick < body.last_shot + 150
                || body.carrying.is_some()
            {
                continue;
            }
            self.bodies[shooter].last_shot = tick;
            self.shots += 1;
            let body = &self.bodies[shooter];
            let eye = Self::eye(body);
            let [fx, fz] = forward(body.yaw);
            let (c, s) = (body.pitch.cos(), body.pitch.sin());
            let dir = Vec3::new(fx * c, s, fz * c);
            let mut best: Option<(Place, f32)> = None;
            for (target, other) in self.bodies.iter().enumerate() {
                if !other.alive || other.team == body.team {
                    continue;
                }
                let to = Self::chest(other) - eye;
                let along = to.dot(dir);
                if along <= 0.0 || along > 100.0 {
                    continue;
                }
                let miss = (to - dir * along).length();
                if miss < 0.45
                    && best.is_none_or(|(_, d)| along < d)
                    && self.world.sight.visible(eye, Self::chest(other))
                {
                    best = Some((target, along));
                }
            }
            let Some((target, _)) = best else {
                continue;
            };
            self.hits += 1;
            let other = &mut self.bodies[target];
            other.health -= 0.2;
            if other.health <= 1e-3 {
                other.alive = false;
                other.dead_at = tick;
                if let Some(battery) = other.carrying.take() {
                    self.batteries[battery] = Lying::Free(other.feet);
                }
                self.stats[target].1 += 1;
                self.stats[target].2 = 0;
                self.stats[shooter].0 += 1;
                self.stats[shooter].2 += 1;
            }
        }
    }

    fn use_and_drop(&mut self, pressed: &[u32]) {
        for place in 0..self.bodies.len() {
            let body = self.bodies[place].clone();
            if !body.alive {
                continue;
            }
            if pressed[place] & key::DROP != 0
                && let Some(battery) = body.carrying
            {
                self.batteries[battery] = Lying::Free(body.feet);
                self.bodies[place].carrying = None;
            }
            if pressed[place] & key::USE == 0 {
                continue;
            }
            let team = body.team;
            match body.carrying {
                Some(battery) => {
                    let home = self.containers.iter().position(|(t, _)| *t == team);
                    if let Some(id) = home
                        && self.containers[id].1.distance_xz(body.feet) < 2.5
                    {
                        self.batteries[battery] = Lying::Home(id as u8);
                        self.bodies[place].carrying = None;
                        self.points[usize::from(team)] += 1;
                        self.delivered[usize::from(team)] += 1;
                    }
                }
                None => {
                    let free = self.batteries.iter().position(
                        |b| matches!(b, Lying::Free(at) if at.distance_xz(body.feet) < 1.8),
                    );
                    let theirs = self
                        .containers
                        .iter()
                        .position(|(t, at)| *t != team && at.distance_xz(body.feet) < 2.5);
                    if let Some(battery) = free {
                        self.batteries[battery] = Lying::Carried(place);
                        self.bodies[place].carrying = Some(battery);
                    } else if let Some(id) = theirs
                        && let Some(battery) = self
                            .batteries
                            .iter()
                            .position(|b| *b == Lying::Home(id as u8))
                    {
                        self.batteries[battery] = Lying::Carried(place);
                        self.bodies[place].carrying = Some(battery);
                        self.points[usize::from(1 - team)] -= 1;
                        self.stolen += 1;
                    }
                }
            }
        }
    }

    fn record(&mut self) {
        let fixes = self
            .bodies
            .iter()
            .map(|body| Fix {
                time: self.tick,
                alive: body.alive,
                inserted: body.spawned,
                feet: body.alive.then_some(body.feet),
                yaw: body.yaw,
                pitch: body.pitch,
                keys: body.keys,
                speed: Some({
                    let [a, l] = body.blend.now(self.kin.blend_ticks);
                    a.hypot(l)
                }),
            })
            .collect();
        let health = self.bodies.iter().map(|b| b.health).collect();
        let facts = WorldFacts {
            time: self.tick,
            stats: self.stats.clone(),
            points: Some(self.points),
            batteries: self
                .batteries
                .iter()
                .map(|b| match b {
                    Lying::Free(at) => BatteryState::Free(*at),
                    Lying::Carried(_) => BatteryState::Carried(None),
                    Lying::Home(id) => BatteryState::Home(*id),
                })
                .collect(),
            container_counts: (0..self.containers.len())
                .map(|id| {
                    self.batteries
                        .iter()
                        .filter(|b| **b == Lying::Home(id as u8))
                        .count()
                })
                .collect(),
        };
        self.history.push_back((self.tick, fixes, health, facts));
        while self.history.len() > 40 {
            self.history.pop_front();
        }
    }

    fn world_at(&self, tick: u32) -> Option<Recorded> {
        self.history.iter().find(|(t, ..)| *t == tick).cloned()
    }
}

/// How a soak run went.
#[derive(Debug, Default)]
struct Outcome {
    winner: Option<u8>,
    end_s: f32,
    kills: [u32; 2],
    delivered: [u32; 2],
    stolen: u32,
    shots: u32,
    hits: u32,
    least_moved_m: f32,
    stuck_notes: usize,
    #[allow(dead_code)]
    notes: Vec<Note>,
}

/// A match of `per_team` bots a side to `win` batteries (Team Deathmatch
/// when `win` is 0: the most kills when time is up), for at most
/// `limit_s`.
fn soak(per_team: usize, win: u32, limit_s: u32, seed: u64) -> Outcome {
    let world = soak_world();
    let teams: Vec<u8> = (0..2 * per_team).map(|p| u8::from(p >= per_team)).collect();
    let driven = vec![true; teams.len()];
    let roles: Vec<Option<&str>> = vec![None; teams.len()];
    let mode = if win > 0 {
        survarium::v026g::maps::mode::BATTERIES
    } else {
        survarium::v026g::maps::mode::TEAM_DEATHMATCH
    };
    let mut bots = Bots::new(
        0x50AC + seed as u32,
        mode,
        0,
        &teams,
        &driven,
        &roles,
        Arc::new(Tuning::default()),
        Trace::none(0),
    );
    bots.map_ready(world.clone());
    let mut game = Game::new(world, &teams, seed);
    for place in 0..teams.len() {
        bots.entered(place, true, 0);
    }
    let mut out = Outcome::default();
    let limit_ms = limit_s * 1000;
    let (mut clock, mut next_world) = (0, 1_000);
    while clock < limit_ms {
        clock += 25;
        while game.tick + TICK_MS <= clock {
            game.step();
        }
        if clock >= next_world {
            next_world += 1_000;
            if let Some((_, fixes, health, facts)) = game.world_at(clock / TICK_MS * TICK_MS - 300)
            {
                for (place, (fix, health)) in fixes.into_iter().zip(health).enumerate() {
                    out.notes.extend(bots.fixed(place, fix, Some(health)));
                }
                out.notes.extend(bots.facts(facts));
            }
        }
        let (made, notes) = bots.poll(Clock {
            clock,
            time_left_s: (limit_ms - clock) as f32 / 1000.0,
            total_s: limit_s as f32,
        });
        out.notes.extend(notes);
        for made in made {
            // Every input keeps the rules: on the grid, ahead, in order,
            // no forbidden bit, a turn under π.
            assert!(made.time > clock && made.time <= clock + 100);
            assert_eq!(made.input.actions & key::NEVER, 0);
            assert!(made.input.rotation_is_sane());
            let queue = &mut game.inputs[made.place];
            assert!(queue.back().is_none_or(|last| last.0 < made.time));
            let r = made.input.rotation;
            let turn = (
                half_value(u16::from_le_bytes([r[0], r[1]])),
                half_value(u16::from_le_bytes([r[2], r[3]])),
            );
            assert!(turn.0.abs() < std::f32::consts::PI && turn.1.abs() < std::f32::consts::PI);
            queue.push_back((made.time, made.input.actions, turn));
        }
        if win > 0
            && let Some(team) = (0..2u8).find(|&t| game.points[usize::from(t)] >= win)
        {
            out.winner = Some(team);
            break;
        }
    }
    out.end_s = clock as f32 / 1000.0;
    for (place, &team) in teams.iter().enumerate() {
        out.kills[usize::from(team)] += game.stats[place].0;
    }
    // Time up: the mode's leader wins (batteries home, else kills).
    if out.winner.is_none() {
        let (a, b) = if win > 0 {
            (game.points[0], game.points[1])
        } else {
            (out.kills[0], out.kills[1])
        };
        out.winner = match a.cmp(&b) {
            std::cmp::Ordering::Greater => Some(0),
            std::cmp::Ordering::Less => Some(1),
            std::cmp::Ordering::Equal => None,
        };
    }
    out.delivered = game.delivered;
    out.stolen = game.stolen;
    out.shots = game.shots;
    out.hits = game.hits;
    out.least_moved_m = game
        .bodies
        .iter()
        .map(|b| b.moved)
        .fold(f32::INFINITY, f32::min);
    out.stuck_notes = out
        .notes
        .iter()
        .filter(|n| n.text.contains("stuck"))
        .count();
    if std::env::var_os("SRP_BOT_NOTES").is_some() {
        for note in &out.notes {
            eprintln!(
                "{:>7} ms  bot {:>2}: {}",
                note.time, note.place as i64, note.text
            );
        }
        let Outcome { notes: _, .. } = &out;
        eprintln!(
            "winner {:?} at {} s; kills {:?}; delivered {:?}; stolen {}; hits {} of {} shots; least moved {:.0} m; stuck notes {}",
            out.winner,
            out.end_s,
            out.kills,
            out.delivered,
            out.stolen,
            out.hits,
            out.shots,
            out.least_moved_m,
            out.stuck_notes
        );
    }
    out
}

#[test]
fn four_bots_a_side_fight_and_bring_batteries_to_a_win() {
    let out = soak(4, 3, 900, 1);
    let total = out.kills[0] + out.kills[1];
    assert!(
        total >= 4,
        "they fought: {:?} kills, {} hits of {} shots",
        out.kills,
        out.hits,
        out.shots
    );
    assert!(
        out.delivered[0] + out.delivered[1] >= 1,
        "batteries brought: {:?}",
        out.delivered
    );
    assert!(
        out.least_moved_m > 30.0,
        "every bot moved: {}",
        out.least_moved_m
    );
    assert!(out.winner.is_some(), "a winner in {} s: {out:?}", out.end_s);
}

#[test]
fn a_team_deathmatch_of_bots_has_kills_on_both_sides() {
    let out = soak(4, 0, 240, 2);
    assert!(
        out.kills[0] > 0 && out.kills[1] > 0,
        "{:?} ({} hits, {} shots)",
        out.kills,
        out.hits,
        out.shots
    );
    assert!(out.least_moved_m > 20.0, "{}", out.least_moved_m);
}
