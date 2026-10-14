//! The match server's bots that walk the map: stage 2 of the player bots
//! (`docs/notes/player_bots.md` §6). The layers are the research's
//! (`docs/notes/player_bots_research.md` B1, B6), so far the ones walking
//! needs:
//!
//! - [`body`]: where the bot's body is, from the worlds the server is given
//!   and the bot's own inputs since;
//! - [`path`]: routes on the walk grid, pulled straight;
//! - [`motor`]: the keys and the view's turns of each tick, smooth as a
//!   player's hands;
//! - [`stuck`]: knowing it is stuck, and getting out;
//! - [`encode`]: the inputs a client would send for them;
//! - [`kin`]: the movement model; [`rng`]: each bot's own random numbers;
//! - [`maps`]: the maps' walk grids, built off the match's thread.
//!
//! What a bot does is the one play of this stage, wandering: it walks to
//! one of the map's objectives (a container, a battery's place, a station)
//! or a floor 20 to 60 m off, looks about there for a few seconds, and
//! picks the next. It never heads into the other side's spawn area. Plays
//! with a purpose come in the next stage.
//!
//! [`Bots`] holds a match's bots and is all the match (`game/driven.rs`)
//! sees of them: the entry of each, the worlds' word on their bodies
//! ([`body::Fix`]), and the inputs they make.

pub mod body;
pub mod encode;
pub mod kin;
pub mod maps;
pub mod motor;
pub mod path;
pub mod rng;
pub mod stuck;

use mapdata::{NodeId, Vec3, World};
use survarium::v026g::match_msgs::{PlayerInput, TICK_MS};

use crate::v026g::drive::key;
use body::{BodyTrack, Fix};
use encode::{Decision, Encoder};
use kin::Kinematics;
use motor::{Feel, Motor, Pose, Walked};
use path::Route;
use rng::Rng;
use stuck::{Recovery, Stuck};

/// From a bot's entry to its first move, ms (as the stage 0 script).
const START_AFTER_MS: u32 = 2_000;
/// A goal is never this near a spawn point of the other side, metres.
const ENEMY_SPAWN_M: f32 = 25.0;
/// Goals are sought this far off, metres (nearer and farther weigh less).
const GOAL_NEAR_M: f32 = 15.0;
const GOAL_FAR_M: f32 = 120.0;
/// A body farther than this off its route plans again, metres.
const OFF_ROUTE_M: f32 = 3.0;
/// A world that puts the body this far from where it was reckoned plans
/// the route again, metres.
const REPLAN_ERROR_M: f32 = 2.0;
/// At a goal, it stands this long, seconds.
const PAUSE_S: (f32, f32) = (1.5, 5.0);
/// Having given up on a stuck spot, it stands this long, seconds.
const GIVE_UP_S: (f32, f32) = (2.0, 4.0);
/// A floor the goal is looked for within, metres.
const GOAL_LOCATE_M: f32 = 3.0;
/// Goals tried before a bot stands a moment and tries again.
const GOAL_TRIES: usize = 4;

/// An input a bot made.
#[derive(Clone, Debug)]
pub struct Made {
    pub place: usize,
    /// Its tick's time on the match clock.
    pub time: u32,
    pub input: PlayerInput,
}

/// Something worth a line in the log.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub place: usize,
    pub time: u32,
    pub text: String,
}

/// Where a bot is going: a point, and the objective it is, if one.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Goal {
    at: Vec3,
    objective: Option<usize>,
}

/// After a step to get unstuck.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Then {
    SameGoal(Goal),
    OtherGoal,
}

#[derive(Clone, Debug)]
enum Doing {
    /// No goal yet (or it was given up).
    Choosing,
    Walking {
        route: Route,
        goal: Goal,
    },
    /// Standing, looking about, until the tick at `until`.
    Pausing {
        until: u32,
    },
    /// Holding `keys` for `ticks` more ticks to get unstuck (a jump at the
    /// first), then `then`.
    Unsticking {
        keys: u32,
        ticks: u32,
        jump: bool,
        then: Then,
    },
}

/// One bot of the match.
pub struct Bot {
    place: usize,
    rng: Rng,
    encoder: Encoder,
    track: BodyTrack,
    motor: Motor,
    doing: Doing,
    stuck: Stuck,
    /// The time of its entry (its 0xE9) on the match clock.
    entered: Option<u32>,
    /// The spawns' team it came out of first: its side on the map.
    side: Option<i32>,
    last_objective: Option<usize>,
    /// The last decided tick.
    now: u32,
}

impl Bot {
    pub fn new(match_id: u32, place: usize, feel: Feel) -> Self {
        let mut rng = Rng::for_bot(match_id, place);
        let motor = Motor::new(feel, &mut rng);
        Self {
            place,
            rng,
            encoder: Encoder::default(),
            track: BodyTrack::default(),
            motor,
            doing: Doing::Choosing,
            stuck: Stuck::default(),
            entered: None,
            side: None,
            last_objective: None,
            now: 0,
        }
    }

    pub fn track(&self) -> &BodyTrack {
        &self.track
    }

    /// Whether it waits for a world to see its body: entered, and no world
    /// since showed it alive.
    pub fn needs_world(&self) -> bool {
        self.entered.is_some() && self.track.estimate().is_none()
    }

    pub fn walking(&self) -> bool {
        matches!(self.doing, Doing::Walking { .. })
    }

    fn note(&self, notes: &mut Vec<Note>, time: u32, text: String) {
        notes.push(Note {
            place: self.place,
            time,
            text,
        });
    }

    /// A world's word on its body.
    pub fn fixed(
        &mut self,
        fix: Fix,
        kin: &Kinematics,
        world: Option<&World>,
        notes: &mut Vec<Note>,
    ) {
        let (time, feet) = (fix.time, fix.feet);
        let new_life = self.track.fixed(fix, kin, world.map(|world| &world.grid));
        if new_life {
            self.doing = Doing::Choosing;
            self.stuck.reset();
            if let (Some(world), Some(feet), None) = (world, feet, self.side) {
                self.side = world
                    .map
                    .spawns
                    .iter()
                    .min_by(|a, b| {
                        a.position
                            .distance_xz(feet)
                            .total_cmp(&b.position.distance_xz(feet))
                    })
                    .map(|spawn| spawn.team);
            }
            self.note(
                notes,
                time,
                format!(
                    "is alive at {:?} (side {:?})",
                    feet.map(Vec3::to_array),
                    self.side
                ),
            );
            return;
        }
        if self.track.dead() {
            if !matches!(self.doing, Doing::Choosing) {
                self.note(notes, time, "is dead".to_owned());
            }
            self.doing = Doing::Choosing;
            self.stuck.reset();
            return;
        }
        let (Some(feet), Doing::Walking { goal, .. }) = (feet, &self.doing) else {
            return;
        };
        let goal = *goal;
        if let Some(recovery) = self.stuck.walking(time, feet) {
            self.recover(recovery, goal, time, feet, notes);
            return;
        }
        let error = self.track.last_error().unwrap_or(0.0);
        if error > REPLAN_ERROR_M
            && let Some(world) = world
        {
            self.note(
                notes,
                time,
                format!("was {error:.1} m off its reckoning; plans again"),
            );
            self.walk_to(world, goal, time, notes);
        }
    }

    /// Stuck at `feet`: a step to get out, as [`stuck`] says.
    fn recover(
        &mut self,
        recovery: Recovery,
        goal: Goal,
        time: u32,
        feet: Vec3,
        notes: &mut Vec<Note>,
    ) {
        let side = if self.rng.unit() < 0.5 {
            key::LEFT
        } else {
            key::RIGHT
        };
        self.note(
            notes,
            time,
            format!("is stuck at {:?}: {recovery:?}", feet.to_array()),
        );
        self.doing = match recovery {
            Recovery::BackOff => Doing::Unsticking {
                keys: key::BACK | side,
                ticks: (self.rng.range(0.5, 0.8) / kin::TICK_S) as u32,
                jump: true,
                then: Then::SameGoal(goal),
            },
            Recovery::Sidestep => {
                self.last_objective = goal.objective;
                Doing::Unsticking {
                    keys: side,
                    ticks: (self.rng.range(0.8, 1.3) / kin::TICK_S) as u32,
                    jump: true,
                    then: Then::OtherGoal,
                }
            }
            Recovery::GiveUp => {
                self.last_objective = goal.objective;
                let pause = self.rng.range(GIVE_UP_S.0, GIVE_UP_S.1);
                Doing::Pausing {
                    until: self.now + (pause * 1000.0) as u32,
                }
            }
        };
    }

    /// The route to `goal`, if there is one (that keeps clear of the spots
    /// it got stuck at, with `avoid`).
    fn route_to(&self, world: &World, goal: Goal, time: u32, avoid: bool) -> Option<Route> {
        let at = self.track.estimate()?;
        let from = at.node.or_else(|| world.grid.locate(at.feet, 1.5))?;
        let to = world.grid.locate(goal.at, GOAL_LOCATE_M)?;
        path::plan(&world.grid, from, to).filter(|route| !avoid || self.stuck.clear(time, route))
    }

    /// Walks to `goal`; stands a moment and chooses again if there is no
    /// route. The stuck watch goes on: a route planned again to the same
    /// goal may run into the same trap.
    fn walk_to(&mut self, world: &World, goal: Goal, time: u32, notes: &mut Vec<Note>) -> bool {
        self.walk_to_avoiding(world, goal, time, true, notes)
    }

    fn walk_to_avoiding(
        &mut self,
        world: &World,
        goal: Goal,
        time: u32,
        avoid: bool,
        notes: &mut Vec<Note>,
    ) -> bool {
        let Some(route) = self.route_to(world, goal, time, avoid) else {
            self.last_objective = goal.objective;
            self.doing = Doing::Pausing {
                until: self.now + 1_000,
            };
            return false;
        };
        self.note(
            notes,
            time,
            format!(
                "walks {:.0} m to {} ({} legs)",
                route.length(),
                goal.objective
                    .map_or("a floor".to_owned(), |k| format!("objective {k}")),
                route.points().len().saturating_sub(1)
            ),
        );
        self.doing = Doing::Walking { route, goal };
        true
    }

    /// Chooses a goal and walks to it: an objective, not the last one, not
    /// near the other side's spawns or a spot it got stuck at, weighted to
    /// those at a walk's distance; else a floor 20 to 60 m off.
    /// If every route it tries passes a spot it got stuck at, it takes one
    /// anyway: the spot may be the only way.
    fn choose(&mut self, world: &World, feet: Vec3, time: u32, notes: &mut Vec<Note>) {
        self.stuck.reset();
        for avoid in [true, false] {
            for _ in 0..GOAL_TRIES {
                let goal = self.pick(world, feet, time);
                if self.walk_to_avoiding(world, goal, time, avoid, notes) {
                    return;
                }
            }
        }
        self.note(
            notes,
            time,
            "has no goal it can walk to; stands a moment".to_owned(),
        );
    }

    fn pick(&mut self, world: &World, feet: Vec3, time: u32) -> Goal {
        let objectives = goals(world);
        let side = self.side;
        let stuck = &self.stuck;
        let fine = |point: Vec3| {
            !stuck.near_spot(time, point)
                && !world.map.spawns.iter().any(|spawn| {
                    side.is_some_and(|side| spawn.team != side)
                        && spawn.position.distance_xz(point) < ENEMY_SPAWN_M
                })
        };
        let weighted: Vec<(usize, f32)> = objectives
            .iter()
            .enumerate()
            .filter(|(k, point)| {
                Some(*k) != self.last_objective
                    && point.distance_xz(feet) > GOAL_LOCATE_M
                    && fine(**point)
            })
            .map(|(k, point)| {
                let d = point.distance_xz(feet);
                let w = if d < GOAL_NEAR_M {
                    0.2
                } else if d > GOAL_FAR_M {
                    0.3
                } else {
                    1.0
                };
                (k, w)
            })
            .collect();
        let total: f32 = weighted.iter().map(|(_, w)| w).sum();
        if total > 0.0 {
            let mut pick = self.rng.range(0.0, total);
            for &(k, w) in &weighted {
                if pick < w {
                    return Goal {
                        at: objectives[k],
                        objective: Some(k),
                    };
                }
                pick -= w;
            }
        }
        for _ in 0..32 {
            let node = self.rng.below(world.grid.len()) as NodeId;
            let point = world.grid.position(node);
            if (20.0..=60.0).contains(&point.distance_xz(feet)) && fine(point) {
                return Goal {
                    at: point,
                    objective: None,
                };
            }
        }
        Goal {
            at: feet,
            objective: None,
        }
    }

    /// The decision for the tick at `time`.
    fn decide(&mut self, time: u32, world: Option<&World>, notes: &mut Vec<Note>) -> Decision {
        self.now = time;
        let (Some(world), Some(at)) = (world, self.track.estimate().copied()) else {
            return self.motor.rest();
        };
        let pose = Pose {
            feet: at.feet,
            yaw: at.yaw,
            pitch: at.pitch,
            speed: at.velocity[0].hypot(at.velocity[1]),
        };
        if matches!(self.doing, Doing::Choosing) {
            self.choose(world, at.feet, time, notes);
        }
        match &mut self.doing {
            Doing::Choosing => self.motor.rest(),
            Doing::Pausing { until } => {
                if time >= *until {
                    self.doing = Doing::Choosing;
                }
                self.motor.stand(&pose, &mut self.rng)
            }
            Doing::Unsticking {
                keys,
                ticks,
                jump,
                then,
            } => {
                let mut held = *keys;
                if std::mem::take(jump) {
                    held |= key::JUMP;
                }
                if *ticks > 0 {
                    *ticks -= 1;
                } else {
                    match *then {
                        Then::SameGoal(goal) => {
                            self.walk_to(world, goal, time, notes);
                        }
                        Then::OtherGoal => self.doing = Doing::Choosing,
                    }
                }
                let decision = self.motor.rest();
                Decision {
                    keys: held,
                    ..decision
                }
            }
            Doing::Walking { route, goal } => {
                let goal = *goal;
                let grid = &world.grid;
                let from = at.node;
                let clear = |to: Vec3| {
                    from.zip(grid.locate(to, 0.6))
                        .is_some_and(|(a, b)| path::straight(grid, a, b))
                };
                let (decision, walked) = self.motor.walk(route, &pose, OFF_ROUTE_M, clear);
                match walked {
                    Walked::OnTheWay => decision,
                    Walked::Arrived => {
                        let pause = self.rng.range(PAUSE_S.0, PAUSE_S.1);
                        self.note(
                            notes,
                            time,
                            format!("is at its goal; looks about {pause:.1} s"),
                        );
                        self.last_objective = goal.objective;
                        self.stuck.reset();
                        self.doing = Doing::Pausing {
                            until: time + (pause * 1000.0) as u32,
                        };
                        decision
                    }
                    Walked::Off(off) => {
                        self.note(
                            notes,
                            time,
                            format!("is {off:.1} m off its route; plans again"),
                        );
                        self.walk_to(world, goal, time, notes);
                        self.motor.rest()
                    }
                }
            }
        }
    }

    /// The inputs due with the clock at `clock`.
    pub fn poll(
        &mut self,
        clock: u32,
        world: Option<&World>,
        kin: &Kinematics,
        made: &mut Vec<Made>,
        notes: &mut Vec<Note>,
    ) {
        let Some(entered) = self.entered else {
            return;
        };
        for time in self.encoder.ticks(clock, entered + START_AFTER_MS) {
            let decision = self.decide(time, world, notes);
            let (input, turn) = self.encoder.encode(time, decision);
            self.track.decided(
                time,
                self.encoder.keys(),
                turn,
                kin,
                world.map(|world| &world.grid),
            );
            if let Some(input) = input {
                made.push(Made {
                    place: self.place,
                    time,
                    input,
                });
            }
        }
    }
}

/// The map's objectives a bot may wander to: containers, the batteries'
/// places, the stations.
pub fn goals(world: &World) -> Vec<Vec3> {
    let map = &world.map;
    map.containers
        .iter()
        .map(|container| container.position)
        .chain(map.battery_spawners.iter().map(|spawner| spawner.item))
        .chain(map.stations.iter().copied())
        .collect()
}

/// A match's bots, by place.
pub struct Bots {
    bots: Vec<Option<Bot>>,
    pub kin: Kinematics,
}

impl Bots {
    /// `driven[place]`: whether the place is a bot's.
    pub fn new(match_id: u32, driven: &[bool]) -> Self {
        let feel = Feel::default();
        Self {
            bots: driven
                .iter()
                .enumerate()
                .map(|(place, &driven)| driven.then(|| Bot::new(match_id, place, feel.clone())))
                .collect(),
            kin: Kinematics::default(),
        }
    }

    /// A place added to the match (a late joiner's: never a bot's).
    pub fn push_place(&mut self) {
        self.bots.push(None);
    }

    pub fn is_bot(&self, place: usize) -> bool {
        self.bots.get(place).is_some_and(Option::is_some)
    }

    pub fn any(&self) -> bool {
        self.bots.iter().any(Option::is_some)
    }

    pub fn bot(&self, place: usize) -> Option<&Bot> {
        self.bots.get(place)?.as_ref()
    }

    /// The bot at `place` entered at `time`.
    pub fn entered(&mut self, place: usize, time: u32) {
        if let Some(Some(bot)) = self.bots.get_mut(place) {
            bot.entered = Some(time);
        }
    }

    pub fn has_entered(&self, place: usize) -> bool {
        self.bot(place).is_some_and(|bot| bot.entered.is_some())
    }

    /// Whether a bot waits to see its body in a world.
    pub fn need_world(&self) -> bool {
        self.bots.iter().flatten().any(Bot::needs_world)
    }

    /// A world's word on the body of the bot at `place`.
    pub fn fixed(&mut self, place: usize, fix: Fix, world: Option<&World>) -> Vec<Note> {
        let mut notes = Vec::new();
        if let Some(Some(bot)) = self.bots.get_mut(place) {
            bot.fixed(fix, &self.kin, world, &mut notes);
        }
        notes
    }

    /// The inputs due with the clock at `clock`, and what is worth a log
    /// line.
    pub fn poll(&mut self, clock: u32, world: Option<&World>) -> (Vec<Made>, Vec<Note>) {
        let (mut made, mut notes) = (Vec::new(), Vec::new());
        for bot in self.bots.iter_mut().flatten() {
            bot.poll(clock, world, &self.kin, &mut made, &mut notes);
        }
        (made, notes)
    }
}

/// The tick grid, for the tests.
#[allow(dead_code)]
const TICK: u32 = TICK_MS;

#[cfg(test)]
mod tests;
