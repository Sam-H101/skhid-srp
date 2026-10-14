//! Play 18, guard the lead (`player_bots_behaviour.md` §3 play 18; §4
//! "little time left: the winning team defends"): in the battery modes, in
//! the last ~2 minutes, with our side ahead, 1-2 bots hold a spot 12-20 m
//! from our container with a sight line to it, crouched, looking at the
//! container and at the way the enemy comes to it, to stop a late denial
//! (play 16). The only time bots guard their base, and never on top of the
//! container (players within 12 m of their own container fared worst).
//!
//! - Two slots per container, so at most two guards.
//! - The spot: `SpotQuery` round our container with a sight line to it,
//!   off every enemy spawn area, picked at random among the best few and
//!   away from a teammate's. The look turns between the container and its
//!   approach every few seconds (drawn); every 35-60 s (drawn) it moves
//!   to another spot, never the same one (humans learn a spot fast, play
//!   15's rule).
//! - It holds till the end, or until a theft (play 2's alarm answers it),
//!   or the lead is gone.

use super::*;
use crate::v026g::bots::curve::{Consideration, Curve, Input};
use crate::v026g::bots::tactics::SpotQuery;

pub const NUMBERS: &[(&str, f32)] = &[
    ("spot_min_m", 12.0),
    ("spot_max_m", 20.0),
    ("spot_samples", 64.0),
    // Picked at random among this many of the best spots.
    ("spot_pick", 4.0),
    // Spots higher than the container weigh this much more per metre.
    ("prefer_high", 0.3),
    // Not within this of a teammate.
    ("apart_m", 5.0),
    // The approach watched: this far from the container toward theirs.
    ("approach_m", 15.0),
    // The look turns between the container and the approach this often.
    ("glance_min_s", 3.0),
    ("glance_max_s", 7.0),
    // Another spot this often, at least `move_apart_m` from the last.
    ("move_min_s", 35.0),
    ("move_max_s", 60.0),
    ("move_apart_m", 6.0),
    ("reach_m", 1.5),
    // A theft this recent keeps guards off (the alarm goes first).
    ("theft_recent_s", 30.0),
];

pub fn tuning() -> PlayTuning {
    PlayTuning {
        rank: Rank::Objective,
        considerations: vec![
            // Full in the last two minutes, 0 above three.
            Consideration::new(
                Input::TimeLeft,
                0.0,
                180.0,
                Curve::Piecewise {
                    points: vec![(0.0, 1.0), (0.67, 1.0), (0.83, 0.3), (1.0, 0.0)],
                },
            ),
            // Ahead by one or more; never level or behind.
            Consideration::new(Input::ScoreGap, 0.0, 1.0, Curve::Step { at: 1.0 }),
            considerations::near(150.0),
            considerations::personality(),
            considerations::health_above(0.2),
        ],
        numbers: Default::default(),
    }
}

pub fn candidates(cx: &Ctx, out: &mut Vec<Candidate>) {
    if !cx.view.mode.has_batteries() || cx.me.carrying.is_some() || cx.sig.score_gap <= 0.0 {
        return;
    }
    if cx.sig.since_theft_s < cx.num(PlayKind::GuardLead, "theft_recent_s") {
        return;
    }
    let Some(ours) = cx.view.container_of(cx.me.team) else {
        return;
    };
    let key = u32::from(ours.id) * 2;
    for slot in [key, key + 1] {
        out.extend(cx.candidate(
            PlayKind::GuardLead,
            TargetRef::Container(ours.id),
            Some(slot),
        ));
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    NoSpot,
    Going {
        spot: Vec3,
        move_at: u32,
    },
    Holding {
        spot: Vec3,
        move_at: u32,
        at_container: bool,
        glance_at: u32,
    },
}

impl Ctx<'_> {
    /// A guard spot round `container`, not within `move_apart_m` of `last`.
    fn guard_spot(&self, container: Vec3, last: Option<Vec3>, rng: &mut Rng) -> Option<Vec3> {
        let num = |name| self.num(PlayKind::GuardLead, name);
        let query = SpotQuery {
            sight_to: Some(container),
            prefer_high: num("prefer_high"),
            side: self.bot.side,
            samples: num("spot_samples") as usize,
            ..SpotQuery::ring(container, num("spot_min_m"), num("spot_max_m"))
        };
        let pick = num("spot_pick") as usize;
        let spots: Vec<Vec3> = self
            .tactics
            .spots(&query, self.now, pick * 2, rng)
            .into_iter()
            .map(|spot| spot.at)
            .filter(|&at| self.friends_near(at, num("apart_m")) == 0)
            .filter(|&at| last.is_none_or(|last| last.distance_xz(at) >= num("move_apart_m")))
            .take(pick)
            .collect();
        (!spots.is_empty()).then(|| spots[rng.below(spots.len())])
    }

    /// The way the enemy comes to our container: toward theirs.
    fn guard_approach(&self, container: Vec3) -> Vec3 {
        let theirs = self
            .view
            .containers
            .iter()
            .find(|c| c.team != self.me.team)
            .map(|c| c.at);
        let Some(theirs) = theirs else {
            return container;
        };
        let d = container.distance_xz(theirs).max(0.1);
        let m = self.num(PlayKind::GuardLead, "approach_m").min(d);
        container + (theirs - container) * (m / d)
    }
}

fn draw_ms(cx: &Ctx, rng: &mut Rng, lo: &str, hi: &str) -> u32 {
    let num = |name| cx.num(PlayKind::GuardLead, name);
    (rng.range(num(lo), num(hi)) * 1000.0) as u32
}

pub fn start(target: TargetRef, cx: &Ctx, rng: &mut Rng) -> State {
    let Some(container) = cx.point_of(target) else {
        return State::NoSpot;
    };
    match cx.guard_spot(container, None, rng) {
        Some(spot) => State::Going {
            spot,
            move_at: cx.now + draw_ms(cx, rng, "move_min_s", "move_max_s"),
        },
        None => State::NoSpot,
    }
}

pub fn step(state: &mut State, target: TargetRef, cx: &Ctx, rng: &mut Rng) -> Step {
    let num = |name| cx.num(PlayKind::GuardLead, name);
    let Some(container) = cx.point_of(target) else {
        return Step::Failed("no container of ours");
    };
    let started = cx.bot.current.map_or(cx.now, |(_, _, since)| since);
    if cx.team.thefts.iter().any(|t| t.time >= started) {
        return Step::Done("a theft: the alarm answers it");
    }
    if cx.sig.score_gap <= 0.0 {
        return Step::Done("the lead is gone");
    }
    // Time to move on: another spot (none found: this one a while more).
    if let State::Going { spot, move_at } | State::Holding { spot, move_at, .. } = *state
        && cx.now >= move_at
    {
        let next = cx.guard_spot(container, Some(spot), rng);
        let move_at = cx.now + draw_ms(cx, rng, "move_min_s", "move_max_s");
        *state = State::Going {
            spot: next.unwrap_or(spot),
            move_at,
        };
    }
    match *state {
        State::NoSpot => Step::Failed("no spot with a sight line"),
        State::Going { spot, move_at } => {
            if cx.me.feet.distance_xz(spot) <= num("reach_m") {
                *state = State::Holding {
                    spot,
                    move_at,
                    at_container: true,
                    glance_at: cx.now + draw_ms(cx, rng, "glance_min_s", "glance_max_s"),
                };
                return Step::Continue(Intent::stay().crouched().looking(Look::At(container)));
            }
            Step::Continue(Intent::to(spot, cx.pace_toward(spot)).within(num("reach_m") * 0.7))
        }
        State::Holding {
            spot,
            move_at,
            mut at_container,
            mut glance_at,
        } => {
            if cx.now >= glance_at {
                at_container = !at_container;
                glance_at = cx.now + draw_ms(cx, rng, "glance_min_s", "glance_max_s");
                *state = State::Holding {
                    spot,
                    move_at,
                    at_container,
                    glance_at,
                };
            }
            let look = if at_container {
                container
            } else {
                cx.guard_approach(container)
            };
            let intent = if cx.me.feet.distance_xz(spot) > num("reach_m") * 2.0 {
                Intent::to(spot, Pace::Walk).within(num("reach_m") * 0.7)
            } else {
                Intent::stay()
            };
            Step::Continue(intent.crouched().looking(Look::At(look)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::Scenario;
    use super::*;
    use crate::v026g::bots::know::Theft;
    use crate::v026g::bots::team::{Asking, assign};
    use crate::v026g::bots::view::{EYE_CROUCHED, Mode};

    const OURS: Vec3 = Vec3::new(12.0, 0.0, 30.0);

    /// Ahead 3 to 2 with 90 s left; bots 0, 2 and 4 of team 0 out in the
    /// field.
    fn scene() -> Scenario {
        let mut s = Scenario::field(Mode::Batteries);
        s.bot(0, 0, Vec3::new(30.0, 0.0, 35.0));
        s.bot(2, 0, Vec3::new(32.0, 0.0, 30.0));
        s.bot(4, 0, Vec3::new(34.0, 0.0, 36.0));
        s.time_left(90.0).points(3, 2);
        s
    }

    fn guards(s: &Scenario, place: Place) -> Vec<Candidate> {
        let mut out = Vec::new();
        candidates(&s.cx(place), &mut out);
        out
    }

    #[test]
    fn guards_only_late_ahead_and_with_no_fresh_theft() {
        let found = guards(&scene(), 0);
        assert_eq!(found.len(), 2, "{found:#?}");
        assert!(found.iter().all(|c| c.target == TargetRef::Container(0)));
        let mut s = scene();
        s.points(2, 3);
        assert!(guards(&s, 0).is_empty(), "behind");
        s.points(2, 2);
        assert!(guards(&s, 0).is_empty(), "level");
        let mut s = scene();
        s.time_left(400.0);
        assert!(guards(&s, 0).is_empty(), "too early");
        let mut s = scene();
        s.sig.since_theft_s = 5.0;
        assert!(guards(&s, 0).is_empty(), "a theft first");
        let mut s = scene();
        s.view.mode = Mode::TeamDeathmatch;
        assert!(guards(&s, 0).is_empty());
    }

    #[test]
    fn at_most_two_guards() {
        let s = scene();
        let asking = [0, 2, 4]
            .into_iter()
            .map(|place| Asking {
                place,
                candidates: guards(&s, place),
                current: None,
            })
            .collect();
        assert_eq!(assign(asking, s.view.clock).len(), 2);
    }

    #[test]
    fn it_holds_a_spot_in_sight_of_the_container_looks_about_and_moves_on() {
        let mut s = scene();
        let steps = s.run(
            0,
            PlayKind::GuardLead,
            TargetRef::Container(0),
            300,
            300,
            3.5,
        );
        assert!(
            steps.iter().all(|s| matches!(s, Step::Continue(_))),
            "{:?}",
            steps.last()
        );
        // The spots it went to: 12-20 m off, seeing the container.
        let mut spots: Vec<Vec3> = Vec::new();
        for step in &steps {
            if let Step::Continue(Intent {
                go: Go::To { at, .. },
                ..
            }) = step
                && spots.last().is_none_or(|last| last.distance_xz(*at) > 0.1)
            {
                spots.push(*at);
            }
        }
        assert!(spots.len() >= 2, "it moved on: {spots:?}");
        for spot in &spots {
            let d = spot.distance_xz(OURS);
            assert!((11.5..=20.5).contains(&d), "{spot:?}");
            let eye = *spot + Vec3::new(0.0, EYE_CROUCHED, 0.0);
            assert!(s.tactics.sees(eye, OURS + Vec3::new(0.0, 1.0, 0.0)));
        }
        assert!(spots[1].distance_xz(spots[0]) >= 6.0);
        // Crouched, still, looking at the container and at its approach.
        let looks: Vec<Vec3> = steps
            .iter()
            .filter_map(|s| match s {
                Step::Continue(Intent {
                    go: Go::Stay,
                    crouch: true,
                    look: Look::At(at),
                    ..
                }) => Some(*at),
                _ => None,
            })
            .collect();
        assert!(looks.contains(&OURS));
        assert!(looks.iter().any(|at| at.x > OURS.x + 10.0), "{looks:?}");
    }

    #[test]
    fn a_theft_or_a_lost_lead_ends_it() {
        let mut s = scene();
        let mut rng = Rng::new(2);
        s.current = Some((PlayKind::GuardLead, TargetRef::Container(0), s.view.clock));
        let mut state = start(TargetRef::Container(0), &s.cx(0), &mut rng);
        assert!(matches!(state, State::Going { .. }));
        assert!(matches!(
            step(&mut state, TargetRef::Container(0), &s.cx(0), &mut rng),
            Step::Continue(_)
        ));
        s.team.thefts.push(Theft {
            battery: 0,
            at: OURS,
            time: s.view.clock + 1,
            thief: None,
        });
        s.view.clock += 300;
        assert_eq!(
            step(&mut state, TargetRef::Container(0), &s.cx(0), &mut rng),
            Step::Done("a theft: the alarm answers it")
        );
        s.team.thefts.clear();
        s.points(3, 3);
        assert_eq!(
            step(&mut state, TargetRef::Container(0), &s.cx(0), &mut rng),
            Step::Done("the lead is gone")
        );
    }
}
