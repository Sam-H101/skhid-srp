//! Soak runs (`player_bots_research.md` B8 point 5): whole matches of bots
//! against a stand-in for the game. Only a real client simulates a match,
//! so these runs stand in for it with a small game of their own:
//! - bodies move by the bots' inputs at their ticks with the movement
//!   model (`kin`), kept on the walk grid;
//! - a shot hits the first enemy whose chest lies within 0.45 m of the
//!   view's ray, with a clear line on the bullet mesh; five hits kill;
//! - the weapon each bot holds fires as the client fires it: an automatic
//!   one at its rate while the trigger is held, any other once a press;
//!   and kicks as the client kicks ([`super::recoil`]): the shots lift the
//!   view itself (the base pitch and the kick), a downward turn is taken
//!   off the kick first, the kick comes back by the weapon's controller;
//!   each shot's kick drawn in the config's range (the bots predict the
//!   middle);
//! - the dead respawn at their side's spawns after 6.4 s (the radio);
//! - a use takes a battery within reach, puts it in at one's own
//!   container, or steals from the other's; a drop drops it;
//! - Team Deathmatch: each death costs the team a ticket; a team out of
//!   tickets no longer respawns, and loses when its last body dies;
//! - a world of every body, the statistics and the batteries comes every
//!   second, 300 ms behind the clock, as the match gives the bots.
//!
//! The stand-in is much simpler than the game (no spread, no side kick, no
//! armour, no stamina, no jumps; the rounds are not counted, the bots count
//! their own): the runs check that bots of two teams find each
//! other, fight, die, come back, take and bring batteries and end a
//! match, with no input out of the rules; not how well.
//! `SRP_BOT_NOTES=1` prints what each bot did.

use std::collections::VecDeque;
use std::sync::Arc;

use mapdata::{BatterySpawner, Container, NodeId, Vec3, World};
use survarium::v026g::match_msgs::TICK_MS;

use super::body::{Fix, slide};
use super::guns::{Arms, Gun};
use super::kin::{Blend, Kinematics, TICK_S, forward, to_world};
use super::recoil::{Cadence, Recoil, kick_of};
use super::rng::Rng;
use super::trace::Trace;
use super::tuning::Tuning;
use super::view::{BatteryState, Clock, WorldFacts};
use super::*;
use crate::v026g::drive::{half_value, key};

/// A body of the stand-in game.
#[derive(Clone, Debug)]
struct Body {
    /// The weapon in hand (the bot's, as its last input was made), its
    /// kick and its trigger.
    gun: &'static Gun,
    recoil: Recoil,
    cadence: Cadence,
    team: u8,
    feet: Vec3,
    node: Option<NodeId>,
    yaw: f32,
    /// The base pitch (`m_look_pitch`, as the worlds give it); the view is
    /// it and the kick.
    pitch: f32,
    blend: Blend,
    keys: u32,
    health: f32,
    alive: bool,
    dead_at: u32,
    spawned: u32,
    carrying: Option<usize>,
    moved: f32,
    /// The bot's target as its last input was made (the reflex's).
    target: Option<Place>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Lying {
    Free(Vec3),
    Carried(Place),
    Home(u8),
}

type Recorded = (u32, Vec<Fix>, Vec<f32>, WorldFacts);
/// An input for a tick: the tick, the keys, the turn, the bot's target
/// and weapon in hand.
type Queued = (u32, u32, (f32, f32), Option<Place>, &'static Gun);

/// The stand-in game.
struct Game {
    world: Arc<World>,
    kin: Kinematics,
    bodies: Vec<Body>,
    batteries: Vec<Lying>,
    containers: Vec<(u8, Vec3)>,
    stats: Vec<(u32, u32, u32)>,
    points: [u32; 2],
    /// Team Deathmatch's tickets.
    tickets: Option<[u32; 2]>,
    /// Timed batteries: the next one's time, and every how long one
    /// appears by a station (round the stations in turn).
    timed: Option<(u32, u32)>,
    inputs: Vec<VecDeque<Queued>>,
    /// The worlds of the last two seconds.
    history: VecDeque<Recorded>,
    tick: u32,
    rng: Rng,
    shots: u32,
    hits: u32,
    /// Shots at the bot's target whose ray met the world short of him, and
    /// those with nothing of him showing.
    walled: u32,
    hidden: u32,
    /// Rays the counts above cast (not the bots').
    metric_rays: u64,
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
        map.stations = vec![Vec3::new(40.0, 0.0, 33.0), Vec3::new(30.0, 0.0, 5.0)];
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
    fn new(
        world: Arc<World>,
        teams: &[u8],
        seed: u64,
        tickets: Option<u32>,
        timed: Option<u32>,
    ) -> Self {
        let containers = world
            .map
            .containers
            .iter()
            .map(|c| (c.team as u8, c.use_point()))
            .collect();
        // Timed batteries start with none lying about.
        let batteries = if timed.is_some() {
            Vec::new()
        } else {
            world
                .map
                .battery_spawners
                .iter()
                .map(|s| Lying::Free(s.item))
                .collect()
        };
        let gun = Arms::fallback().primary.0;
        let body = |team| Body {
            gun,
            recoil: Recoil::new(kick_of(gun.item)),
            cadence: Cadence::default(),
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
            moved: 0.0,
            target: None,
        };
        let mut game = Self {
            kin: Kinematics::default(),
            bodies: teams.iter().map(|&team| body(team)).collect(),
            batteries,
            containers,
            stats: vec![(0, 0, 0); teams.len()],
            points: [0, 0],
            tickets: tickets.map(|t| [t, t]),
            timed: timed.map(|every| (every, every)),
            inputs: vec![VecDeque::new(); teams.len()],
            history: VecDeque::new(),
            tick: 0,
            rng: Rng::new(seed),
            shots: 0,
            hits: 0,
            walled: 0,
            hidden: 0,
            metric_rays: 0,
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
        // Weapon 1 in hand afresh.
        body.gun = Arms::fallback().primary.0;
        body.recoil = Recoil::new(kick_of(body.gun.item));
        body.cadence = Cadence::default();
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

    /// One tick: each weapon's kick moves on, each body takes its input
    /// (the world of the tick shows it, before the move; a downward turn
    /// is taken off the kick first), then moves; shots, uses, drops,
    /// deaths, respawns.
    fn step(&mut self) {
        self.tick += TICK_MS;
        let tick = self.tick;
        let n = self.bodies.len();
        let mut pressed = vec![0u32; n];
        for body in &mut self.bodies {
            body.recoil.update(tick);
        }
        for (place, queue) in self.inputs.iter_mut().enumerate() {
            while let Some(&(at, keys, turn, target, gun)) = queue.front() {
                if at > tick {
                    break;
                }
                assert_eq!(at, tick, "an input for a tick already played");
                queue.pop_front();
                let body = &mut self.bodies[place];
                pressed[place] = keys & !body.keys;
                body.keys = keys;
                body.target = target;
                // Another weapon taken, or a reload: no kick.
                if gun.item != body.gun.item {
                    body.gun = gun;
                    body.recoil = Recoil::new(kick_of(gun.item));
                    body.recoil.update(tick);
                    body.cadence = Cadence::default();
                } else if pressed[place] & key::RELOAD != 0 {
                    body.recoil.reset();
                }
                body.yaw += turn.0;
                let taken = body.recoil.turn(turn.1);
                body.pitch = (body.pitch + turn.1 + taken).clamp(-1.0, 1.0);
            }
        }
        self.record();
        for place in 0..n {
            if !self.bodies[place].alive {
                let team = usize::from(self.bodies[place].team);
                let may = self.tickets.is_none_or(|tickets| tickets[team] > 0);
                if may && tick >= self.bodies[place].dead_at + 6_400 {
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
        // Timed batteries: one appears by the next station in turn.
        if let Some((next, every)) = self.timed
            && tick >= next
        {
            let stations = &self.world.map.stations;
            let at = stations[self.batteries.len() % stations.len()];
            self.batteries
                .push(Lying::Free(at + Vec3::new(1.5, 0.3, 0.0)));
            self.timed = Some((next + every, every));
        }
    }

    /// Shots, each along the view (the base pitch and the kick) and each
    /// kicking it: a hit on the nearest enemy chest within 0.45 m of the
    /// ray.
    fn shoot(&mut self, tick: u32) {
        for shooter in 0..self.bodies.len() {
            let body = &mut self.bodies[shooter];
            let held = body.alive && body.keys & key::FIRE != 0 && body.carrying.is_none();
            let (auto, interval_ms) = (body.gun.auto, body.gun.shot_ms());
            let mut times = Vec::new();
            body.cadence
                .tick(tick, held, auto, interval_ms, |t| times.push(t));
            for at in times {
                self.shot(shooter, at);
            }
        }
    }

    /// One shot of `shooter` at `at` ms (within the tick).
    fn shot(&mut self, shooter: Place, at: f32) {
        self.shots += 1;
        let body = &self.bodies[shooter];
        let eye = Self::eye(body);
        let [fx, fz] = forward(body.yaw);
        let pitch = body.recoil.view(body.pitch);
        let (c, s) = (pitch.cos(), pitch.sin());
        let dir = Vec3::new(fx * c, s, fz * c);
        let kick = body.recoil.kick();
        let top = kick.top(self.rng.unit());
        self.bodies[shooter].recoil.shot(at, top, 0.0);
        let tick = self.tick;
        let body = &self.bodies[shooter];
        // A shot at the bot's target with nothing of him showing
        // (`aimpoint`'s parts), or whose ray meets the world short of
        // him.
        if let Some(target) = body.target.map(|t| &self.bodies[t])
            && target.alive
        {
            let before = super::aimpoint::rays_cast();
            let crouched = target.keys & key::CROUCH != 0;
            if !super::aimpoint::shows(eye, target.feet, crouched, &self.world.sight, || true) {
                self.hidden += 1;
            }
            let reach = (Self::chest(target) - eye).length() - 0.5;
            if self.world.sight.ray_cast(eye, dir, reach).is_some() {
                self.walled += 1;
            }
            self.metric_rays += super::aimpoint::rays_cast() - before;
        }
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
            return;
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
            if let Some(tickets) = self.tickets.as_mut() {
                let team = usize::from(other.team);
                tickets[team] = tickets[team].saturating_sub(1);
            }
            self.stats[shooter].0 += 1;
            self.stats[shooter].2 += 1;
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
            points: Some(self.tickets.unwrap_or(self.points)),
            can_respawn: Some(self.tickets.map_or([true, true], |t| [t[0] > 0, t[1] > 0])),
            next_battery: self.timed.map(|(next, _)| next),
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

/// Every input the bots made and every note they wrote, as one number:
/// two runs that decide the same give the same digest. The proof that a
/// change for speed or memory left the bots as they were
/// (`docs/notes/player_bots_mapdata.md` §6).
#[derive(Clone, Copy, Debug, Default)]
struct Decisions {
    digest: mapdata::digest::Digest,
    inputs: u64,
    notes: u64,
}

impl Decisions {
    fn made(&mut self, made: &Made) {
        self.text(&format!("{} {} {:?}", made.place, made.time, made.input));
        self.inputs += 1;
    }

    fn note(&mut self, note: &Note) {
        self.text(&format!("{} {} {}", note.place, note.time, note.text));
        self.notes += 1;
    }

    /// With `SRP_TRACE_DECISIONS=<file>`, each line is also appended
    /// there: two runs' files show where they part.
    fn text(&mut self, text: &str) {
        if let Some(path) = std::env::var_os("SRP_TRACE_DECISIONS") {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap();
            writeln!(file, "{text}").unwrap();
        }
        self.digest.len(text.len());
        for chunk in text.as_bytes().chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.digest.u64(u64::from_le_bytes(word));
        }
    }

    fn line(&self) -> String {
        format!(
            "decisions {:016x} ({} inputs, {} notes)",
            self.digest.finish(),
            self.inputs,
            self.notes
        )
    }
}

/// How a soak run went.
#[derive(Debug, Default)]
struct Outcome {
    decisions: Decisions,
    winner: Option<u8>,
    end_s: f32,
    kills: [u32; 2],
    delivered: [u32; 2],
    stolen: u32,
    shots: u32,
    hits: u32,
    walled: u32,
    hidden: u32,
    /// Sight and aim rays the bots cast (`aimpoint`), per bot and second
    /// of play.
    rays_per_bot_s: f32,
    least_moved_m: f32,
    stuck_notes: usize,
    #[allow(dead_code)]
    notes: Vec<Note>,
}

/// A match of `per_team` bots a side to `win` batteries (Team Deathmatch
/// when `win` is 0: the most kills when time is up), for at most
/// `limit_s`.
fn soak(per_team: usize, win: u32, tdm_tickets: u32, limit_s: u32, seed: u64) -> Outcome {
    soak_mode(per_team, win, tdm_tickets, None, limit_s, seed)
}

/// [`soak`], with timed batteries (one every `timed_s`) when given.
fn soak_mode(
    per_team: usize,
    win: u32,
    tdm_tickets: u32,
    timed_s: Option<u32>,
    limit_s: u32,
    seed: u64,
) -> Outcome {
    soak_tuned(
        per_team,
        win,
        tdm_tickets,
        timed_s,
        limit_s,
        seed,
        bench_tuning(),
    )
}

/// [`soak_mode`] with the bots' `tuning`.
fn soak_tuned(
    per_team: usize,
    win: u32,
    tdm_tickets: u32,
    timed_s: Option<u32>,
    limit_s: u32,
    seed: u64,
    tuning: Tuning,
) -> Outcome {
    let world = soak_world();
    let teams: Vec<u8> = (0..2 * per_team).map(|p| u8::from(p >= per_team)).collect();
    let driven = vec![true; teams.len()];
    let roles: Vec<Option<&str>> = vec![None; teams.len()];
    let mode = if timed_s.is_some() {
        survarium::v026g::maps::mode::TIMED_BATTERIES
    } else if win > 0 {
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
        Arc::new(tuning),
        Trace::none(0),
    );
    bots.map_ready(world.clone());
    let tickets = (win == 0).then_some(tdm_tickets);
    let mut game = Game::new(world, &teams, seed, tickets, timed_s.map(|s| s * 1000));
    for place in 0..teams.len() {
        bots.entered(place, true, 0);
    }
    let mut out = Outcome::default();
    let rays_before = super::aimpoint::rays_cast();
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
                    let notes = bots.fixed(place, fix, Some(health));
                    notes.iter().for_each(|note| out.decisions.note(note));
                    out.notes.extend(notes);
                }
                let notes = bots.facts(facts);
                notes.iter().for_each(|note| out.decisions.note(note));
                out.notes.extend(notes);
            }
        }
        let (made, notes) = bots.poll(Clock {
            clock,
            time_left_s: (limit_ms - clock) as f32 / 1000.0,
            total_s: limit_s as f32,
        });
        notes.iter().for_each(|note| out.decisions.note(note));
        out.notes.extend(notes);
        for made in made {
            out.decisions.made(&made);
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
            let bot = bots.bots[made.place].as_ref();
            let target = bot.and_then(|bot| bot.reflex.target());
            let gun = bot.map_or(Arms::fallback().primary.0, |bot| bot.reflex.held_gun());
            queue.push_back((made.time, made.input.actions, turn, target, gun));
        }
        if win > 0
            && let Some(team) = (0..2u8).find(|&t| game.points[usize::from(t)] >= win)
        {
            out.winner = Some(team);
            break;
        }
        // Team Deathmatch: a team out of tickets with no one alive loses.
        if let Some(tickets) = game.tickets
            && let Some(loser) = (0..2u8).find(|&t| {
                tickets[usize::from(t)] == 0 && !game.bodies.iter().any(|b| b.team == t && b.alive)
            })
        {
            out.winner = Some(1 - loser);
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
    out.walled = game.walled;
    out.hidden = game.hidden;
    out.rays_per_bot_s = (super::aimpoint::rays_cast() - rays_before - game.metric_rays) as f32
        / teams.len() as f32
        / out.end_s.max(1.0);
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
            "winner {:?} at {} s; kills {:?}; delivered {:?}; stolen {}; hits {} of {} shots, {} into a wall short of the target, {} at a hidden target; {:.1} sight and aim rays per bot a second; least moved {:.0} m; stuck notes {}",
            out.winner,
            out.end_s,
            out.kills,
            out.delivered,
            out.stolen,
            out.hits,
            out.shots,
            out.walled,
            out.hidden,
            out.rays_per_bot_s,
            out.least_moved_m,
            out.stuck_notes
        );
    }
    out
}

/// A short run every build: two minutes of 4 against 4, fights and no
/// input out of the rules.
#[test]
fn a_short_soak_has_fights_and_keeps_the_input_rules() {
    let out = soak(4, 3, 0, 120, 3);
    assert!(
        out.kills[0] + out.kills[1] >= 2,
        "{:?} ({} hits of {} shots)",
        out.kills,
        out.hits,
        out.shots
    );
    assert!(out.least_moved_m > 20.0, "{}", out.least_moved_m);
    // Never through the world (`aimpoint`): before it, 48 of 416 shots
    // went into a wall short of the target or at one of whom nothing
    // showed; the few left are a tick's turn or step.
    assert!(
        out.walled + out.hidden <= out.shots / 33,
        "{} into a wall, {} at a hidden target, of {} shots",
        out.walled,
        out.hidden,
        out.shots
    );
}

/// The short run with a tiny search budget: nearly every route search goes
/// on over several polls ([`Searching`]), its bot walking the way so far,
/// and the bots still move, fight and keep the input rules.
#[test]
fn a_short_soak_with_its_searches_in_slices_still_plays() {
    let before = super::path::searches();
    let mut tuning = Tuning::default();
    tuning.think.search_floors_per_poll = 300;
    let out = soak_tuned(4, 3, 0, None, 120, 3, tuning);
    let after = super::path::searches();
    assert!(
        after.sliced - before.sliced >= 50,
        "{} of {} searches sliced",
        after.sliced - before.sliced,
        after.searches - before.searches
    );
    assert!(
        out.kills[0] + out.kills[1] >= 2,
        "{:?} ({} hits of {} shots)",
        out.kills,
        out.hits,
        out.shots
    );
    assert!(out.least_moved_m > 20.0, "{}", out.least_moved_m);
    // Its bots are gone, and with them this thread's search arrays.
    assert_eq!(mapdata::search_memory_bytes(), 0);
}

/// The whole match (minutes of play: `cargo test --release -- --ignored`).
#[test]
#[ignore = "a whole match; run in release"]
fn four_bots_a_side_fight_and_bring_batteries_to_a_win() {
    let out = soak(4, 3, 0, 900, 1);
    eprintln!("{}", out.decisions.line());
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
#[ignore = "a whole match; run in release"]
fn a_team_deathmatch_of_bots_is_fought_to_the_last_ticket() {
    // 25 tickets a side: the endgame (no respawns, the hunt) comes.
    let out = soak(4, 0, 25, 900, 2);
    eprintln!("{}", out.decisions.line());
    assert!(
        out.winner.is_some(),
        "a winner in {} s: {:?}",
        out.end_s,
        out.kills
    );
    assert!(out.end_s < 899.0, "ended by the tickets: {} s", out.end_s);
    assert!(
        out.kills[0] > 0 && out.kills[1] > 0,
        "{:?} ({} hits, {} shots)",
        out.kills,
        out.hits,
        out.shots
    );
    assert!(out.least_moved_m > 20.0, "{}", out.least_moved_m);
}

/// Timed batteries: they come by the stations over time; bots wait there
/// and bring them home.
#[test]
#[ignore = "a whole match; run in release"]
fn timed_batteries_are_waited_for_at_the_stations_and_brought_home() {
    let out = soak_mode(4, 4, 0, Some(20), 900, 4);
    eprintln!("{}", out.decisions.line());
    assert!(
        out.delivered[0] + out.delivered[1] >= 2,
        "{:?}",
        out.delivered
    );
    assert!(
        out.notes.iter().any(|n| n.text.contains("takes Station")),
        "no station waits"
    );
    assert!(out.winner.is_some());
}

/// Where the time of the bots' polls goes, seconds.
#[derive(Default)]
struct Times {
    polls: u64,
    view: f64,
    commander: f64,
    think: f64,
    ticks: f64,
    /// Each poll's whole time, µs.
    each: Vec<f32>,
    /// The longest of each part in one poll, µs.
    worst: [f32; 4],
}

/// The default tuning, with `SRP_BENCH_SEARCH_FLOORS=<n>` (`max`: no bound)
/// as the floors a poll's searches may expand: the whole matches and the
/// real-map runs at other budgets.
fn bench_tuning() -> Tuning {
    let mut tuning = Tuning::default();
    if let Ok(floors) = std::env::var("SRP_BENCH_SEARCH_FLOORS") {
        tuning.think.search_floors_per_poll = if floors == "max" {
            u32::MAX
        } else {
            floors
                .parse()
                .expect("SRP_BENCH_SEARCH_FLOORS: a number or max")
        };
    }
    tuning
}

/// [`Bots::poll`] with each part timed; the same steps in the same order.
fn timed_poll(bots: &mut Bots, clock: Clock, times: &mut Times) -> (Vec<Made>, Vec<Note>) {
    use std::time::Instant;
    let started = Instant::now();
    let (mut made, mut notes) = (Vec::new(), Vec::new());
    let now = clock.clock;
    let world = bots.tactics.as_ref().map(|t| t.world.clone());
    bots.tracker.held_sounds(now);
    if let Some(tactics) = bots.tactics.as_mut() {
        tactics.hazards.advance(now);
    }
    let view = bots
        .tracker
        .view(&clock, &bots.tuning.kin, world.as_deref());
    let viewed = Instant::now();
    if bots.tactics.is_some() {
        for team in 0..2u8 {
            if now >= bots.teams[usize::from(team)].next_commander {
                bots.commander(team, &view, &mut notes);
            }
        }
    }
    let commanded = Instant::now();
    let (mut think, mut ticks) = (0.0, 0.0);
    let mut rays = RayBudget(bots.tuning.think.rays_per_poll);
    bots.searching.left = SEARCHES_PER_POLL;
    bots.searching.floors = bots.tuning.think.search_floors_per_poll as usize;
    let a = Instant::now();
    bots.go_on_searching();
    ticks += a.elapsed().as_secs_f64();
    for index in 0..bots.bots.len() {
        if bots.bots[index].is_none() {
            continue;
        }
        let a = Instant::now();
        bots.think(index, &view, &mut rays, &mut notes);
        let b = Instant::now();
        bots.ticks(index, &clock, &view, &mut made);
        think += (b - a).as_secs_f64();
        ticks += b.elapsed().as_secs_f64();
    }
    let parts = [
        (viewed - started).as_secs_f64(),
        (commanded - viewed).as_secs_f64(),
        think,
        ticks,
    ];
    times.polls += 1;
    times.view += parts[0];
    times.commander += parts[1];
    times.think += parts[2];
    times.ticks += parts[3];
    for (worst, part) in times.worst.iter_mut().zip(parts) {
        *worst = worst.max(part as f32 * 1e6);
    }
    times.each.push(started.elapsed().as_secs_f32() * 1e6);
    (made, notes)
}

/// The cost of the bots on the real maps, only with
/// `SRP_MAPDATA_DIR=<mapdata-dump files>` (and `SRP_BOT_ROUTES` for the
/// human routes): the stand-in game on a real map, polled every 5 ms as
/// the match server's loop does with nobody sending, each part of the poll
/// timed (the stand-in's own time not counted).
///
/// ```text
/// SRP_MAPDATA_DIR=<dir> SRP_BOT_ROUTES=<json> cargo test -p match-server --release real_bot_cost -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement; needs SRP_MAPDATA_DIR"]
fn real_bot_cost() {
    use std::time::Instant;
    let Some(dir) = std::env::var_os("SRP_MAPDATA_DIR").map(std::path::PathBuf::from) else {
        eprintln!("skipping: SRP_MAPDATA_DIR is not set");
        return;
    };
    let table = std::env::var_os("SRP_BOT_ROUTES")
        .map(|path| super::routes::RouteTable::load(std::path::Path::new(&path)).unwrap());
    let limit_s: u32 = std::env::var("SRP_BENCH_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(240);
    let sizes: Vec<usize> = std::env::var("SRP_BENCH_SIDES")
        .ok()
        .map(|s| s.split(',').filter_map(|n| n.parse().ok()).collect())
        .unwrap_or(vec![4, 8, 10]);
    // Another match of the same map and size (0: the digests' runs).
    let seed: u32 = std::env::var("SRP_BENCH_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    use survarium::v026g::maps::mode;
    let maps: Vec<(u8, u8)> = std::env::var("SRP_BENCH_MAPS")
        .ok()
        .map(|s| {
            s.split(',')
                .filter_map(|m| {
                    let (level, mode) = m.split_once('/')?;
                    Some((level.parse().ok()?, mode.parse().ok()?))
                })
                .collect()
        })
        .unwrap_or(vec![
            (6, mode::BATTERIES),
            (1, mode::TEAM_DEATHMATCH),
            (3, mode::TIMED_BATTERIES),
        ]);
    for (level, mode) in maps {
        let project = format!("level_{level:02}/day_{mode}");
        let started = Instant::now();
        let map = mapdata::MapData::read(&dir.join(mapdata::format::file_name(&project))).unwrap();
        let world = Arc::new(World::build(map, mapdata::CELL, mapdata::Body::default()));
        let built_s = started.elapsed().as_secs_f32();
        for &per_team in &sizes {
            let teams: Vec<u8> = (0..2 * per_team).map(|p| u8::from(p >= per_team)).collect();
            let driven = vec![true; teams.len()];
            let roles: Vec<Option<&str>> = vec![None; teams.len()];
            let mut bots = Bots::new(
                0xBE7C + per_team as u32 + seed * 1000,
                mode,
                level,
                &teams,
                &driven,
                &roles,
                Arc::new(bench_tuning()),
                Trace::none(0),
            );
            if let Some(table) = &table {
                bots.set_routes(table);
            }
            let started = Instant::now();
            let summary = bots.map_ready(world.clone());
            let ready_ms = started.elapsed().as_secs_f32() * 1e3;
            let (tickets, timed) = match mode {
                mode::TEAM_DEATHMATCH => (Some(60), None),
                mode::TIMED_BATTERIES => (None, Some(20_000)),
                _ => (None, None),
            };
            let mut game = Game::new(
                world.clone(),
                &teams,
                per_team as u64 + u64::from(seed) * 1000,
                tickets,
                timed,
            );
            for place in 0..teams.len() {
                bots.entered(place, true, 0);
            }
            let mut times = Times::default();
            let mut decisions = Decisions::default();
            let mut plays: std::collections::BTreeMap<String, u32> = Default::default();
            super::path::reset_worst();
            let searches_before = super::path::searches();
            let rays_before = super::aimpoint::rays_cast();
            let limit_ms = limit_s * 1000;
            let (mut clock, mut next_world) = (0, 1_000);
            // Living bots' ticks with a goal to walk to, and those of them
            // with no route: a bot standing where it means to walk.
            let (mut goal_ticks, mut stood_ticks) = (0u64, 0u64);
            let mut stood_by: std::collections::BTreeMap<String, u64> = Default::default();
            while clock < limit_ms {
                clock += 5;
                while game.tick + TICK_MS <= clock {
                    game.step();
                    for bot in bots.bots.iter().flatten() {
                        let body = &game.bodies[bot.place];
                        if let Go::To { at, within_m } = bot.intent.go
                            && body.alive
                            && body.feet.distance_xz(at) > within_m
                        {
                            goal_ticks += 1;
                            if bot.route.is_none() {
                                stood_ticks += 1;
                                let play = bot
                                    .assignment
                                    .as_ref()
                                    .map_or("none".into(), |a| format!("{:?}", a.kind));
                                *stood_by.entry(play).or_default() += 1;
                            }
                        }
                    }
                }
                if clock >= next_world {
                    next_world += 1_000;
                    if let Some((_, fixes, health, facts)) =
                        game.world_at(clock / TICK_MS * TICK_MS - 300)
                    {
                        for (place, (fix, health)) in fixes.into_iter().zip(health).enumerate() {
                            for note in bots.fixed(place, fix, Some(health)) {
                                decisions.note(&note);
                            }
                        }
                        for note in bots.facts(facts) {
                            decisions.note(&note);
                        }
                    }
                }
                let clock_now = Clock {
                    clock,
                    time_left_s: (limit_ms - clock) as f32 / 1000.0,
                    total_s: limit_s as f32,
                };
                let (made, notes) = timed_poll(&mut bots, clock_now, &mut times);
                for note in notes {
                    decisions.note(&note);
                    let key: String = note
                        .text
                        .split(|c: char| c == '(' || c == ':' || c.is_ascii_digit())
                        .next()
                        .unwrap_or("")
                        .to_owned();
                    *plays.entry(key).or_default() += 1;
                }
                for made in made {
                    decisions.made(&made);
                    let r = made.input.rotation;
                    let turn = (
                        half_value(u16::from_le_bytes([r[0], r[1]])),
                        half_value(u16::from_le_bytes([r[2], r[3]])),
                    );
                    let bot = bots.bots[made.place].as_ref();
                    let target = bot.and_then(|bot| bot.reflex.target());
                    let gun = bot.map_or(Arms::fallback().primary.0, |bot| bot.reflex.held_gun());
                    game.inputs[made.place].push_back((
                        made.time,
                        made.input.actions,
                        turn,
                        target,
                        gun,
                    ));
                }
            }
            let rays = super::aimpoint::rays_cast() - rays_before - game.metric_rays;
            let searches = super::path::searches();
            let (searched, failed, search_us) = (
                searches.searches - searches_before.searches,
                searches.failed - searches_before.failed,
                searches.us - searches_before.us,
            );
            let stages: Vec<(u64, f64)> = (0..4)
                .map(|k| {
                    let n = searches.stages[k] - searches_before.stages[k];
                    let us = searches.stage_us[k] - searches_before.stage_us[k];
                    (n, us / 1e3 / n.max(1) as f64)
                })
                .collect();
            eprintln!(
                "  by stage (no way, corridor, widened, whole): (searches, ms each) {stages:?}; worst search {:.1} ms",
                searches.worst_us / 1e3
            );
            let by_expanded: Vec<u64> = (0..8)
                .map(|k| searches.by_expanded[k] - searches_before.by_expanded[k])
                .collect();
            eprintln!(
                "  floors expanded: {} in all, worst search {}; searches by floors expanded (up to {:?}, more): {by_expanded:?}; sliced {} in {} slices",
                searches.expanded - searches_before.expanded,
                searches.worst_expanded,
                super::path::EXPANDED_BUCKETS,
                searches.sliced - searches_before.sliced,
                searches.slices - searches_before.slices,
            );
            eprintln!(
                "  grid searches {searched} ({:.1} a bot a second), {failed} failed; {:.2} ms each on average; {:.0} % of the bots' time",
                searched as f64 / teams.len() as f64 / limit_s as f64,
                search_us / 1e3 / searched.max(1) as f64,
                search_us / 1e4 / (times.view + times.commander + times.think + times.ticks)
            );
            if std::env::var_os("SRP_BENCH_WHY").is_some() {
                let tactics = bots.tactics.as_ref().unwrap();
                let grid = &tactics.world.grid;
                for bot in bots.bots.iter().flatten() {
                    let me = game.bodies[bot.place].feet;
                    let goal = match bot.intent.go {
                        Go::To { at, .. } => Some(at),
                        _ => None,
                    };
                    let from = grid.locate(me, 1.5);
                    let to = goal.and_then(|g| grid.locate(g, 3.0));
                    let open = from.zip(to).map(|(a, b)| path::plan(grid, a, b).is_some());
                    let allowed = from.zip(to).map(|(a, b)| {
                        path::plan_where(grid, a, b, |n| tactics.allowed(n, clock, bot.side, false))
                            .is_some()
                    });
                    let objective = from.zip(to).map(|(a, b)| {
                        path::plan_where(grid, a, b, |n| tactics.allowed(n, clock, bot.side, true))
                            .is_some()
                    });
                    eprintln!(
                        "  bot {} team {} side {:?} at {me:?} goal {goal:?} objective {} from {from:?} to {to:?}: path open {open:?}, allowed {allowed:?}, objective-allowed {objective:?}; play {:?}",
                        bot.place,
                        bot.team,
                        bot.side,
                        bot.intent.objective,
                        bot.assignment.as_ref().map(|a| (a.kind, a.target)),
                    );
                }
            }
            let mut top: Vec<_> = plays.into_iter().collect();
            top.sort_by(|a, b| b.1.cmp(&a.1));
            eprintln!("  notes: {:?}", &top[..top.len().min(12)]);
            eprintln!(
                "  stood with a goal and no route: {stood_ticks} of {goal_ticks} bot ticks ({:.1} %); delivered {:?}",
                100.0 * stood_ticks as f64 / goal_ticks.max(1) as f64,
                game.delivered
            );
            let mut by: Vec<_> = stood_by.into_iter().collect();
            by.sort_by(|a, b| b.1.cmp(&a.1));
            eprintln!("  stood by play: {:?}", &by[..by.len().min(8)]);
            let line = format!("{project} {per_team}v{per_team}: {}", decisions.line());
            eprintln!("  {line}");
            if let Some(out) = std::env::var_os("SRP_DECISIONS_OUT") {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(out)
                    .unwrap();
                writeln!(file, "{line}").unwrap();
            }
            let total = times.view + times.commander + times.think + times.ticks;
            let mut each = std::mem::take(&mut times.each);
            each.sort_by(f32::total_cmp);
            let at = |q: f64| each[((each.len() - 1) as f64 * q) as usize];
            let kills: u32 = game.stats.iter().map(|s| s.0).sum();
            eprintln!(
                "{project} {per_team}v{per_team}: world built in {built_s:.2} s, map_ready {ready_ms:.1} ms ({summary})\n  \
                 {limit_s} s of play: bots {:.1} ms a second = {:.2} % of a core; per bot {:.3} %\n  \
                 poll µs p50 {:.0}, p99 {:.0}, p99.9 {:.0}, max {:.0}; {} polls\n  \
                 parts: view {:.0} %, commander {:.0} %, think {:.0} %, ticks {:.0} %; worst µs view {:.0}, commander {:.0}, think {:.0}, ticks {:.0}\n  \
                 rays {:.1} per bot a second; kills {kills}, shots {}, hits {}",
                total * 1e3 / limit_s as f64,
                total * 100.0 / limit_s as f64,
                total * 100.0 / limit_s as f64 / teams.len() as f64,
                at(0.5),
                at(0.99),
                at(0.999),
                each[each.len() - 1],
                times.polls,
                100.0 * times.view / total,
                100.0 * times.commander / total,
                100.0 * times.think / total,
                100.0 * times.ticks / total,
                times.worst[0],
                times.worst[1],
                times.worst[2],
                times.worst[3],
                rays as f64 / teams.len() as f64 / limit_s as f64,
                game.shots,
                game.hits,
            );
        }
    }
}

/// A body at `x` on the line z = 0, alive or not, in a world at `time`.
fn standing(time: u32, x: f32, alive: bool) -> Fix {
    Fix {
        time,
        alive,
        inserted: 100,
        feet: alive.then(|| Vec3::new(x, 0.0, 0.0)),
        yaw: 0.0,
        pitch: 0.0,
        keys: 0,
        speed: Some(0.0),
    }
}

/// Which bot gives its place up to a late joiner (`Bots::to_give_way`): a
/// dead one first, else the one farthest from the fight; never a carrier,
/// never a living one of a side that can no longer respawn; and one that
/// left is forgotten by the others.
#[test]
fn the_bot_that_gives_way_is_dead_or_far_from_the_fight_and_carries_nothing() {
    use survarium::v026g::maps::mode;
    // A human of team 0 at x = 0; team 1's bots at 5, 40 and 30 m.
    let teams = [0, 1, 1, 1];
    let driven = [false, true, true, true];
    let mut bots = Bots::new(
        0x6A7E,
        mode::BATTERIES,
        0,
        &teams,
        &driven,
        &[None; 4],
        Arc::new(Tuning::default()),
        Trace::none(0),
    );
    for place in 0..4 {
        bots.entered(place, true, 100);
    }
    let world = |bots: &mut Bots, time: u32, alive: [bool; 4]| {
        for (place, x) in [0.0, 5.0, 40.0, 30.0].into_iter().enumerate() {
            bots.fixed(place, standing(time, x, alive[place]), Some(1.0));
        }
    };
    world(&mut bots, 1_000, [true; 4]);
    let candidates = [1, 2, 3];
    assert_eq!(bots.to_give_way(&candidates, 1_000), Some(2));
    // Bot 2 picks up the battery lying at its feet: bot 3 goes instead.
    let lying = Vec3::new(40.0, 0.3, 0.0);
    bots.facts(WorldFacts {
        time: 1_000,
        batteries: vec![BatteryState::Free(lying)],
        ..WorldFacts::default()
    });
    bots.facts(WorldFacts {
        time: 2_000,
        batteries: vec![BatteryState::Carried(None)],
        ..WorldFacts::default()
    });
    assert_eq!(bots.to_give_way(&candidates, 2_000), Some(3));
    // Bot 1, nearest the fight, is dead: it goes first.
    world(&mut bots, 3_000, [true, false, true, true]);
    assert_eq!(bots.to_give_way(&candidates, 3_000), Some(1));
    // Its side can no longer respawn: only the dead one may go, and with
    // none dead, none.
    bots.facts(WorldFacts {
        time: 4_000,
        can_respawn: Some([true, false]),
        ..WorldFacts::default()
    });
    assert_eq!(bots.to_give_way(&candidates, 4_000), Some(1));
    world(&mut bots, 5_000, [true; 4]);
    assert_eq!(bots.to_give_way(&candidates, 5_000), None);

    // Bot 3 leaves: it is forgotten, and no partner of anyone.
    bots.bots[1].as_mut().unwrap().partner = Some(3);
    bots.entered(3, false, 6_000);
    assert!(!bots.is_bot(3) && bots.is_bot(1));
    assert_eq!(bots.bots[1].as_ref().unwrap().partner, None);
    assert!(
        !bots
            .tracker()
            .view(
                &Clock {
                    clock: 6_000,
                    time_left_s: 0.0,
                    total_s: 0.0
                },
                &bots.tuning.kin,
                None
            )
            .places[3]
            .present
    );
    let (made, _) = bots.poll(Clock {
        clock: 9_000,
        time_left_s: 600.0,
        total_s: 900.0,
    });
    assert!(made.iter().all(|made| made.place != 3));
}
