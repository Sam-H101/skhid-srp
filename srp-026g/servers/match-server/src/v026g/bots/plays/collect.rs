//! Pick up a battery lying about and bring it home: the battery modes'
//! point, and §4's "our carrier killed: the nearest bot goes for the
//! dropped battery" (a dropped battery lies about like any).
//!
//! One bot per battery (its slot). Taking it is a use on the key going
//! down (bit 21, PROVEN instant with a ~0.6 s animation); carrying, the
//! bot cannot shoot (`Intent::hold_fire`) and is slower; at its own
//! container, another use puts it in. The worlds say whether it carries
//! (`PlaceView::carrying`, up to two seconds late), so each use is tried a
//! few times at most.

use super::*;
use crate::v026g::bots::curve::{Consideration, Curve, Input};
use crate::v026g::bots::view::BatteryState;
use crate::v026g::drive::key;

pub const NUMBERS: &[(&str, f32)] = &[
    ("reach_m", 1.5),
    ("use_every_s", 1.2),
    ("tries", 4.0),
    ("give_up_s", 120.0),
];

pub fn tuning() -> PlayTuning {
    PlayTuning {
        rank: Rank::Objective,
        considerations: vec![
            considerations::near(200.0),
            considerations::health_above(0.25),
            considerations::modifier(Input::ObjectiveUrgency, 0.6),
            Consideration::new(Input::EnemiesNearTarget, 0.0, 4.0, Curve::Linear).inverted(),
        ],
        numbers: Default::default(),
    }
}

pub fn candidates(cx: &Ctx, out: &mut Vec<Candidate>) {
    if !cx.view.mode.has_batteries() {
        return;
    }
    // Carrying already: bring it home, at the top of the objective band.
    if let Some(battery) = cx.me.carrying {
        if let Some(mut candidate) = cx.candidate(
            PlayKind::Collect,
            TargetRef::Battery(battery),
            Some(battery as u32),
        ) {
            candidate.rank = Rank::Alarm;
            out.push(candidate);
        }
        return;
    }
    // A battery no way leads to now (inside a hazard) is left for later;
    // one on a shelf is taken from the floor beside it.
    for (battery, state) in cx.view.batteries.iter().enumerate() {
        if matches!(state, BatteryState::Free(_))
            && cx
                .battery_at(battery)
                .and_then(|at| cx.tactics.stand_to_take(at))
                .is_some_and(|stand| cx.reachable(stand, false))
        {
            out.extend(cx.candidate(
                PlayKind::Collect,
                TargetRef::Battery(battery),
                Some(battery as u32),
            ));
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Fetch,
    Take { tries: u8, last_use: u32 },
    Bring,
    PutIn { tries: u8, last_use: u32 },
}

pub fn start(_target: TargetRef, cx: &Ctx, _rng: &mut Rng) -> State {
    if cx.me.carrying.is_some() {
        State::Bring
    } else {
        State::Fetch
    }
}

pub fn step(state: &mut State, target: TargetRef, cx: &Ctx, _rng: &mut Rng) -> Step {
    let num = |name| cx.num(PlayKind::Collect, name);
    let every = (num("use_every_s") * 1000.0) as u32;
    let tries = num("tries") as u8;
    if let Some((_, _, started)) = cx.bot.current
        && cx.now > started + (num("give_up_s") * 1000.0) as u32
    {
        return Step::Failed("took too long");
    }
    let home = cx.view.container_of(cx.me.team).map(|c| c.at);
    match state {
        State::Fetch | State::Take { .. } if cx.me.carrying.is_some() => {
            *state = State::Bring;
            step(state, target, cx, _rng)
        }
        State::Fetch => {
            let Some(at) = cx.point_of(target) else {
                return Step::Failed("the battery is gone");
            };
            if !matches!(
                target,
                TargetRef::Battery(b) if matches!(cx.view.batteries.get(b), Some(BatteryState::Free(_)))
            ) {
                return Step::Failed("someone took it");
            }
            // A hazard switched on round it (the cloud's energy rose).
            let Some(stand) = cx.tactics.stand_to_take(at) else {
                return Step::Failed("no way to it");
            };
            if !cx.reachable(stand, false) {
                return Step::Failed("no way to it");
            }
            if cx.me.feet.distance_xz(stand) <= num("reach_m") {
                *state = State::Take {
                    tries: 0,
                    last_use: 0,
                };
                return Step::Continue(Intent::stay().looking(Look::At(at)));
            }
            Step::Continue(Intent::to(stand, cx.pace_toward(stand)).within(num("reach_m") * 0.6))
        }
        State::Take { tries: n, last_use } => {
            let at = cx.point_of(target);
            if *n >= tries {
                return Step::Failed("could not take it");
            }
            let mut intent = Intent::stay().crouched();
            if let Some(at) = at {
                intent = intent.looking(Look::At(at));
            }
            if cx.now >= *last_use + every {
                *n += 1;
                *last_use = cx.now;
                intent = intent.pressing(key::USE);
            }
            Step::Continue(intent)
        }
        State::Bring => {
            if cx.me.carrying.is_none() {
                return Step::Done("brought home");
            }
            let Some(home) = home else {
                return Step::Failed("no container of ours");
            };
            if cx.me.feet.distance_xz(home) <= num("reach_m") + 0.5 {
                *state = State::PutIn {
                    tries: 0,
                    last_use: 0,
                };
            }
            // A carrier cannot shoot, and does not sprint (LIKELY).
            Step::Continue(
                Intent::to(home, Pace::Walk)
                    .within(num("reach_m"))
                    .hold_fire(),
            )
        }
        State::PutIn { tries: n, last_use } => {
            if cx.me.carrying.is_none() {
                return Step::Done("put it in");
            }
            if *n >= tries {
                return Step::Failed("could not put it in");
            }
            let mut intent = Intent::stay().hold_fire();
            if let Some(home) = home {
                intent = intent.looking(Look::At(home));
            }
            if cx.now >= *last_use + every {
                *n += 1;
                *last_use = cx.now;
                intent = intent.pressing(key::USE);
            }
            Step::Continue(intent)
        }
    }
}

#[cfg(test)]
mod tests {
    use mapdata::{Rotation, Shape, ShapeKind, Vec3, Zone, ZoneKind};

    use super::*;
    use crate::v026g::bots::plays::tests::Scenario;
    use crate::v026g::bots::tactics::Tactics;
    use crate::v026g::bots::view::Mode;

    /// The field with a toxic zone (always on) of radius `r` round
    /// (25, 0, 32).
    fn with_toxic(r: f32) -> Scenario {
        let mut s = Scenario::field(Mode::Batteries);
        let world = crate::v026g::bots::tactics::tests::field_with(|map| {
            map.zones.push(Zone {
                kind: ZoneKind::Damage,
                name: "toxic".into(),
                team: -1,
                position: Vec3::new(25.0, 0.0, 32.0),
                damage_type: "toxic".into(),
                damage: None,
                shapes: vec![Shape {
                    kind: ShapeKind::Sphere,
                    center: Vec3::new(25.0, 1.0, 32.0),
                    rotation: Rotation::IDENTITY,
                    half: Vec3::new(r, r, r),
                }],
            });
        });
        s.tactics = Tactics::new(world, 1.0);
        s.bot(0, 0, Vec3::new(10.0, 0.0, 10.0));
        s
    }

    fn collects(s: &Scenario) -> Vec<TargetRef> {
        let mut out = Vec::new();
        candidates(&s.cx(0), &mut out);
        out.iter()
            .filter(|c| c.kind == PlayKind::Collect)
            .map(|c| c.target)
            .collect()
    }

    /// A battery inside a zone the bots keep out of is no candidate (it
    /// stood still on it before); one beside it is.
    #[test]
    fn a_battery_inside_a_hazard_is_left_and_one_outside_is_taken() {
        let mut s = with_toxic(4.0);
        s.battery(BatteryState::Free(Vec3::new(25.0, 0.0, 32.0)));
        s.battery(BatteryState::Free(Vec3::new(25.0, 0.0, 15.0)));
        assert_eq!(collects(&s), vec![TargetRef::Battery(1)]);
        assert!(
            s.tactics
                .route(
                    Vec3::new(10.0, 0.0, 10.0),
                    Vec3::new(25.0, 0.0, 32.0),
                    s.view.clock,
                    Some(0)
                )
                .is_none(),
            "no route to the one left"
        );
        // A battery on the marked edge of a zone (1 m, the margin 1 m): a
        // floor let through leads to it, so it is taken.
        let mut s = with_toxic(1.0);
        s.battery(BatteryState::Free(Vec3::new(25.0, 0.0, 30.1)));
        let edge = s
            .tactics
            .world
            .grid
            .locate(Vec3::new(25.0, 0.0, 30.1), 3.0)
            .unwrap();
        assert!(!s.tactics.allowed(edge, s.view.clock, Some(0), false));
        assert_eq!(collects(&s), vec![TargetRef::Battery(0)]);
    }

    /// A battery on a shelf 2 m up (past the jump): its own floor is no
    /// walk's, but from the ground beside it a crouched eye is within a
    /// use's reach in a clear line, so it is taken from there; one in the
    /// shelf's middle, out of reach from every side, is left.
    #[test]
    fn a_battery_on_a_shelf_is_taken_from_the_floor_beside_it() {
        use crate::v026g::bots::body::tests::floor;
        use mapdata::{RayTree, WalkGrid, World};

        let mut s = Scenario::field(Mode::Batteries);
        let mut triangles = floor(0.0, 0.0, 80.0, 40.0, 0.0).to_vec();
        triangles.extend(floor(47.0, 13.0, 51.0, 17.0, 2.0));
        let grid = WalkGrid::build(
            &triangles,
            [0.0, 0.0, 80.0, 40.0],
            0.25,
            mapdata::Body::default(),
        );
        let map = s.tactics.world.map.clone();
        let world = std::sync::Arc::new(World::assemble(
            map,
            grid.into(),
            RayTree::build(&triangles).into(),
        ));
        s.tactics = Tactics::new(world, 1.0);
        s.bot(0, 0, Vec3::new(10.0, 0.0, 10.0));
        let edge = Vec3::new(51.0, 2.2, 15.0);
        let middle = Vec3::new(49.0, 2.2, 15.0);
        s.battery(BatteryState::Free(edge));
        s.battery(BatteryState::Free(middle));
        let stand = s.tactics.stand_to_take(edge).unwrap();
        assert!(stand.y.abs() < 0.01 && stand.x > 51.0, "{stand:?}");
        assert_eq!(s.tactics.stand_to_take(middle), None);
        assert_eq!(collects(&s), vec![TargetRef::Battery(0)]);
        // The fetch walks to the floor beside it, then takes it there.
        let target = TargetRef::Battery(0);
        s.current = Some((PlayKind::Collect, target, s.view.clock));
        let mut rng = Rng::new(1);
        let mut state = start(target, &s.cx(0), &mut rng);
        let Step::Continue(intent) = step(&mut state, target, &s.cx(0), &mut rng) else {
            panic!("goes for it");
        };
        assert!(matches!(intent.go, Go::To { at, .. } if at == stand));
        s.bot(0, 0, stand);
        step(&mut state, target, &s.cx(0), &mut rng);
        assert!(matches!(state, State::Take { .. }));
    }

    /// Fetching a battery a hazard closes round: the play gives up at once
    /// instead of standing till `give_up_s`.
    #[test]
    fn a_fetch_gives_up_when_no_way_leads_to_the_battery() {
        let mut s = with_toxic(4.0);
        s.battery(BatteryState::Free(Vec3::new(25.0, 0.0, 32.0)));
        let target = TargetRef::Battery(0);
        s.current = Some((PlayKind::Collect, target, s.view.clock));
        let mut rng = Rng::new(1);
        let mut state = start(target, &s.cx(0), &mut rng);
        assert_eq!(
            step(&mut state, target, &s.cx(0), &mut rng),
            Step::Failed("no way to it")
        );
    }
}
