//! Where a late joiner of a match being played goes: the side that counts
//! for less ([`place_weaker`]). A room's match (`lobby-server`'s `rooms`)
//! takes a member who comes while it is played so, until [`LOAD_MARGIN`]
//! is left of it (the owner's of 2026-10-17: the queue's late joins, their
//! scoring across matches and their patience went with the queue).

use std::time::Duration;

/// A match takes late joiners until this is left of it (the owner's of
/// 2026-10-19); one who comes in its last seconds waits in the room for the
/// next. A joiner still loading when the match ends is safe: its loading
/// screen has set the map name the results screen reads (`results.md` §6,
/// hazard 10, is a game that never loaded a match).
pub const LOAD_MARGIN: Duration = Duration::from_secs(30);

/// A side of a match being played, or a joiner: its players and what they
/// count for together beside being there (their strengths, in
/// `crate::matchmaking::STRENGTH_SCALE`s).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Worth {
    pub players: usize,
    pub strength: i64,
}

/// How a player is put into a match being played ([`place_weaker`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LateRule {
    /// The most players of a side.
    pub capacity: usize,
    /// The roster places the match has left.
    pub room: usize,
    /// What a player counts for by being there.
    pub player_value: i64,
    /// What each side's lead counts for; nothing for the side that trails.
    pub lead: [i64; 2],
}

/// Who goes into a match being played, and on which side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LatePlacement {
    /// Joiner index and team.
    pub assignments: Vec<(usize, u8)>,
    /// The sides with them.
    pub sides: [Worth; 2],
    /// What the sides count for apart, side 0 less side 1, before and with
    /// them: players, strengths and the lead together.
    pub apart_before: i64,
    pub apart_after: i64,
}

/// Where a match being played, whose sides are `sides` now, takes `joiner`:
/// on the side that counts for less (players, strengths and the lead); of
/// two that count alike the one with fewer players, and of two alike in
/// that too side 0. Only that side being full (`capacity`) with it, and the
/// match's `room`, stop it.
pub fn place_weaker(sides: [Worth; 2], joiner: Worth, rule: &LateRule) -> Option<LatePlacement> {
    let worth = |sides: &[Worth; 2], side: usize| {
        rule.player_value * sides[side].players as i64 + sides[side].strength + rule.lead[side]
    };
    let side =
        usize::from((worth(&sides, 1), sides[1].players) < (worth(&sides, 0), sides[0].players));
    let mut after = sides;
    after[side].players += joiner.players;
    after[side].strength += joiner.strength;
    if joiner.players == 0 || joiner.players > rule.room || after[side].players > rule.capacity {
        return None;
    }
    Some(LatePlacement {
        assignments: vec![(0, side as u8)],
        sides: after,
        apart_before: worth(&sides, 0) - worth(&sides, 1),
        apart_after: worth(&after, 0) - worth(&after, 1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(players: usize, strength: i64) -> Worth {
        Worth { players, strength }
    }

    fn rule() -> LateRule {
        LateRule {
            capacity: 10,
            room: 20,
            player_value: 1200,
            lead: [0, 0],
        }
    }

    #[test]
    fn a_joiner_goes_to_the_weaker_side_even_if_uneven() {
        let one = side(1, 300);
        // Three against three, side 1 weaker by value: there.
        let placed = place_weaker([side(3, 1000), side(3, 400)], one, &rule()).unwrap();
        assert_eq!(placed.assignments, [(0, 1)]);
        assert_eq!(placed.sides, [side(3, 1000), side(4, 700)]);
        assert_eq!(placed.apart_before, 600);
        assert_eq!(placed.apart_after, 600 - 1200 - 300);
        // A side with more players that counts for less still is the
        // weaker: four against three, and the four go to five.
        let placed = place_weaker([side(4, -3000), side(3, 0)], one, &rule()).unwrap();
        assert_eq!(placed.assignments, [(0, 0)]);
        assert_eq!(placed.sides[0].players, 5);
        // The lead counts: side 0 leads by more than side 1 is stronger.
        let led = LateRule {
            lead: [1000, 0],
            ..rule()
        };
        let placed = place_weaker([side(3, 0), side(3, 500)], one, &led).unwrap();
        assert_eq!(placed.assignments, [(0, 1)]);
    }

    #[test]
    fn ties_go_to_fewer_players_then_side_0() {
        let one = side(1, 0);
        // Alike by value: the side with fewer players (its players count
        // for more each).
        let placed = place_weaker([side(4, -1200), side(3, 0)], one, &rule()).unwrap();
        assert_eq!(placed.apart_before, 0);
        assert_eq!(placed.assignments, [(0, 1)]);
        let placed = place_weaker([side(3, 0), side(4, -1200)], one, &rule()).unwrap();
        assert_eq!(placed.assignments, [(0, 0)]);
        // Alike in both: side 0.
        let placed = place_weaker([side(3, 0), side(3, 0)], one, &rule()).unwrap();
        assert_eq!(placed.assignments, [(0, 0)]);
        // An empty match: side 0.
        let placed = place_weaker([side(0, 0), side(0, 0)], one, &rule()).unwrap();
        assert_eq!(placed.sides, [side(1, 0), side(0, 0)]);
    }

    #[test]
    fn a_full_weaker_side_or_no_room_takes_nobody() {
        let one = side(1, 0);
        let small = LateRule {
            capacity: 3,
            ..rule()
        };
        // The weaker side is full: nobody, though the other has room (the
        // joiner would make the stronger side stronger).
        assert_eq!(place_weaker([side(2, 2000), side(3, 0)], one, &small), None);
        let placed = place_weaker([side(3, 0), side(2, 2000)], one, &small);
        assert_eq!(placed, None);
        let placed = place_weaker([side(2, 0), side(3, 2000)], one, &small).unwrap();
        assert_eq!(placed.assignments, [(0, 0)]);
        // No roster place left.
        let no_room = LateRule { room: 0, ..rule() };
        assert_eq!(place_weaker([side(3, 0), side(2, 0)], one, &no_room), None);
        // Nobody to place.
        assert_eq!(
            place_weaker([side(3, 0), side(2, 0)], Worth::default(), &rule()),
            None
        );
        // No rule of sizes: five against three, and the three are stronger,
        // so the five get six.
        let placed = place_weaker([side(5, 0), side(3, 3000)], one, &rule()).unwrap();
        assert_eq!(placed.sides.map(|side| side.players), [6, 3]);
    }
}
