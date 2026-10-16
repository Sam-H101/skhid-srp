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
use crate::v026g::bots::view;
use crate::v026g::drive::key;

/// How far over a lying battery's position a bot aims to take it: inside
/// its box whether the position is the box's foot or its middle (the box
/// is about 0.25 m high).
pub const BATTERY_AIM_M: f32 = 0.1;
/// How near the eye comes to what it uses: the client's use is a ray of
/// 2 m from the eye (`base_player::detect_usable_object`, PROVEN), so a
/// bot that stopped farther (the walk's arrival is loose, the spot to stand
/// can be off the battery) steps in closer while it presses.
pub const USE_NEAR_M: f32 = 1.6;
/// The steepest look down a take is tried at, radians: the view's pitch
/// stops at 1 rad (`kin`, PROVEN), so from a crouch (the eye 0.95 m up)
/// over a battery on the floor no ray passes within the use's 0.2 m of
/// its middle closer than about 0.3 m on the ground (`bots::tests`): that
/// is a look down of about 1.2 rad. Past this the bot steps back first.
pub const TAKE_PITCH_MAX_RAD: f32 = 1.15;
/// The look down a take stands for, radians: the eye about 1.2 m from a
/// battery on the floor, the use's ray 1.5 m long. A step back goes
/// there; the walk's arrival (the motor's `arrive_m`, 0.6 m) leaves it
/// between about 0.5 and 1.2 m off, every one of them good.
pub const TAKE_PITCH_RAD: f32 = 0.6;

/// How far off a battery `below` metres under the eye the look down at
/// it is `pitch`, on the ground.
fn take_flat_m(below: f32, pitch: f32) -> f32 {
    below.max(0.0) / pitch.tan()
}

/// Where a bot right over a battery (its middle `aim`, `below` metres
/// under the eye) steps back to so it can look at it:
/// [`TAKE_PITCH_RAD`]'s distance off it, the way it came from (else
/// behind its back), else round that way by eighths of a turn; a floor
/// walked to in a straight line.
fn take_back_spot(cx: &Ctx, aim: Vec3, below: f32) -> Option<Vec3> {
    let me = cx.me.feet;
    let flat = take_flat_m(below, TAKE_PITCH_RAD);
    let (dx, dz) = (me.x - aim.x, me.z - aim.z);
    let away = if dx.hypot(dz) > 0.05 {
        dz.atan2(dx)
    } else {
        let [fx, fz] = crate::v026g::bots::kin::forward(cx.me.yaw);
        (-fz).atan2(-fx)
    };
    [0, 1, -1, 2, -2, 3, -3, 4].into_iter().find_map(|k| {
        let a = away + k as f32 * std::f32::consts::FRAC_PI_4;
        let spot = Vec3::new(aim.x + flat * a.cos(), me.y, aim.z + flat * a.sin());
        cx.tactics
            .clear_line_for(me, spot, cx.now, cx.bot.side, false)
            .then_some(spot)
    })
}

pub const NUMBERS: &[(&str, f32)] = &[
    ("reach_m", 1.5),
    ("use_every_s", 1.2),
    ("tries", 4.0),
    ("give_up_s", 120.0),
    // A spawner's spot weighs this times the chance a missing battery lies
    // in sight of it (missing batteries x spots worth a look within
    // `cluster_m` of it / all spots worth a look), at least `search_min`.
    ("search_weight", 0.9),
    ("search_min", 0.25),
    ("cluster_m", 30.0),
    // A spot within this of one a teammate is going to look at is his.
    ("spread_m", 20.0),
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

impl Ctx<'_> {
    /// Whether a battery lying at `at` may be taken: a floor to stand on
    /// for it (one on a shelf is taken from the floor beside it) that a way
    /// may lead to ([`Ctx::reachable`]), and no search of this bot or its
    /// team proved no way to it lately ([`Ctx::no_way_lately`]). A battery
    /// no way leads to now (inside a hazard) is left for later. Ambush
    /// asks it too: a battery nobody can take is not waited at.
    pub fn can_take(&self, at: Vec3) -> bool {
        self.tactics
            .stand_to_take(at)
            .is_some_and(|stand| self.reachable(stand, false) && !self.no_way_lately(stand))
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
    for battery in 0..cx.view.batteries.len() {
        if cx.battery_lying(battery).is_some_and(|at| cx.can_take(at)) {
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
    // spots, by no more bots than are missing, apart from each other, the
    // spots with more spots in sight of them first (one look checks them
    // all, as a human looks round a yard).
    let missing = cx.batteries_unaccounted();
    let searching = matches!(
        cx.bot.current,
        Some((PlayKind::Collect, TargetRef::Spawner(_), _))
    );
    let others: Vec<usize> = cx
        .team
        .searching
        .iter()
        .filter(|(place, _)| *place != cx.me.place)
        .map(|(_, index)| *index)
        .collect();
    if missing == 0 || (!searching && others.len() >= missing) {
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
    let (cluster, spread) = (num("cluster_m"), num("spread_m"));
    for &i in &worth {
        let item = spawners[i].item;
        // Its own spot it keeps (else bots near each other trade spots
        // every tick); a new one is not one near a teammate's.
        let own = matches!(cx.bot.current, Some((_, TargetRef::Spawner(j), _)) if j == i);
        let taken = others.iter().any(|&j| {
            spawners
                .get(j)
                .is_some_and(|s| s.item.distance_xz(item) < spread)
        });
        if (taken && !own) || !cx.can_take(item) {
            continue;
        }
        let near = worth
            .iter()
            .filter(|&&j| spawners[j].item.distance_xz(item) <= cluster)
            .count();
        let chance = (missing as f32 * near as f32 / worth.len() as f32).min(1.0);
        let weight = (num("search_weight") * chance).max(num("search_min"));
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
            // The use's ray from the eye must hit the battery: at it, not a
            // metre over it as at a place (`Look::At`), which from a
            // crouch passes over a battery on the floor.
            let mut intent = Intent::stay().crouched();
            // Stepping back off it: no use pressed (none could take it),
            // though the time counts as a try's (no endless step back).
            let mut stepping_back = false;
            if let Some(at) = at {
                let aim = at + Vec3::new(0.0, BATTERY_AIM_M, 0.0);
                let eye = cx.me.feet + Vec3::new(0.0, view::EYE_CROUCHED, 0.0);
                let below = eye.y - aim.y;
                let stand = cx.tactics.stand_to_take(at);
                if below.atan2(eye.distance_xz(aim)) > TAKE_PITCH_MAX_RAD {
                    // Right over it: the view does not look down that far
                    // (its pitch stops at 1 rad, `kin`), so the use's ray
                    // passes beside it. A step back first.
                    if let Some(back) = take_back_spot(cx, aim, below) {
                        intent = Intent::to(back, Pace::Walk)
                            .within(cx.tuning.feel.arrive_m + 0.1)
                            .crouched();
                        stepping_back = true;
                    }
                } else if (aim - eye).length() > USE_NEAR_M
                    && let Some(stand) = stand
                {
                    // On the floor: stopped short of it, where the look
                    // down is easy; on a shelf: at the floor beside it.
                    let within = if (stand - at).length() < 0.01 {
                        take_flat_m(below, TAKE_PITCH_RAD).max(0.25)
                    } else {
                        0.25
                    };
                    intent = Intent::to(stand, Pace::Walk).within(within).crouched();
                }
                intent = intent.looking(Look::Use(aim));
            }
            if cx.now >= *last_use + every {
                *n += 1;
                *last_use = cx.now;
                if !stepping_back {
                    intent = intent.pressing(key::USE);
                }
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
                // Its use box's middle is a metre over `home`
                // (`Container::use_point`).
                let eye = cx.me.feet + Vec3::new(0.0, view::EYE_STANDING, 0.0);
                if (home + Vec3::new(0.0, 1.0, 0.0) - eye).length() > USE_NEAR_M {
                    intent = Intent::to(home, Pace::Walk).within(0.5).hold_fire();
                }
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
        s.team.searching = vec![(1, 0), (2, 1)];
        assert!(collects(&s).is_empty());
        // One looking at spawner 2: the others are left to this bot.
        s.team.searching = vec![(1, 2)];
        assert_eq!(collects(&s), spawners[..2].to_vec());
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
        s.team.searching.clear();
        let mut out = Vec::new();
        candidates(&s.cx(0), &mut out);
        out.sort_by(|a, b| b.score.get().total_cmp(&a.score.get()));
        let targets: Vec<TargetRef> = out.iter().map(|c| c.target).collect();
        assert_eq!(targets[0], TargetRef::Battery(0), "{out:?}");
        assert!(!targets.contains(&TargetRef::Spawner(1)));
        assert_eq!(s.cx(0).batteries_unaccounted(), 1);
        s.team.searching = vec![(1, 2)];
        assert_eq!(collects(&s), vec![TargetRef::Battery(0)]);
        s.team.searching.clear();
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

    /// A battery in a pocket that toxic ground (on all match) rings: the
    /// floor it lies on is entered from free floors (the pocket's), so the
    /// look at its next floors said yes, while the search says no way; now
    /// the filter says no too, and a fetch of it gives up at once. One in
    /// a ring of a switching zone is still tried (the zone opens).
    #[test]
    fn a_battery_in_a_pocket_ringed_by_toxic_ground_is_left() {
        let ring = |kind_timer: bool| {
            let world = crate::v026g::bots::tactics::tests::field_with(|map| {
                let wall = |x: f32, z: f32, hx: f32, hz: f32| Shape {
                    kind: ShapeKind::Box,
                    center: Vec3::new(x, 1.0, z),
                    rotation: Rotation::IDENTITY,
                    half: Vec3::new(hx, 1.0, hz),
                };
                map.zones.push(Zone {
                    kind: ZoneKind::Damage,
                    name: "toxic".into(),
                    team: -1,
                    position: Vec3::new(20.0, 0.0, 30.0),
                    damage_type: "toxic".into(),
                    damage: None,
                    shapes: vec![
                        wall(20.0, 25.0, 5.5, 0.5),
                        wall(20.0, 35.0, 5.5, 0.5),
                        wall(15.0, 30.0, 0.5, 5.5),
                        wall(25.0, 30.0, 0.5, 5.5),
                    ],
                });
            });
            let mut s = Scenario::field(Mode::Batteries);
            s.tactics = Tactics::new(world, 1.0);
            if kind_timer {
                // As `Hazards::attach` makes a timer group's zone.
                s.tactics.hazards.zones[0].control = crate::v026g::bots::anomalies::Control::Timer;
            }
            s.bot(0, 0, Vec3::new(10.0, 0.0, 10.0));
            s.battery(BatteryState::Free(Vec3::new(20.0, 0.0, 30.0)));
            s
        };
        let s = ring(false);
        let at = Vec3::new(20.0, 0.0, 30.0);
        assert!(
            s.tactics
                .route(Vec3::new(10.0, 0.0, 10.0), at, s.view.clock, Some(0))
                .is_none(),
            "no way in"
        );
        let goal = s.tactics.world.grid.locate(at, 3.0).unwrap();
        assert!(s.tactics.allowed(goal, s.view.clock, Some(0), false));
        assert!(collects(&s).is_empty());
        let mut s = s;
        let target = TargetRef::Battery(0);
        s.current = Some((PlayKind::Collect, target, s.view.clock));
        let mut rng = Rng::new(1);
        let mut state = start(target, &s.cx(0), &mut rng);
        assert_eq!(
            step(&mut state, target, &s.cx(0), &mut rng),
            Step::Failed("no way to it")
        );
        // A switching ring: the filter leaves it to the search.
        assert_eq!(collects(&ring(true)), vec![TargetRef::Battery(0)]);
    }

    /// The real maps (private files; `SRP_MAPDATA_DIR`, in release): for
    /// every battery spawner of the battery modes and a bot at each side's
    /// first spawn, Collect's filter (`Ctx::reachable`) says what the
    /// route search says, every zone on (as before the anomalies' attach).
    /// At 8c1ce49 they disagreed on 11 spawners (22 of 982 pairs, on
    /// level_01, 02, 03 and 07): pockets ringed by toxic ground, where the
    /// filter's look at the goal's next floors said yes and the search no
    /// (battery 4 of the owner's match of 2026-10-16: 183 takes).
    #[test]
    #[ignore = "needs the private map files"]
    fn collects_filter_agrees_with_the_search_on_the_real_maps() {
        let dir = std::env::var_os("SRP_MAPDATA_DIR").expect("set SRP_MAPDATA_DIR");
        let mut wrong = Vec::new();
        let mut pairs = 0;
        for level in 1..=8u8 {
            for mode in [2u8, 3] {
                let path =
                    std::path::Path::new(&dir).join(format!("level_{level:02}_day_{mode}.mapdata"));
                let map = mapdata::MapData::read(&path).unwrap();
                let world = std::sync::Arc::new(mapdata::World::build(
                    map,
                    mapdata::CELL,
                    mapdata::Body::default(),
                ));
                let mut s = Scenario::field(Mode::Batteries);
                s.tactics = Tactics::new(world.clone(), 1.0);
                let started = std::time::Instant::now();
                let spawn = world
                    .grid
                    .locate(world.map.spawns[0].position, 1.5)
                    .unwrap();
                assert!(s.tactics.past_fixed_zones(spawn));
                eprintln!(
                    "level {level} mode {mode}: past the fixed zones in {:.2} ms",
                    started.elapsed().as_secs_f64() * 1e3
                );
                for (i, spawner) in world.map.battery_spawners.iter().enumerate() {
                    let Some(stand) = s.tactics.stand_to_take(spawner.item) else {
                        continue;
                    };
                    for side in [0, 1] {
                        let Some(from) = world
                            .map
                            .spawns
                            .iter()
                            .find(|spawn| spawn.team == side)
                            .map(|spawn| spawn.position)
                        else {
                            continue;
                        };
                        s.side = Some(side);
                        s.bot(0, side as u8, from);
                        let filter = s.cx(0).reachable(stand, false);
                        let search = s
                            .tactics
                            .route(from, stand, s.view.clock, Some(side))
                            .is_some();
                        pairs += 1;
                        if filter != search {
                            wrong.push((level, mode, i, side, filter, search));
                        }
                    }
                }
            }
        }
        eprintln!("{} of {pairs} disagree", wrong.len());
        assert!(wrong.is_empty(), "{wrong:?}");
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
