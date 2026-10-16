//! Play 2, carrier interception, and the answer to a theft
//! (`player_bots_behaviour.md` §3 play 2, §4 "a battery the team picked up
//! disappears" and "an enemy steals from our container"): an enemy carrier
//! is slow, cannot shoot and is bound for his container. The bot meets him
//! at a point on his likely way home that it reaches 1-2 s before him
//! (drawn per run), else chases his last known place.
//!
//! - Where he is comes only through [`Ctx::known_place`]: the bot's
//!   memory, the thief cheat included. A theft whose thief is not known
//!   yet is answered from where the battery was last known.
//! - His likely way is the route from him to his container (off our spawn
//!   area, as his own routes are); points along it are tried from him on,
//!   the first one the bot reaches `lead` seconds before him is the meet.
//!   It is planned again every few seconds and once he is past it.
//! - A recent theft of ours is an alarm (rank Alarm) for the 1-2 nearest
//!   bots of the team: two slot keys per theft, so at most two take it.
//!   Any other enemy carrier is an objective for one bot.
//! - It ends when he no longer carries (dropped, home, dead), gives up
//!   when he is lost, gets into his base, or after a time drawn per run.

use super::*;
use crate::v026g::bots::view::BatteryState;

pub const NUMBERS: &[(&str, f32)] = &[
    // A theft this recent is an alarm.
    ("theft_recent_s", 30.0),
    // The alarm goes to this many of the nearest bots.
    ("alarm_bots", 2.0),
    // A walk is the ground distance times this (as `Ctx::path_dist`).
    ("path_factor", 1.25),
    // The bot's speed on the way (sprinting where it may; GUESS between
    // walk 3.5 and sprint 5.0 m/s) and a carrier's (he does not sprint,
    // LIKELY, as `collect` assumes).
    ("my_speed", 4.5),
    ("carrier_speed", 3.5),
    // The bot is at the meet this long before him, drawn per run.
    ("lead_min_s", 1.0),
    ("lead_max_s", 2.0),
    // Points along his way are tried this far apart.
    ("meet_step_m", 4.0),
    ("meet_reach_m", 3.0),
    // He is past the meet when this much nearer his home than it.
    ("passed_m", 3.0),
    ("replan_s", 2.0),
    // A meet planned again this near the last one keeps the last one.
    ("keep_m", 5.0),
    // This near him it goes straight at him.
    ("close_m", 8.0),
    ("chase_reach_m", 2.0),
    ("give_up_min_s", 45.0),
    ("give_up_max_s", 60.0),
];

pub fn tuning() -> PlayTuning {
    PlayTuning {
        rank: Rank::Objective,
        considerations: vec![
            considerations::near(200.0),
            considerations::health_above(0.25),
            considerations::personality(),
            considerations::modifier(Input::ObjectiveUrgency, 0.6),
        ],
        numbers: Default::default(),
    }
}

/// What a target stands for while an enemy carries its battery: the
/// carrier if the view can tell (a theft's thief otherwise).
struct Carry {
    carrier: Option<Place>,
}

impl Ctx<'_> {
    fn intercept_is_enemy(&self, place: Place) -> bool {
        self.view
            .place(place)
            .is_some_and(|p| p.team != self.me.team)
    }

    /// The carry the target stands for, while an enemy carries.
    fn intercept_carry(&self, target: TargetRef) -> Option<Carry> {
        match target {
            TargetRef::Place(p) => (self.intercept_is_enemy(p)
                && self
                    .view
                    .batteries
                    .contains(&BatteryState::Carried(Some(p))))
            .then_some(Carry { carrier: Some(p) }),
            TargetRef::Point(at) => {
                let theft = self.team.thefts.iter().rev().find(|t| t.at == at)?;
                match self.view.batteries.get(theft.battery)? {
                    BatteryState::Carried(carrier)
                        if carrier.is_none_or(|p| self.intercept_is_enemy(p)) =>
                    {
                        Some(Carry {
                            carrier: carrier.or(theft.thief),
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Where the carrier is as far as the bot knows.
    fn intercept_him(&self, target: TargetRef, carry: &Carry) -> Option<Vec3> {
        let known = carry.carrier.and_then(|p| self.known_place(p));
        match target {
            TargetRef::Point(at) => Some(known.unwrap_or(at)),
            _ => known,
        }
    }

    /// His container: his team's, else the one not ours.
    fn intercept_home(&self, carry: &Carry) -> Option<Vec3> {
        let team = carry
            .carrier
            .and_then(|p| self.view.place(p))
            .map(|p| p.team);
        self.view
            .containers
            .iter()
            .find(|c| team.map_or(c.team != self.me.team, |team| c.team == team))
            .map(|c| c.at)
    }

    /// Whether a run at `target` would fail at its first step, as
    /// [`step`] plans it: no meet ahead of him (the earliest lead drawn)
    /// or he is close, and the chase's goal (he) is in his base or a zone.
    fn intercept_fails_at_once(&self, target: TargetRef) -> bool {
        let num = |name| self.num(PlayKind::Intercept, name);
        let Some(carry) = self.intercept_carry(target) else {
            return false;
        };
        let (Some(him), Some(home)) = (
            self.intercept_him(target, &carry),
            self.intercept_home(&carry),
        ) else {
            return false;
        };
        if self.tactics.fine_goal(him, self.now, self.bot.side) {
            return false;
        }
        self.me.feet.distance_xz(him) < num("close_m")
            || self.intercept_meet(him, home, num("lead_min_s")).is_none()
    }

    /// Whether this bot is among the `n` bots of its team nearest `at`.
    fn intercept_among_nearest(&self, at: Vec3, n: usize) -> bool {
        let mine = self.me.feet.distance_xz(at);
        self.view
            .team(self.me.team)
            .filter(|p| p.bot && p.place != self.me.place && p.feet.distance_xz(at) < mine)
            .count()
            < n
    }

    /// The point on his way home the bot reaches `lead_s` before him, the
    /// first from him on.
    fn intercept_meet(&self, him: Vec3, home: Vec3, lead_s: f32) -> Option<Vec3> {
        let num = |name| self.num(PlayKind::Intercept, name);
        let his_side = self.bot.side.and_then(|side| self.enemy_side(side));
        let step = num("meet_step_m").max(0.5);
        // Points along his way: (his metres to it, the point).
        let along = |length: f32, at: &dyn Fn(f32) -> (f32, Vec3)| -> Vec<(f32, Vec3)> {
            let n = (length / step).ceil() as usize;
            (1..=n).map(|i| at((i as f32 * step).min(length))).collect()
        };
        let points = match self.tactics.route(him, home, self.now, his_side) {
            Some(route) => along(route.length(), &|s| (s, route.point_at(s))),
            None => {
                // No route known: the straight way, with a detour factor.
                let length = him.distance_xz(home);
                let factor = num("path_factor");
                along(length, &|s| (s * factor, him + (home - him) * (s / length)))
            }
        };
        let (his_speed, my_speed) = (num("carrier_speed"), num("my_speed"));
        points
            .into_iter()
            .find(|&(his_m, q)| {
                self.path_dist(q) / my_speed + lead_s <= his_m / his_speed
                    && self.tactics.fine_goal(q, self.now, self.bot.side)
            })
            .map(|(_, q)| q)
    }
}

pub fn candidates(cx: &Ctx, out: &mut Vec<Candidate>) {
    if !cx.view.mode.has_batteries() || cx.me.carrying.is_some() {
        return;
    }
    let recent = (cx.num(PlayKind::Intercept, "theft_recent_s") * 1000.0) as u32;
    let alarm_bots = cx.num(PlayKind::Intercept, "alarm_bots") as usize;
    for (battery, state) in cx.view.batteries.iter().enumerate() {
        let BatteryState::Carried(carrier) = *state else {
            continue;
        };
        if carrier.is_some_and(|p| !cx.intercept_is_enemy(p)) {
            continue;
        }
        let theft = cx
            .team
            .thefts
            .iter()
            .filter(|t| t.battery == battery && t.time + recent >= cx.now)
            .max_by_key(|t| t.time);
        // His place if known, else the theft's spot.
        let target = match carrier.filter(|&p| cx.known_place(p).is_some()) {
            Some(p) => TargetRef::Place(p),
            None => match theft {
                Some(theft) => TargetRef::Point(theft.at),
                None => continue,
            },
        };
        let Some(at) = cx.point_of(target) else {
            continue;
        };
        // A chase that fails at its first step (he is in his base, with
        // no meet left on his way) is no candidate: else it is taken and
        // dropped again at once (27 times in the owner's match of
        // 2026-10-16).
        if cx.intercept_fails_at_once(target) {
            continue;
        }
        let key = battery as u32 * 2;
        if theft.is_some() && cx.intercept_among_nearest(at, alarm_bots) {
            for slot in [key, key + 1] {
                if let Some(mut candidate) = cx.candidate(PlayKind::Intercept, target, Some(slot)) {
                    candidate.rank = Rank::Alarm;
                    out.push(candidate);
                }
            }
        } else {
            out.extend(cx.candidate(PlayKind::Intercept, target, Some(key)));
        }
    }
}

/// What every step of a run keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    pub until: u32,
    pub lead_s: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// To the meet, ahead of him on his way.
    Cut { meet: Vec3, planned: u32, run: Run },
    /// At the meet, watching his way.
    Wait { meet: Vec3, planned: u32, run: Run },
    /// After him.
    Chase { planned: u32, run: Run },
}

impl State {
    fn run(&self) -> Run {
        match self {
            Self::Cut { run, .. } | Self::Wait { run, .. } | Self::Chase { run, .. } => *run,
        }
    }
}

pub fn start(_target: TargetRef, cx: &Ctx, rng: &mut Rng) -> State {
    let num = |name| cx.num(PlayKind::Intercept, name);
    let give_up = rng.range(num("give_up_min_s"), num("give_up_max_s"));
    State::Chase {
        // Planned at the first step.
        planned: 0,
        run: Run {
            until: cx.now + (give_up * 1000.0) as u32,
            lead_s: rng.range(num("lead_min_s"), num("lead_max_s")),
        },
    }
}

pub fn step(state: &mut State, target: TargetRef, cx: &Ctx, _rng: &mut Rng) -> Step {
    let num = |name| cx.num(PlayKind::Intercept, name);
    let run = state.run();
    let Some(carry) = cx.intercept_carry(target) else {
        return Step::Done("he no longer carries it");
    };
    if cx.now >= run.until {
        return Step::Failed("he got away");
    }
    let Some(him) = cx.intercept_him(target, &carry) else {
        return Step::Failed("lost him");
    };
    let Some(home) = cx.intercept_home(&carry) else {
        return Step::Failed("no container of his");
    };
    let replan_ms = (num("replan_s") * 1000.0) as u32;
    let me = cx.me.feet;
    let close = me.distance_xz(him) < num("close_m");
    let past = |meet: Vec3| him.distance_xz(home) < meet.distance_xz(home) - num("passed_m");
    // Planned again: a meet, else the chase.
    let replan = |state: &mut State| {
        let last = match *state {
            State::Cut { meet, .. } | State::Wait { meet, .. } => Some(meet),
            State::Chase { .. } => None,
        };
        *state = match cx.intercept_meet(him, home, run.lead_s) {
            Some(meet) if !close => State::Cut {
                // Kept if it hardly moved (the meet drifts as the bot
                // nears it).
                meet: last
                    .filter(|last| last.distance_xz(meet) <= num("keep_m") && !past(*last))
                    .unwrap_or(meet),
                planned: cx.now,
                run,
            },
            _ => State::Chase {
                planned: cx.now,
                run,
            },
        };
    };
    match *state {
        State::Cut { meet, planned, .. } | State::Wait { meet, planned, .. }
            if close || past(meet) || cx.now >= planned + replan_ms =>
        {
            replan(state);
        }
        State::Chase { planned, .. } if cx.now >= planned + replan_ms => replan(state),
        _ => {}
    }
    if let State::Cut { meet, planned, .. } = *state
        && me.distance_xz(meet) <= num("meet_reach_m")
    {
        *state = State::Wait { meet, planned, run };
    }
    match *state {
        State::Cut { meet, .. } => Step::Continue(
            Intent::to(meet, Pace::Run)
                .within(num("meet_reach_m") * 0.7)
                .looking(Look::Path),
        ),
        State::Wait { meet, .. } => Step::Continue(
            Intent::to(meet, Pace::Walk)
                .within(num("meet_reach_m"))
                .crouched()
                .looking(Look::At(him)),
        ),
        State::Chase { .. } => {
            if !cx.tactics.fine_goal(him, cx.now, cx.bot.side) {
                return Step::Failed("he is in his base");
            }
            Step::Continue(Intent::to(him, Pace::Run).within(num("chase_reach_m")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::Scenario;
    use super::*;
    use crate::v026g::bots::know::{CheatKind, Source, Theft};
    use crate::v026g::bots::team::{Asking, assign};
    use crate::v026g::bots::view::Mode;

    const THIEF: Vec3 = Vec3::new(22.0, 0.0, 36.0);
    const THEIR_HOME: Vec3 = Vec3::new(68.0, 0.0, 30.0);

    /// An enemy (place 1) has just taken battery 0 from our container at
    /// (12, 30) and is at (22, 36), known by the thief cheat; the bot at
    /// (34, 0, 31) stands between him and his container.
    fn scene(theft: bool) -> Scenario {
        let mut s = Scenario::field(Mode::Batteries);
        s.bot(0, 0, Vec3::new(34.0, 0.0, 31.0));
        s.enemy(1, 1, THIEF, Some(Source::Cheat(CheatKind::Thief)));
        s.view.places[1].carrying = Some(0);
        s.battery(BatteryState::Carried(Some(1)));
        if theft {
            s.team.thefts.push(Theft {
                battery: 0,
                at: Vec3::new(12.0, 0.0, 30.0),
                time: s.view.clock - 2_000,
                thief: Some(1),
            });
        }
        s
    }

    fn intercepts(s: &Scenario, place: Place) -> Vec<Candidate> {
        let mut out = Vec::new();
        candidates(&s.cx(place), &mut out);
        out
    }

    #[test]
    fn a_theft_is_an_alarm_for_the_two_nearest_bots_and_any_carrier_an_objective() {
        let s = scene(true);
        let found = intercepts(&s, 0);
        assert_eq!(found.len(), 2, "{found:#?}");
        assert!(found.iter().all(|c| c.rank == Rank::Alarm));
        assert!(found.iter().all(|c| c.target == TargetRef::Place(1)));
        assert_ne!(found[0].slot, found[1].slot);
        // No theft: an objective, one slot.
        let found = intercepts(&scene(false), 0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rank, Rank::Objective);
        // Two bots nearer him: no alarm for this one.
        let mut s = scene(true);
        s.bot(2, 0, Vec3::new(25.0, 0.0, 37.0));
        s.bot(3, 0, Vec3::new(26.0, 0.0, 34.0));
        assert!(intercepts(&s, 0).iter().all(|c| c.rank == Rank::Objective));
        assert!(intercepts(&s, 2).iter().all(|c| c.rank == Rank::Alarm));
        // A friend carrying, or a battery lying about: none.
        let mut s = scene(false);
        s.human(1, 0, THIEF);
        assert!(intercepts(&s, 0).is_empty());
        let mut s = scene(false);
        s.view.batteries[0] = BatteryState::Free(THIEF);
        assert!(intercepts(&s, 0).is_empty());
        // Not known and no theft: nothing to go on.
        let mut s = scene(false);
        s.memory = Default::default();
        assert!(intercepts(&s, 0).is_empty());
        // Not known, a theft: the theft's spot.
        let mut s = scene(true);
        s.memory = Default::default();
        let found = intercepts(&s, 0);
        assert!(
            found
                .iter()
                .all(|c| c.target == TargetRef::Point(Vec3::new(12.0, 0.0, 30.0))),
            "{found:#?}"
        );
    }

    /// A carrier in his base (his spawn area, x 75 here) with no meet
    /// left ahead of him: the chase would fail at its first step, so it is
    /// no candidate (it was taken and dropped at once, in bursts); once he
    /// is out, it is one again, and the step chases.
    #[test]
    fn a_carrier_in_his_base_is_no_candidate() {
        let mut s = scene(false);
        s.bot(0, 0, Vec3::new(50.0, 0.0, 31.0));
        s.enemy(1, 1, Vec3::new(72.0, 0.0, 30.0), Some(Source::Seen));
        let cx = s.cx(0);
        assert!(
            !cx.tactics
                .fine_goal(Vec3::new(72.0, 0.0, 30.0), s.view.clock, Some(0))
        );
        let mut rng = Rng::new(1);
        let mut state = start(TargetRef::Place(1), &cx, &mut rng);
        assert_eq!(
            step(&mut state, TargetRef::Place(1), &cx, &mut rng),
            Step::Failed("he is in his base")
        );
        assert!(intercepts(&s, 0).is_empty());
        s.enemy(1, 1, Vec3::new(45.0, 0.0, 31.0), Some(Source::Seen));
        assert_eq!(intercepts(&s, 0).len(), 1);
    }

    #[test]
    fn at_most_two_bots_answer_a_theft() {
        let mut s = scene(true);
        s.bot(2, 0, Vec3::new(30.0, 0.0, 38.0));
        s.bot(3, 0, Vec3::new(40.0, 0.0, 36.0));
        let asking = [0, 2, 3]
            .into_iter()
            .map(|place| Asking {
                place,
                candidates: intercepts(&s, place),
                current: None,
            })
            .collect();
        let out = assign(asking, s.view.clock);
        assert_eq!(out.len(), 2, "{out:?}");
        assert_ne!(out[0].1.slot, out[1].1.slot);
    }

    #[test]
    fn it_meets_him_ahead_on_his_way_and_watches_him_then_gives_up() {
        let mut s = scene(true);
        let steps = s.run(0, PlayKind::Intercept, TargetRef::Place(1), 300, 300, 4.5);
        let Some(Step::Continue(Intent {
            go: Go::To { at: meet, .. },
            ..
        })) = steps.first()
        else {
            panic!("{:?}", steps.first());
        };
        // Between him and his container.
        assert!(meet.distance_xz(THEIR_HOME) < THIEF.distance_xz(THEIR_HOME));
        assert!(steps.iter().any(|step| matches!(
            step,
            Step::Continue(Intent { crouch: true, look: Look::At(at), .. }) if at.distance_xz(THIEF) < 1.0
        )));
        assert_eq!(steps.last(), Some(&Step::Failed("he got away")));
    }

    #[test]
    fn behind_him_it_chases_and_it_ends_when_he_drops_it() {
        let mut s = scene(true);
        s.bot(0, 0, Vec3::new(18.0, 0.0, 39.0));
        let steps = s.run(0, PlayKind::Intercept, TargetRef::Place(1), 3, 300, 4.5);
        assert!(
            matches!(steps[0], Step::Continue(Intent { go: Go::To { at, .. }, .. }) if at == THIEF),
            "{steps:?}"
        );
        let mut rng = Rng::new(1);
        let mut state = start(TargetRef::Place(1), &s.cx(0), &mut rng);
        s.view.batteries[0] = BatteryState::Free(THIEF);
        assert_eq!(
            step(&mut state, TargetRef::Place(1), &s.cx(0), &mut rng),
            Step::Done("he no longer carries it")
        );
    }
}
