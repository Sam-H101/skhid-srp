//! Play 21: Trade partner (`player_bots_behaviour.md` §3, §4 "a teammate
//! killed at a spot"): bots go in pairs (`BotFacts::partner`); when one
//! dies, the other gets ~3 s to trade on that spot, then breaks off.
//!
//! The death is the kill feed's (`MatchView::deaths`, fair: the HUD shows
//! it) or the team's danger spot with that victim. The bot goes at the spot
//! looking at the killer's known place (the memory's nearest enemy to the
//! spot), else at the spot; the reflex shoots if a sense gives him. It is
//! an alarm: it beats every other play for its few seconds, except a
//! carrier's (a carrier cannot shoot).

use super::*;
use crate::v026g::bots::curve::{Consideration, Curve, Input};

pub const NUMBERS: &[(&str, f32)] = &[
    // A death this recent is traded, seconds.
    ("fresh_s", 3.0),
    // The trade lasts this long, seconds.
    ("trade_min_s", 2.5),
    ("trade_max_s", 3.5),
    // Past this from the spot, no trade, metres.
    ("max_m", 50.0),
    // A bot at an objective play (a battery's work) trades only a death
    // this near it: the killer is near it too, metres.
    ("objective_m", 20.0),
    // A known enemy this near the spot is the killer, metres.
    ("killer_m", 30.0),
    // It runs at the spot until this near, then walks, metres.
    ("walk_m", 15.0),
    ("reach_m", 3.0),
];

pub fn tuning() -> PlayTuning {
    PlayTuning {
        rank: Rank::Alarm,
        considerations: vec![
            // Most personalities trade; pushers most.
            Consideration::new(
                Input::Personality,
                0.0,
                2.0,
                Curve::Piecewise {
                    points: vec![(0.0, 0.3), (0.5, 0.8), (1.0, 1.0)],
                },
            ),
            considerations::near(80.0),
            considerations::health_above(0.25),
            considerations::cooldown(5.0),
        ],
        numbers: Default::default(),
    }
}

/// How near the partner's death must be to the bot for it to trade it,
/// metres: `max_m`; at a battery's work (an objective play) `objective_m`,
/// so it keeps at the battery unless the killer is near it too (match
/// 0x5402a730 at 54.7 s: a partner killed 12.5 m off, the bot went on
/// collecting and was killed next).
fn reach(tuning: &Tuning, current: Option<PlayKind>) -> f32 {
    let busy = current.is_some_and(|kind| {
        let rank = tuning
            .plays
            .get(&kind)
            .map_or_else(|| default_tuning(kind).rank, |play| play.rank);
        kind != PlayKind::Trade && rank == Rank::Objective
    });
    tuning.number(PlayKind::Trade, if busy { "objective_m" } else { "max_m" })
}

/// Where and when the partner died lately, as far as the team knows.
fn partner_death(cx: &Ctx) -> Option<(Vec3, u32)> {
    let partner = cx.bot.partner?;
    let fresh = (cx.num(PlayKind::Trade, "fresh_s") * 1000.0) as u32;
    let feed = cx
        .view
        .deaths
        .iter()
        .filter(|d| d.place == partner)
        .map(|d| (d.at, d.time));
    let danger = cx
        .team
        .danger
        .iter()
        .filter(|d| d.victim == partner)
        .map(|d| (d.at, d.time));
    feed.chain(danger)
        .filter(|(_, time)| *time <= cx.now && cx.now - *time <= fresh)
        .max_by_key(|(_, time)| *time)
}

pub fn candidates(cx: &Ctx, out: &mut Vec<Candidate>) {
    if cx.me.carrying.is_some() {
        return;
    }
    let reach = reach(cx.tuning, cx.bot.current.map(|(kind, _, _)| kind));
    if let Some((at, _)) = partner_death(cx)
        && at.distance_xz(cx.me.feet) <= reach
    {
        out.extend(cx.candidate(PlayKind::Trade, TargetRef::Point(at), None));
        return;
    }
    // The running trade stays a candidate until its step ends it.
    if let Some((PlayKind::Trade, target, _)) = cx.bot.current {
        out.extend(cx.candidate(PlayKind::Trade, target, None));
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Trade { until: u32 },
}

pub fn start(_target: TargetRef, cx: &Ctx, rng: &mut Rng) -> State {
    let s = rng.range(
        cx.num(PlayKind::Trade, "trade_min_s"),
        cx.num(PlayKind::Trade, "trade_max_s"),
    );
    State::Trade {
        until: cx.now + (s * 1000.0) as u32,
    }
}

pub fn step(state: &mut State, target: TargetRef, cx: &Ctx, _rng: &mut Rng) -> Step {
    let num = |name| cx.num(PlayKind::Trade, name);
    let State::Trade { until } = state;
    if cx.now >= *until {
        return Step::Done("traded or broke off");
    }
    let Some(spot) = cx.point_of(target) else {
        return Step::Failed("no spot to trade on");
    };
    // The killer: the known enemy nearest the spot.
    let killer = cx
        .memory
        .tracks()
        .iter()
        .filter(|t| t.at.distance_xz(spot) <= num("killer_m"))
        .min_by(|a, b| a.at.distance_xz(spot).total_cmp(&b.at.distance_xz(spot)))
        .map(|t| t.at);
    let look = Look::At(killer.unwrap_or(spot));
    let d = cx.me.feet.distance_xz(spot);
    if d <= num("reach_m") {
        return Step::Continue(Intent::stay().looking(look));
    }
    let pace = if d > num("walk_m") {
        Pace::Run
    } else {
        Pace::Walk
    };
    Step::Continue(
        Intent::to(spot, pace)
            .within(num("reach_m") * 0.7)
            .looking(look),
    )
}

#[cfg(test)]
mod tests {
    use super::super::tests::Scenario;
    use super::*;
    use crate::v026g::bots::know::{DangerSpot, Source};
    use crate::v026g::bots::view::{Death, Mode};

    const SPOT: Vec3 = Vec3::new(38.0, 0.0, 33.0);

    /// Bot 0 at (28, 0, 35), its partner 2 died at `SPOT` `ago` ms back;
    /// the killer, enemy 1, heard at (48, 0, 34).
    fn widowed(ago: u32) -> Scenario {
        let mut s = Scenario::field(Mode::Batteries);
        s.bot(0, 0, Vec3::new(28.0, 0.0, 35.0));
        s.bot(2, 0, SPOT).alive = false;
        s.enemy(1, 1, Vec3::new(48.0, 0.0, 34.0), Some(Source::Heard));
        s.partner = Some(2);
        let time = s.view.clock - ago;
        s.view.deaths.push(Death {
            place: 2,
            at: SPOT,
            time,
        });
        s
    }

    fn trades(s: &Scenario) -> Vec<Candidate> {
        let mut out = Vec::new();
        candidates(&s.cx(0), &mut out);
        out
    }

    #[test]
    fn a_fresh_partner_death_is_an_alarm_and_an_old_one_or_a_strangers_is_not() {
        let s = widowed(1_000);
        let out = trades(&s);
        assert_eq!(out.len(), 1, "{out:#?}");
        assert_eq!(out[0].rank, Rank::Alarm);
        assert_eq!(out[0].target, TargetRef::Point(SPOT));
        assert_eq!(s.candidates(0)[0].kind, PlayKind::Trade);
        assert!(trades(&widowed(5_000)).is_empty(), "too old");
        let mut s = widowed(1_000);
        s.partner = Some(3);
        assert!(trades(&s).is_empty(), "not its partner");
        let mut s = widowed(1_000);
        s.view.places[0].carrying = Some(0);
        assert!(trades(&s).is_empty(), "a carrier");
        // The team's danger spot says it as well as the feed.
        let mut s = widowed(1_000);
        s.view.deaths.clear();
        let time = s.view.clock - 500;
        s.team.danger.push(DangerSpot {
            at: SPOT,
            time,
            victim: 2,
        });
        assert_eq!(trades(&s).len(), 1);
    }

    /// At a battery's work a bot trades only a death near it (20 m); at
    /// any other play out to 50 m.
    #[test]
    fn at_a_batterys_work_only_a_near_death_is_traded() {
        let at = |d: f32, current: Option<(PlayKind, TargetRef, u32)>| {
            let mut s = widowed(1_000);
            s.view.places[0].feet = SPOT + Vec3::new(-d, 0.0, 0.0);
            s.current = current;
            trades(&s).len()
        };
        let since = Some(1_000);
        let collect = since.map(|t| (PlayKind::Collect, TargetRef::Battery(0), t));
        let crossfire = since.map(|t| (PlayKind::Crossfire, TargetRef::None, t));
        assert_eq!(at(15.0, collect), 1, "near: traded");
        assert_eq!(at(30.0, collect), 0, "far: it keeps at the battery");
        assert_eq!(at(30.0, crossfire), 1, "no battery's work: traded");
        assert_eq!(at(30.0, None), 1);
        assert_eq!(at(60.0, None), 0, "past max_m");
    }

    #[test]
    fn it_trades_alike_in_every_mode() {
        let score = |mode| {
            let mut s = widowed(1_000);
            s.view.mode = mode;
            trades(&s)[0].score.get()
        };
        let batteries = score(Mode::Batteries);
        assert_eq!(batteries, score(Mode::TeamDeathmatch));
        assert_eq!(batteries, score(Mode::TimedBatteries));
    }

    #[test]
    fn it_goes_at_the_spot_looking_at_the_killer_and_breaks_off_in_about_three_seconds() {
        let mut s = widowed(500);
        let killer = Vec3::new(48.0, 0.0, 34.0);
        let steps = s.run(0, PlayKind::Trade, TargetRef::Point(SPOT), 30, 250, 4.0);
        assert!(
            matches!(&steps[0], Step::Continue(i) if matches!(i.go, Go::To { at, .. } if at == SPOT) && i.look == Look::At(killer)),
            "{steps:?}"
        );
        assert_eq!(steps.last(), Some(&Step::Done("traded or broke off")));
        let ms = (steps.len() - 1) as f32 * 250.0;
        assert!((2_500.0..=3_750.0).contains(&ms), "{ms}");
        assert!(s.view.places[0].feet.distance_xz(SPOT) < 6.0);
    }
}
