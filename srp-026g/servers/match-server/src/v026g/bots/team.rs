//! A team's commander (`player_bots_research.md` B1, B2; Killzone's
//! squads): every ~0.5 s it hands each bot of the team a play.
//!
//! - Each bot's candidates are sorted by rank band, then score.
//! - **Commitment:** a bot keeps its play unless a candidate of a higher
//!   band comes, or one of the same band beats it by [`MARGIN`] (research
//!   B9: no flapping between near-equal plays).
//! - **Slots:** a candidate with a slot (an ambush on a battery, a battery
//!   to fetch, a guard post) goes to one bot only; bots are served in the
//!   order of their best score, so the best-placed bot gets the slot.
//!   A slot already held stays with its holder while its play is still a
//!   candidate, unless another bot's candidate for it beats the holder's
//!   by [`MARGIN`] (else two bots near one battery swap it every tick,
//!   neither going for it; seen in the soaks).
//! - The commander says why: each assignment carries the winner's score and
//!   the runners-up for the trace.

use super::plays::{Candidate, PlayKind, SlotKey, TargetRef};
use super::view::Place;

/// A candidate of the same band must beat the running play by this.
pub const MARGIN: f32 = 0.12;

/// A bot's play from the commander.
#[derive(Clone, Debug, PartialEq)]
pub struct Assignment {
    pub kind: PlayKind,
    pub target: TargetRef,
    pub slot: Option<SlotKey>,
    pub score: f32,
    /// Since when (match ms).
    pub since: u32,
    pub reason: &'static str,
}

/// One bot's input to the commander.
pub struct Asking<'a> {
    pub place: Place,
    pub candidates: Vec<Candidate>,
    pub current: Option<&'a Assignment>,
}

fn better(a: &Candidate, b: &Candidate) -> std::cmp::Ordering {
    b.rank
        .cmp(&a.rank)
        .then(b.score.get().total_cmp(&a.score.get()))
}

/// The assignments for one commander tick: `(place, assignment, changed)`.
/// A bot with no candidate at all gets none.
pub fn assign(mut asking: Vec<Asking>, now: u32) -> Vec<(Place, Assignment, bool)> {
    for bot in &mut asking {
        bot.candidates.sort_by(better);
    }
    // The best-placed bots first.
    asking.sort_by(|a, b| {
        let best = |x: &Asking| {
            x.candidates
                .first()
                .map_or((super::plays::Rank::Roam, 0.0), |c| (c.rank, c.score.get()))
        };
        let (ra, sa) = best(a);
        let (rb, sb) = best(b);
        rb.cmp(&ra).then(sb.total_cmp(&sa))
    });
    // Slots held by running plays still asked for: (slot, holder, score).
    let held: Vec<(SlotKey, Place, f32)> = asking
        .iter()
        .filter_map(|bot| {
            let current = bot.current?;
            let slot = current.slot?;
            bot.candidates
                .iter()
                .find(|c| {
                    c.slot == Some(slot) && c.kind == current.kind && c.target == current.target
                })
                .map(|c| (slot, bot.place, c.score.get()))
        })
        .collect();
    let mut taken: Vec<SlotKey> = Vec::new();
    let mut out = Vec::new();
    for bot in asking {
        let free = |c: &&Candidate| {
            c.slot.is_none_or(|slot| {
                !taken.contains(&slot)
                    && held.iter().all(|&(s, holder, score)| {
                        s != slot || holder == bot.place || c.score.get() >= score + MARGIN
                    })
            })
        };
        let Some(best) = bot.candidates.iter().find(free) else {
            continue;
        };
        // The running play, if it is still a candidate and its slot free.
        let running = bot.current.and_then(|current| {
            bot.candidates
                .iter()
                .filter(free)
                .find(|c| c.kind == current.kind && c.target == current.target)
                .map(|c| (current, c))
        });
        let (choice, changed, reason) = match running {
            Some((current, c))
                if c.rank >= best.rank && best.score.get() < c.score.get() + MARGIN =>
            {
                (c, false, current.reason)
            }
            Some((_, c)) if c.rank == best.rank => (best, true, "a better play by the margin"),
            _ if bot.current.is_some() => (best, true, "the play ran out or a higher band came"),
            _ => (best, true, "the best play"),
        };
        if let Some(slot) = choice.slot {
            taken.push(slot);
        }
        let since = match (changed, bot.current) {
            (false, Some(current)) => current.since,
            _ => now,
        };
        out.push((
            bot.place,
            Assignment {
                kind: choice.kind,
                target: choice.target,
                slot: choice.slot,
                score: choice.score.get(),
                since,
                reason,
            },
            changed,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v026g::bots::curve::{Explain, Score};
    use crate::v026g::bots::plays::Rank;

    fn c(kind: PlayKind, rank: Rank, score: f32, slot: Option<u32>, target: usize) -> Candidate {
        Candidate {
            kind,
            target: TargetRef::Battery(target),
            rank,
            score: Score::new(score),
            why: Explain::default(),
            slot: slot.map(|key| SlotKey { kind, key }),
        }
    }

    #[test]
    fn a_slot_goes_to_the_best_placed_bot_and_the_other_takes_its_next_play() {
        let asking = vec![
            Asking {
                place: 1,
                candidates: vec![
                    c(PlayKind::Collect, Rank::Objective, 0.5, Some(0), 0),
                    c(PlayKind::Roam, Rank::Roam, 0.9, None, 9),
                ],
                current: None,
            },
            Asking {
                place: 2,
                candidates: vec![c(PlayKind::Collect, Rank::Objective, 0.8, Some(0), 0)],
                current: None,
            },
        ];
        let out = assign(asking, 1_000);
        let of = |place| out.iter().find(|(p, _, _)| *p == place).unwrap().1.kind;
        assert_eq!(of(2), PlayKind::Collect);
        assert_eq!(
            of(1),
            PlayKind::Roam,
            "the band below once the slot is taken"
        );
    }

    #[test]
    fn a_held_slot_stays_with_its_holder_unless_beaten_by_the_margin() {
        let current = Assignment {
            kind: PlayKind::Collect,
            target: TargetRef::Battery(0),
            slot: Some(SlotKey {
                kind: PlayKind::Collect,
                key: 0,
            }),
            score: 0.5,
            since: 100,
            reason: "the best play",
        };
        let run = |other: f32| {
            let asking = vec![
                Asking {
                    place: 1,
                    candidates: vec![
                        c(PlayKind::Collect, Rank::Objective, 0.5, Some(0), 0),
                        c(PlayKind::Roam, Rank::Roam, 0.9, None, 9),
                    ],
                    current: Some(&current),
                },
                Asking {
                    place: 2,
                    candidates: vec![
                        c(PlayKind::Collect, Rank::Objective, other, Some(0), 0),
                        c(PlayKind::Roam, Rank::Roam, 0.9, None, 8),
                    ],
                    current: None,
                },
            ];
            let out = assign(asking, 1_000);
            let of = |place| out.iter().find(|(p, _, _)| *p == place).unwrap().1.kind;
            (of(1), of(2))
        };
        // A little better placed: the holder keeps it.
        assert_eq!(run(0.55), (PlayKind::Collect, PlayKind::Roam));
        // Much better placed: it goes over.
        assert_eq!(run(0.8), (PlayKind::Roam, PlayKind::Collect));
    }

    #[test]
    fn a_running_play_is_kept_unless_beaten_by_the_margin_or_a_higher_band() {
        let current = Assignment {
            kind: PlayKind::Roam,
            target: TargetRef::Battery(1),
            slot: None,
            score: 0.5,
            since: 100,
            reason: "the best play",
        };
        let ask = |candidates| {
            assign(
                vec![Asking {
                    place: 0,
                    candidates,
                    current: Some(&current),
                }],
                2_000,
            )
        };
        // A slightly better roam: kept, with its start time.
        let out = ask(vec![
            c(PlayKind::Roam, Rank::Roam, 0.5, None, 1),
            c(PlayKind::Roam, Rank::Roam, 0.6, None, 2),
        ]);
        assert!(!out[0].2 && out[0].1.since == 100);
        // Much better: changed.
        let out = ask(vec![
            c(PlayKind::Roam, Rank::Roam, 0.3, None, 1),
            c(PlayKind::Roam, Rank::Roam, 0.6, None, 2),
        ]);
        assert!(out[0].2 && out[0].1.target == TargetRef::Battery(2));
        // A higher band, however low its score: changed.
        let out = ask(vec![
            c(PlayKind::Roam, Rank::Roam, 0.9, None, 1),
            c(PlayKind::Collect, Rank::Objective, 0.1, None, 3),
        ]);
        assert_eq!(out[0].1.kind, PlayKind::Collect);
    }
}
