//! Play 10: Health: a wounded bot retreats to resupply (`player_bots_behaviour.md` §3): by a covered route to the team resupply zone.
//!
//! NOT BUILT YET: no candidates, so it is never chosen.

use super::*;

pub const NUMBERS: &[(&str, f32)] = &[];

pub fn tuning() -> PlayTuning {
    PlayTuning {
        rank: Rank::Fight,
        considerations: vec![considerations::personality()],
        numbers: Default::default(),
    }
}

pub fn candidates(_cx: &Ctx, _out: &mut Vec<Candidate>) {}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
}

pub fn start(_target: TargetRef, _cx: &Ctx, _rng: &mut Rng) -> State {
    State::Idle
}

pub fn step(_state: &mut State, _target: TargetRef, _cx: &Ctx, _rng: &mut Rng) -> Step {
    Step::Done("not built")
}
