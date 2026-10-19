//! Where a late joiner of a match being played goes: the side with fewer
//! players, and of two alike in that the one that counts for less
//! ([`place_weaker`]; the owner's of 2026-10-19, after a 4 v 8). A room's match (`lobby-server`'s `rooms`)
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
/// first by players, on the side with fewer in the match (the owner's of
/// 2026-10-19: a room's match went to 4 v 8, its four strong and leading);
/// of two alike in that, the one that counts for less (players, strengths
/// and the lead); of two alike in that too, side 0. Only that side being
/// full (`capacity`) with it, and the match's `room`, stop it.
pub fn place_weaker(sides: [Worth; 2], joiner: Worth, rule: &LateRule) -> Option<LatePlacement> {
    let worth = |sides: &[Worth; 2], side: usize| {
        rule.player_value * sides[side].players as i64 + sides[side].strength + rule.lead[side]
    };
    let side =
        usize::from((sides[1].players, worth(&sides, 1)) < (sides[0].players, worth(&sides, 0)));
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
    fn a_joiner_goes_to_the_side_with_fewer_players_however_strong() {
        let one = side(1, 300);
        // Four against three, the three far stronger and leading: the three
        // get the joiner (live, 2026-10-19: a match went to 4 v 8 by value).
        let led = LateRule {
            lead: [0, 5000],
            ..rule()
        };
        let placed = place_weaker([side(4, -3000), side(3, 3000)], one, &led).unwrap();
        assert_eq!(placed.assignments, [(0, 1)]);
        assert_eq!(placed.sides, [side(4, -3000), side(4, 3300)]);
        assert_eq!(placed.apart_before, 1200 - 3000 - 3000 - 5000);
        assert_eq!(placed.apart_after, -3000 - 3300 - 5000);
        let placed = place_weaker([side(3, 3000), side(4, -3000)], one, &rule()).unwrap();
        assert_eq!(placed.assignments, [(0, 0)]);
    }

    #[test]
    fn of_sides_alike_in_players_the_weaker_then_side_0() {
        let one = side(1, 0);
        // Three against three, side 1 weaker by value: there.
        let placed = place_weaker([side(3, 1000), side(3, 400)], one, &rule()).unwrap();
        assert_eq!(placed.assignments, [(0, 1)]);
        assert_eq!(placed.sides, [side(3, 1000), side(4, 400)]);
        // The lead counts: side 0 leads by more than side 1 is stronger.
        let led = LateRule {
            lead: [1000, 0],
            ..rule()
        };
        let placed = place_weaker([side(3, 0), side(3, 500)], one, &led).unwrap();
        assert_eq!(placed.assignments, [(0, 1)]);
        // Alike in both: side 0.
        let placed = place_weaker([side(3, 0), side(3, 0)], one, &rule()).unwrap();
        assert_eq!(placed.assignments, [(0, 0)]);
        // An empty match: side 0.
        let placed = place_weaker([side(0, 0), side(0, 0)], one, &rule()).unwrap();
        assert_eq!(placed.sides, [side(1, 0), side(0, 0)]);
    }

    #[test]
    fn a_full_side_or_no_room_takes_nobody() {
        let one = side(1, 0);
        let small = LateRule {
            capacity: 3,
            ..rule()
        };
        // The side with fewer goes to three, its capacity.
        let placed = place_weaker([side(2, 2000), side(3, 0)], one, &small).unwrap();
        assert_eq!(placed.assignments, [(0, 0)]);
        // Both full: nobody.
        assert_eq!(place_weaker([side(3, 0), side(3, 0)], one, &small), None);
        // No roster place left.
        let no_room = LateRule { room: 0, ..rule() };
        assert_eq!(place_weaker([side(3, 0), side(2, 0)], one, &no_room), None);
        // Nobody to place.
        assert_eq!(
            place_weaker([side(3, 0), side(2, 0)], Worth::default(), &rule()),
            None
        );
    }
}
