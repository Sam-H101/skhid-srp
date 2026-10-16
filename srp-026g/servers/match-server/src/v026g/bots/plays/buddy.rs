//! Play 25: keep with the partner. Bots go in pairs (play 21's
//! `BotFacts::partner`); the follower of a pair (the higher place) keeps
//! within trading reach of its leader instead of roaming on its own, and the
//! leader waits a little for a follower left behind (`plays::roam`).
//!
//! Why (the players' matches of 6+ players, 2026-10-14, private study): a
//! death with a teammate within 30 m was answered by a kill of the team
//! within seconds 78 % of the time on the winning side, 61 % with no one
//! near (chance alone gave about 55 %); winners died near a teammate more
//! often, and in the battery modes they stood nearer each other (nearest
//! teammate 31 m against 35 m) and were alone (no one within 50 m) less.
//! Being forward, crouching or sprinting did not separate winners from
//! losers once the sides were of equal size. The tick-level recordings say
//! how near: the killer was killed within 5 s after 20 % of the deaths
//! with a teammate within 20 m, after 3 % of those with none.
//!
//! The follower goes to a point `close_m` short of the leader on its own
//! side of him (a second angle, not his back), runs while farther than
//! `keep_m`, walks nearer, and stands looking about once there. Fights,
//! objectives and alarms outrank it (it is in the roam band), so it is the
//! default of a follower and nothing more; when the leader is down the
//! follower roams by itself.

use super::*;
use crate::v026g::bots::curve::{Consideration, Curve, Input};

pub const NUMBERS: &[(&str, f32)] = &[
    // Stand this far from the leader, metres.
    ("close_m", 9.0),
    // Farther than this, run to him, metres.
    ("keep_m", 18.0),
    // The leader waits for a follower farther than this, metres...
    ("wait_m", 25.0),
    // ...but not for one this far (just respawned at home), metres.
    ("wait_max_m", 90.0),
    // The leader waits at most this long a run, seconds.
    ("wait_s", 6.0),
    ("give_up_s", 90.0),
];

pub fn tuning() -> PlayTuning {
    PlayTuning {
        rank: Rank::Roam,
        considerations: vec![
            // Every personality keeps with its partner; support most.
            Consideration::new(
                Input::Personality,
                0.0,
                2.0,
                Curve::Piecewise {
                    points: vec![(0.0, 0.7), (0.5, 0.9), (1.0, 1.0)],
                },
            ),
        ],
        numbers: Default::default(),
    }
}

/// The bot's partner, alive, when the bot follows him (the higher place
/// of the pair follows).
pub fn leader<'a>(cx: &Ctx<'a>) -> Option<&'a PlaceView> {
    let partner = cx.bot.partner?;
    if partner > cx.me.place || !cx.bot.paired {
        return None;
    }
    cx.view
        .place(partner)
        .filter(|p| p.present && p.alive && p.team == cx.me.team)
}

/// The follower to wait for, when the bot leads its pair and the follower
/// is left behind (farther than `wait_m`, nearer than `wait_max_m`).
pub fn lagging<'a>(cx: &Ctx<'a>) -> Option<&'a PlaceView> {
    let partner = cx.bot.partner?;
    if partner < cx.me.place || !cx.bot.paired {
        return None;
    }
    let num = |name| cx.num(PlayKind::Buddy, name);
    cx.view
        .place(partner)
        .filter(|p| p.present && p.alive && p.team == cx.me.team)
        .filter(|p| {
            let d = p.feet.distance_xz(cx.me.feet);
            d > num("wait_m") && d < num("wait_max_m")
        })
}

pub fn candidates(cx: &Ctx, out: &mut Vec<Candidate>) {
    if cx.me.carrying.is_some() {
        return;
    }
    if let Some(leader) = leader(cx) {
        out.extend(cx.candidate(PlayKind::Buddy, TargetRef::Place(leader.place), None));
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Follow,
}

pub fn start(_target: TargetRef, _cx: &Ctx, _rng: &mut Rng) -> State {
    State::Follow
}

pub fn step(_state: &mut State, target: TargetRef, cx: &Ctx, _rng: &mut Rng) -> Step {
    let num = |name| cx.num(PlayKind::Buddy, name);
    if let Some((_, _, started)) = cx.bot.current
        && cx.now > started + (num("give_up_s") * 1000.0) as u32
    {
        return Step::Done("kept with him long enough");
    }
    let TargetRef::Place(place) = target else {
        return Step::Failed("no partner");
    };
    let Some(him) = cx
        .view
        .place(place)
        .filter(|p| p.present && p.alive && p.team == cx.me.team)
    else {
        return Step::Done("the partner is down");
    };
    let d = cx.me.feet.distance_xz(him.feet);
    let close = num("close_m");
    if d <= close * 1.4 {
        return Step::Continue(Intent::stay());
    }
    // Short of him on the bot's own side: beside, not in his back.
    let k = close / d;
    let at = Vec3::new(
        him.feet.x + (cx.me.feet.x - him.feet.x) * k,
        him.feet.y,
        him.feet.z + (cx.me.feet.z - him.feet.z) * k,
    );
    let pace = if d > num("keep_m") {
        cx.pace_toward(at)
    } else {
        Pace::Walk
    };
    Step::Continue(Intent::to(at, pace).within(close * 0.3))
}

#[cfg(test)]
mod tests {
    use super::super::tests::Scenario;
    use super::*;
    use crate::v026g::bots::view::Mode;

    /// Bot 2 follows its partner bot 0, `d` metres ahead on the x axis.
    fn pair(d: f32) -> Scenario {
        let mut s = Scenario::field(Mode::Batteries);
        s.bot(0, 0, Vec3::new(20.0 + d, 0.0, 30.0));
        s.bot(2, 0, Vec3::new(20.0, 0.0, 30.0));
        s.partner = Some(0);
        s
    }

    #[test]
    fn the_follower_keeps_with_its_leader_instead_of_roaming_alone() {
        let s = pair(40.0);
        let best = s.candidates(2);
        // Its default: the roam band holds only this (fights outrank it).
        let roam_band: Vec<_> = best.iter().filter(|c| c.rank == Rank::Roam).collect();
        assert_eq!(roam_band[0].kind, PlayKind::Buddy, "{best:#?}");
        assert!(best.iter().all(|c| c.kind != PlayKind::Roam), "{best:#?}");
        // The leader (lower place) roams; it follows no one.
        let mut s = pair(40.0);
        s.partner = Some(2);
        let lead = s.candidates(0);
        assert!(lead.iter().all(|c| c.kind != PlayKind::Buddy));
        assert!(lead.iter().any(|c| c.kind == PlayKind::Roam));
        // A dead leader: the follower roams by itself.
        let mut s = pair(40.0);
        s.view.places[0].alive = false;
        let alone = s.candidates(2);
        assert!(alone.iter().all(|c| c.kind != PlayKind::Buddy));
        assert!(alone.iter().any(|c| c.kind == PlayKind::Roam));
        // A team's odd bot partners one way, for its trades: it follows
        // no one, and no one waits for it.
        let mut s = pair(40.0);
        s.paired = false;
        let odd = s.candidates(2);
        assert!(odd.iter().all(|c| c.kind != PlayKind::Buddy), "{odd:#?}");
        let mut s = pair(0.0);
        s.partner = Some(2);
        s.view.places[0].feet = Vec3::new(60.0, 0.0, 30.0);
        assert!(lagging(&s.cx(0)).is_some());
        s.paired = false;
        assert!(lagging(&s.cx(0)).is_none());
    }

    #[test]
    fn it_closes_to_beside_him_and_stands_there() {
        let mut s = pair(40.0);
        let steps = s.run(2, PlayKind::Buddy, TargetRef::Place(0), 40, 250, 5.0);
        let d = s.view.places[2].feet.distance_xz(s.view.places[0].feet);
        assert!((5.0..=13.0).contains(&d), "{d}");
        assert_eq!(
            steps
                .last()
                .map(|s| matches!(s, Step::Continue(i) if i.go == Go::Stay)),
            Some(true)
        );
        // The leader down: the play ends.
        s.view.places[0].alive = false;
        let steps = s.run(2, PlayKind::Buddy, TargetRef::Place(0), 2, 250, 5.0);
        assert_eq!(steps[0], Step::Done("the partner is down"));
    }

    #[test]
    fn a_leader_waits_for_a_follower_left_behind_but_not_for_one_at_home() {
        let mut s = pair(0.0);
        s.partner = Some(2);
        s.view.places[0].feet = Vec3::new(60.0, 0.0, 30.0);
        assert!(lagging(&s.cx(0)).is_some(), "40 m behind");
        s.view.places[0].feet = Vec3::new(40.0, 0.0, 30.0);
        assert!(lagging(&s.cx(0)).is_none(), "20 m: near enough");
        s.view.places[0].feet = Vec3::new(20.0, 0.0, 130.0);
        assert!(lagging(&s.cx(0)).is_none(), "100 m: respawned at home");
    }
}
