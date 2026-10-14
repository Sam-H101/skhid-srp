//! Play 17 in its first form, and every bot's default: go forward toward
//! the enemy base's exits (where humans fight best, `player_bots_behaviour.md`
//! §3 play 15), by a way of its own.
//!
//! The routes of humans per map are not in the bots' data yet (they are a
//! private table); until then a route's variety comes from a waypoint off
//! the straight way, drawn per run (research B2 "dual utility": not always
//! the best). The team spreads over the exits (`FriendsNearTarget`
//! inverted). It sprints only far from known danger
//! ([`Ctx::pace_toward`]). At the exit it looks about a few seconds, then
//! the commander picks again.

use super::*;
use crate::v026g::bots::curve::{Consideration, Curve, Input};

pub const NUMBERS: &[(&str, f32)] = &[
    ("detour_min_m", 10.0),
    ("detour_max_m", 30.0),
    ("look_min_s", 3.0),
    ("look_max_s", 8.0),
    ("give_up_s", 90.0),
];

pub fn tuning() -> PlayTuning {
    PlayTuning {
        rank: Rank::Roam,
        considerations: vec![
            considerations::near(250.0),
            Consideration::new(Input::FriendsNearTarget, 0.0, 3.0, Curve::Linear).inverted(),
        ],
        numbers: Default::default(),
    }
}

pub fn candidates(cx: &Ctx, out: &mut Vec<Candidate>) {
    let Some(enemy) = cx.bot.side.and_then(|side| cx.enemy_side(side)) else {
        // No side known yet: anywhere ahead will do.
        out.extend(cx.candidate(PlayKind::Roam, TargetRef::None, None));
        return;
    };
    let exits = cx.tactics.exits(enemy);
    for index in 0..exits.len() {
        out.extend(cx.candidate(PlayKind::Roam, TargetRef::Exit(index), None));
    }
    if exits.is_empty() {
        out.extend(cx.candidate(PlayKind::Roam, TargetRef::None, None));
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// To the waypoint off the straight way, then to the goal.
    Via {
        via: Vec3,
        goal: Vec3,
    },
    Going {
        goal: Vec3,
    },
    Looking {
        until: u32,
    },
}

impl Ctx<'_> {
    /// Run (sprint where the way allows) unless a known danger is within
    /// 30 m of the bot or of `at`; walk then (play 17's sprint rule).
    pub fn pace_toward(&self, at: Vec3) -> Pace {
        let near = |p: Vec3| {
            self.memory
                .tracks()
                .iter()
                .any(|t| t.at.distance_xz(p) < 30.0)
                || self
                    .team
                    .danger_within(self.now, 20_000)
                    .any(|d| d.at.distance_xz(p) < 30.0)
        };
        if near(self.me.feet) || near(at) {
            Pace::Walk
        } else {
            Pace::Run
        }
    }

    /// Where roaming heads for `target`: the exit, else a floor ahead
    /// toward the other side's spawns.
    fn roam_goal(&self, target: TargetRef) -> Option<Vec3> {
        self.point_of(target).or_else(|| {
            let enemy = self.bot.side.and_then(|side| self.enemy_side(side))?;
            let spawns: Vec<Vec3> = self
                .tactics
                .world
                .map
                .spawns
                .iter()
                .filter(|s| s.team == enemy)
                .map(|s| s.position)
                .collect();
            let first = *spawns.first()?;
            // Halfway there: never into their spawn area.
            Some(self.me.feet + (first - self.me.feet) * 0.5)
        })
    }
}

pub fn start(target: TargetRef, cx: &Ctx, rng: &mut Rng) -> State {
    // The exit or the way ahead; with no side known, a floor 20-60 m off.
    let goal = cx.roam_goal(target).or_else(|| {
        let grid = &cx.tactics.world.grid;
        (0..32).find_map(|_| {
            let point = grid.position(rng.below(grid.len()) as mapdata::NodeId);
            ((20.0..=60.0).contains(&point.distance_xz(cx.me.feet))
                && cx.tactics.fine_goal(point, cx.now, cx.bot.side))
            .then_some(point)
        })
    });
    let Some(goal) = goal else {
        return State::Looking {
            until: cx.now + 2_000,
        };
    };
    let me = cx.me.feet;
    let (dx, dz) = (goal.x - me.x, goal.z - me.z);
    let length = dx.hypot(dz);
    if length < 30.0 {
        return State::Going { goal };
    }
    for _ in 0..6 {
        let offset = rng.range(
            cx.num(PlayKind::Roam, "detour_min_m"),
            cx.num(PlayKind::Roam, "detour_max_m"),
        ) * if rng.unit() < 0.5 { -1.0 } else { 1.0 };
        let t = rng.range(0.35, 0.65);
        let via = Vec3::new(
            me.x + dx * t - dz / length * offset,
            me.y,
            me.z + dz * t + dx / length * offset,
        );
        if cx.tactics.fine_goal(via, cx.now, cx.bot.side) {
            return State::Via { via, goal };
        }
    }
    State::Going { goal }
}

pub fn step(state: &mut State, _target: TargetRef, cx: &Ctx, rng: &mut Rng) -> Step {
    if let Some((_, _, started)) = cx.bot.current
        && cx.now > started + (cx.num(PlayKind::Roam, "give_up_s") * 1000.0) as u32
    {
        return Step::Done("roamed long enough");
    }
    match state {
        State::Via { via, goal } => {
            let (via, goal) = (*via, *goal);
            if cx.me.feet.distance_xz(via) < 4.0 {
                *state = State::Going { goal };
            }
            Step::Continue(Intent::to(via, cx.pace_toward(via)).within(3.0))
        }
        State::Going { goal } => {
            let goal = *goal;
            if cx.me.feet.distance_xz(goal) < 3.0 {
                let look = rng.range(
                    cx.num(PlayKind::Roam, "look_min_s"),
                    cx.num(PlayKind::Roam, "look_max_s"),
                );
                *state = State::Looking {
                    until: cx.now + (look * 1000.0) as u32,
                };
                return Step::Continue(Intent::stay());
            }
            Step::Continue(Intent::to(goal, cx.pace_toward(goal)).within(2.0))
        }
        State::Looking { until } => {
            if cx.now >= *until {
                return Step::Done("looked about at its goal");
            }
            Step::Continue(Intent::stay().crouched())
        }
    }
}
