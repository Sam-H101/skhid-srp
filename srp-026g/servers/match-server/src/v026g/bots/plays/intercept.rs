//! Play 2: Carrier interception, and the answer to a theft (§4) (`player_bots_behaviour.md` §3): meet an enemy carrier at a choke he reaches 1-2 s after the bot, else chase; the 1-2 nearest bots answer a theft.
//!
//! NOT BUILT YET: no candidates, so it is never chosen.

use super::*;

pub const NUMBERS: &[(&str, f32)] = &[];

pub fn tuning() -> PlayTuning {
    PlayTuning {
        rank: Rank::Objective,
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
