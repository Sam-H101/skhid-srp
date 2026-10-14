//! Play 15: Hold the exits (`player_bots_behaviour.md` §3): hold a forward choke 30-60 m from the enemy base, never in a spawn area; high ground, 15-30 m sight lines; move on after 1-2 kills or ~40 s; long holds in Team Deathmatch only.
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
