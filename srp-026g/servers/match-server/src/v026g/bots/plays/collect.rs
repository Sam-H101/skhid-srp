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
    for (battery, state) in cx.view.batteries.iter().enumerate() {
        if matches!(state, BatteryState::Free(_)) && cx.battery_at(battery).is_some() {
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
            if cx.me.feet.distance_xz(at) <= num("reach_m") {
                *state = State::Take {
                    tries: 0,
                    last_use: 0,
                };
                return Step::Continue(Intent::stay().looking(Look::At(at)));
            }
            Step::Continue(Intent::to(at, cx.pace_toward(at)).within(num("reach_m") * 0.6))
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
