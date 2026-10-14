//! Play 12: Cover a human (`player_bots_behaviour.md` §3): follow a human teammate at 5-10 m, looking where he is not.
//!
//! NOT BUILT YET: no candidates, so it is never chosen.

use super::*;

pub const NUMBERS: &[(&str, f32)] = &[];

pub fn tuning() -> PlayTuning {
    PlayTuning {
        rank: Rank::Roam,
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
