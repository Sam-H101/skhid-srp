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
//!
//! With the battery cheat off (`CheatTuning::batteries`) a bot goes only
//! for batteries its team knows of (`Ctx::battery_lying`), and while some
//! batteries are not accounted for it looks for them at the spawners' spots
//! as a human does (`TargetRef::Spawner`): a spot is checked once a bot of
//! the team sees it (`sense::sense_batteries`), and no more bots look than
//! batteries are missing.

use super::*;
use crate::v026g::bots::curve::{Consideration, Curve, Input};
use crate::v026g::drive::key;

pub const NUMBERS: &[(&str, f32)] = &[
    ("reach_m", 1.5),
    ("use_every_s", 1.2),
    ("tries", 4.0),
    ("give_up_s", 120.0),
    // A spawner's spot weighs this times the chance a missing battery lies
    // there (missing batteries / spots worth a look), at least `search_min`.
    ("search_weight", 0.9),
    ("search_min", 0.25),
];

/// Spawners' slots come after the batteries'.
const SPAWNER_SLOT: u32 = 0x1_0000;

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
    let can_take = |at: Vec3| {
        cx.tactics
            .stand_to_take(at)
            .is_some_and(|stand| cx.reachable(stand, false) && !cx.no_way_lately(stand))
    };
    for battery in 0..cx.view.batteries.len() {
        if cx.battery_lying(battery).is_some_and(can_take) {
            out.extend(cx.candidate(
                PlayKind::Collect,
                TargetRef::Battery(battery),
                Some(battery as u32),
            ));
        }
    }
    if cx.tuning.cheats.batteries {
        return;
    }
    // The batteries the team cannot place are looked for at the spawners'
    // spots, by no more bots than are missing.
    let missing = cx.batteries_unaccounted();
    let searching = matches!(
        cx.bot.current,
        Some((PlayKind::Collect, TargetRef::Spawner(_), _))
    );
    if missing == 0 || (!searching && cx.team.searchers as usize >= missing) {
        return;
    }
    let spawners = &cx.tactics.world.map.battery_spawners;
    let worth: Vec<usize> = (0..spawners.len())
        .filter(|&i| cx.spawner_worth_checking(i))
        .collect();
    if worth.is_empty() {
        return;
    }
    let num = |name| cx.num(PlayKind::Collect, name);
    let chance = (missing as f32 / worth.len() as f32).min(1.0);
    let weight = (num("search_weight") * chance).max(num("search_min"));
    for i in worth {
        if !can_take(spawners[i].item) {
            continue;
        }
        if let Some(mut candidate) = cx.candidate(
            PlayKind::Collect,
            TargetRef::Spawner(i),
            Some(SPAWNER_SLOT + i as u32),
        ) {
            candidate.score = Score::new(candidate.score.get() * weight);
            if !candidate.score.is_zero() {
                out.push(candidate);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Going to look at a spawner's spot.
    Search,
    Fetch,
    Take {
        tries: u8,
        last_use: u32,
    },
    Bring,
    PutIn {
        tries: u8,
        last_use: u32,
    },
}

pub fn start(target: TargetRef, cx: &Ctx, _rng: &mut Rng) -> State {
    if cx.me.carrying.is_some() {
        State::Bring
    } else if matches!(target, TargetRef::Spawner(_)) {
        State::Search
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
        State::Search | State::Fetch | State::Take { .. } if cx.me.carrying.is_some() => {
            *state = State::Bring;
            step(state, target, cx, _rng)
        }
        State::Search => {
            let TargetRef::Spawner(index) = target else {
                return Step::Failed("no spawner");
            };
            let Some(at) = cx.point_of(target) else {
                return Step::Failed("no spawner");
            };
            let started = cx.bot.current.map_or(0, |(_, _, since)| since);
            if cx
                .team
                .spawner_seen(index)
                .is_some_and(|seen| seen >= started)
            {
                // Whatever lay there is known now: the commander picks again.
                return Step::Done("looked at the spot");
            }
            if !cx.spawner_worth_checking(index) {
                return Step::Done("the spot needs no look");
            }
            let Some(stand) = cx.tactics.stand_to_take(at) else {
                return Step::Failed("no way to it");
            };
            if !cx.reachable(stand, false) || cx.no_way_lately(stand) {
                return Step::Failed("no way to it");
            }
            Step::Continue(Intent::to(stand, cx.pace_toward(stand)).within(num("reach_m") * 0.6))
        }
        State::Fetch => {
            let Some(at) = cx.point_of(target) else {
                return Step::Failed("the battery is gone");
            };
            if !matches!(target, TargetRef::Battery(b) if cx.battery_lying(b).is_some()) {
                return Step::Failed("someone took it");
            }
            // A hazard switched on round it (the cloud's energy rose).
            let Some(stand) = cx.tactics.stand_to_take(at) else {
                return Step::Failed("no way to it");
            };
            // Or every way in crosses a hazard (a battery ringed by fire):
            // the search said so, and the bot does something else.
            if !cx.reachable(stand, false) || cx.no_way_lately(stand) {
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
            // Seen gone at last (it was taken since the team saw it).
            if !cx.tuning.cheats.batteries
                && !matches!(target, TargetRef::Battery(b) if cx.battery_lying(b).is_some())
            {
                return Step::Failed("the battery is gone");
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
    use crate::v026g::bots::view::{BatteryState, Mode};

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

    /// A battery whose search lately proved no way (every way in crosses a
    /// fire) is left until that is forgotten, and a fetch of it gives up.
    #[test]
    fn a_battery_no_way_was_found_to_lately_is_left_for_a_while() {
        let mut s = with_toxic(0.3);
        let at = Vec3::new(45.0, 0.0, 32.0);
        s.battery(BatteryState::Free(at));
        assert_eq!(collects(&s), vec![TargetRef::Battery(0)]);
        let target = TargetRef::Battery(0);
        s.current = Some((PlayKind::Collect, target, s.view.clock));
        let mut rng = Rng::new(1);
        let mut state = start(target, &s.cx(0), &mut rng);
        s.no_way_to = vec![(at + Vec3::new(1.0, 0.0, 0.0), s.view.clock + 5_000)];
        assert!(collects(&s).is_empty());
        assert_eq!(
            step(&mut state, target, &s.cx(0), &mut rng),
            Step::Failed("no way to it")
        );
        // Forgotten: fetched again.
        s.view.clock += 5_000;
        assert_eq!(collects(&s), vec![TargetRef::Battery(0)]);
    }

    /// The field with three battery spawners (as the soaks' map), the
    /// battery cheat off, and a bot of team 0 at (10, 0, 10).
    fn fair() -> (Scenario, [Vec3; 3]) {
        use mapdata::BatterySpawner;
        let mut s = Scenario::field(Mode::Batteries);
        s.tuning.cheats.batteries = false;
        let items = [(40.0, 33.0), (25.0, 12.0), (55.0, 12.0)].map(|(x, z)| Vec3::new(x, 0.3, z));
        let world = crate::v026g::bots::tactics::tests::field_with(|map| {
            map.battery_spawners = items
                .iter()
                .map(|&item| BatterySpawner {
                    position: Vec3::new(item.x, 0.0, item.z),
                    item,
                    layer: "default".into(),
                })
                .collect();
        });
        s.tactics = Tactics::new(world, 1.0);
        s.bot(0, 0, Vec3::new(10.0, 0.0, 10.0));
        (s, items)
    }

    /// With the battery cheat off a bot goes only for the batteries its
    /// team knows of; while some are missing it looks at the spawners'
    /// spots (no more bots than batteries missing), a known battery first;
    /// a look ends once a bot of the team has seen the spot, and a fetch
    /// gives up when the battery is seen gone.
    #[test]
    fn without_the_cheat_a_bot_takes_known_batteries_and_looks_for_the_rest() {
        use crate::v026g::bots::know::KnownBattery;
        let (mut s, items) = fair();
        // Two batteries, at spawners 1 and 2; the team knows neither.
        s.battery(BatteryState::Free(items[1]));
        s.battery(BatteryState::Free(items[2]));
        let spawners = (0..3).map(TargetRef::Spawner).collect::<Vec<_>>();
        assert_eq!(collects(&s), spawners);
        // Two bots looking already: no more.
        s.team.searchers = 2;
        assert!(collects(&s).is_empty());
        s.team.searchers = 1;
        assert_eq!(collects(&s), spawners);
        // A bot saw battery 0 at spawner 1: it comes first, spawner 1
        // needs no look, and one battery is missing.
        s.team.set_battery(
            0,
            Some(KnownBattery {
                at: items[1],
                time: s.view.clock,
                hud: false,
            }),
        );
        let mut out = Vec::new();
        candidates(&s.cx(0), &mut out);
        out.sort_by(|a, b| b.score.get().total_cmp(&a.score.get()));
        let targets: Vec<TargetRef> = out.iter().map(|c| c.target).collect();
        assert_eq!(targets[0], TargetRef::Battery(0), "{out:?}");
        assert!(!targets.contains(&TargetRef::Spawner(1)));
        assert_eq!(s.cx(0).batteries_unaccounted(), 1);
        s.team.searchers = 1;
        assert_eq!(collects(&s), vec![TargetRef::Battery(0)]);
        s.team.searchers = 0;
        // Looking at spawner 0: it walks there until the spot is seen.
        let target = TargetRef::Spawner(0);
        s.current = Some((PlayKind::Collect, target, s.view.clock));
        let mut rng = Rng::new(1);
        let mut state = start(target, &s.cx(0), &mut rng);
        assert_eq!(state, State::Search);
        let Step::Continue(intent) = step(&mut state, target, &s.cx(0), &mut rng) else {
            panic!("goes to look");
        };
        assert!(matches!(intent.go, Go::To { at, .. } if at.distance_xz(items[0]) < 2.0));
        s.view.clock += 1_000;
        s.team.saw_spawner(0, s.view.clock);
        assert_eq!(
            step(&mut state, target, &s.cx(0), &mut rng),
            Step::Done("looked at the spot")
        );
        // Fetching battery 0: once its spot is seen empty (an enemy took
        // it), the fetch gives up.
        let target = TargetRef::Battery(0);
        s.current = Some((PlayKind::Collect, target, s.view.clock));
        let mut state = start(target, &s.cx(0), &mut rng);
        assert!(matches!(
            step(&mut state, target, &s.cx(0), &mut rng),
            Step::Continue(_)
        ));
        s.view.batteries[0] = BatteryState::Carried(Some(5));
        assert!(
            matches!(
                step(&mut state, target, &s.cx(0), &mut rng),
                Step::Continue(_)
            ),
            "not seen gone yet: still believed there"
        );
        s.team.set_battery(0, None);
        assert_eq!(
            step(&mut state, target, &s.cx(0), &mut rng),
            Step::Failed("the battery is gone")
        );
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
