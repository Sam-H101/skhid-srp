//! What a player counts for when the sides of a match are made, and the
//! balance rating that is a part of it (`docs/notes/matchmaking.md` §8).
//!
//! All of it is srp's own [choice]: the client knows nothing of how the
//! original server made its sides. The numbers are fitted to the players'
//! server's matches of 2026-10-12 and 13 (88 with a winner), which is
//! little: they are settings ([`Weights`]) to be fitted again.
//!
//! - **A side counts** [`Weights::player`] for each of its players, and
//!   each player's [`Weights::strength`]: its gear level, its own level and
//!   its balance rating as far as that is trusted. The queue makes the two
//!   sides count alike (`crate::matchmaking`'s `seat_balanced`).
//! - **The balance rating** ([`Balance`]) is not the ranking's
//!   (`Account::rating`), which the client shows and which every player of
//!   a side moves by alike whatever the sides' sizes. This one is kept for
//!   the sides only. It starts at 0 (the average), a win raises it and a
//!   loss lowers it ([`changes`]), by more the less the result was to be
//!   expected: the side with a player more is expected to win
//!   ([`PLAYER`]), so winning with it moves little.
//! - **It is trusted by its matches** ([`Weights::trust`]): a rating of no
//!   match counts for nothing, of [`Weights::trust_matches`] for half.

use survarium::v026g::match_msgs::rules::DRAW;

use crate::matchmaking::STRENGTH_SCALE;

/// A whole point of the balance rating, as it is kept: thousandths.
pub const UNIT: i32 = 1000;

/// What one player more is worth to a side, in rating points: the odds of
/// the side with a player more were about nine to one (it won 35 of 38
/// matches; the fit's 2.2 to 2.3).
pub const PLAYER: f64 = 2.2;

/// How far a match moves the ratings of a side, all its players together,
/// when the result was a toss-up and went one way: half of this. Shared
/// among the players of the match, so a match of seven a side does not
/// move its sides seven times as far as a duel does (a step for each
/// player called a seven against seven at 99 to 1, wrongly).
pub const STEP: f64 = 0.8;

/// A new player's rating moves this many times further, on top of
/// [`STEP`], and less with each match: `STEP * (1 + NEW_PLAYER / (1 +
/// matches))`. Its first match moves it four times as far as its
/// hundredth.
pub const NEW_PLAYER: f64 = 3.0;

/// An account's balance rating: thousandths of a point ([`UNIT`]), 0 the
/// average, and the matches that moved it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Balance {
    pub rating: i32,
    pub matches: u32,
}

impl Balance {
    /// The rating moved by `change`, with one match more.
    pub fn moved(self, change: i32) -> Self {
        Self {
            rating: self.rating.saturating_add(change),
            matches: self.matches.saturating_add(1),
        }
    }
}

/// How a match moves each balance rating, in thousandths: `players` are
/// (team, rating) of those who were in the match, `winner` the winning
/// team (2 a draw). A match with one side, or with no known winner, moves
/// nothing; a player of no team is not moved.
pub fn changes(players: &[(u8, Balance)], winner: u8) -> Vec<i32> {
    let count = |team: u8| players.iter().filter(|player| player.0 == team).count();
    let (first, second) = (count(0), count(1));
    if first == 0 || second == 0 || winner > DRAW {
        return vec![0; players.len()];
    }
    let side = |team: u8| if team == 0 { 1.0 } else { -1.0 };
    let apart: f64 = players
        .iter()
        .filter(|player| player.0 < 2)
        .map(|&(team, balance)| side(team) * f64::from(balance.rating) / f64::from(UNIT))
        .sum::<f64>()
        + PLAYER * (first as f64 - second as f64);
    let expected = 1.0 / (1.0 + (-apart).exp());
    let score = match winner {
        0 => 1.0,
        1 => 0.0,
        _ => 0.5,
    };
    let shared = 2.0 / (first + second) as f64;
    players
        .iter()
        .map(|&(team, balance)| {
            if team > 1 {
                return 0;
            }
            let step = STEP * (1.0 + NEW_PLAYER / (1.0 + f64::from(balance.matches))) * shared;
            (step * (score - expected) * side(team) * f64::from(UNIT)).round() as i32
        })
        .collect()
}

/// What the parts of a player count for when the sides are made, each in
/// gear levels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weights {
    /// A player, by being there. With an odd count the side with a player
    /// more has this much more, so the stronger players go to the other.
    /// 0: only the strengths are made even, as before 2026-10-13.
    pub player: f64,
    /// A level of the account (`Account::level`: its skill points).
    pub player_level: f64,
    /// A whole point of the balance rating, fully trusted.
    pub rating: f64,
    /// The matches after which a rating counts for half. 0: it counts in
    /// full from its first match.
    pub trust_matches: f64,
}

impl Default for Weights {
    /// The fit of 2026-10-13 (player count, gear level, player level and
    /// the rating, trusted, against who won): a player 2.28, a gear level
    /// 0.15, a player level 0.18, a rating point 0.58. In gear levels that
    /// is 15, 1.2 and 3.9; without the rating the same fit said 12 and
    /// 1.5, and the splits were the same from 8 to 12.
    fn default() -> Self {
        Self {
            player: 12.0,
            player_level: 1.5,
            rating: 4.0,
            trust_matches: 5.0,
        }
    }
}

impl Weights {
    /// [`Self::player`] as the queue takes it (`Matchmaking::set_player_value`).
    pub fn player_value(&self) -> i32 {
        scaled(self.player)
    }

    /// How far a rating of `matches` matches is trusted, 0 to 1.
    pub fn trust(&self, matches: u32) -> f64 {
        let matches = f64::from(matches);
        if matches <= 0.0 {
            0.0
        } else {
            matches / (matches + self.trust_matches.max(0.0))
        }
    }

    /// What a player counts for on top of [`Self::player`], as the queue
    /// takes it (`Matchmaking::set_strengths`): its gear level
    /// (`matchmaking::level`'s `min`), its own level and its rating.
    pub fn strength(&self, gear_level: u8, player_level: u8, balance: Balance) -> i32 {
        let rating = f64::from(balance.rating) / f64::from(UNIT);
        scaled(
            f64::from(gear_level)
                + self.player_level * f64::from(player_level)
                + self.rating * self.trust(balance.matches) * rating,
        )
    }
}

/// Gear levels as the queue's whole numbers.
fn scaled(levels: f64) -> i32 {
    (levels * f64::from(STRENGTH_SCALE))
        .round()
        .clamp(f64::from(i32::MIN / 64), f64::from(i32::MAX / 64)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new() -> Balance {
        Balance::default()
    }

    fn rated(rating: i32, matches: u32) -> Balance {
        Balance { rating, matches }
    }

    #[test]
    fn equal_sides_move_by_as_much_each_way() {
        // Two new players: a toss-up, the step shared between two players
        // and four times as far for a first match.
        let moved = changes(&[(0, new()), (1, new())], 0);
        assert_eq!(moved, [1600, -1600]);
        assert_eq!(changes(&[(0, new()), (1, new())], 1), [-1600, 1600]);
        assert_eq!(changes(&[(0, new()), (1, new())], DRAW), [0, 0]);
    }

    #[test]
    fn a_match_moves_its_sides_as_far_whatever_their_size() {
        let duel: i32 = changes(&[(0, rated(0, 50)), (1, rated(0, 50))], 0)[0];
        let seven: Vec<(u8, Balance)> = (0..14).map(|i| (i % 2, rated(0, 50))).collect();
        let side: i32 = changes(&seven, 0)
            .iter()
            .zip(&seven)
            .filter(|(_, player)| player.0 == 0)
            .map(|(change, _)| *change)
            .sum();
        assert!((side - duel).abs() <= 7, "{side} against {duel}");
    }

    #[test]
    fn winning_with_a_player_more_moves_little_and_losing_with_it_much() {
        let players = [(0, rated(0, 20)), (0, rated(0, 20)), (1, rated(0, 20))];
        let won = changes(&players, 0);
        let lost = changes(&players, 1);
        assert!(won[0] > 0 && won[2] < 0);
        assert!(lost[0] < 0 && lost[2] > 0);
        assert!(lost[0].abs() > 5 * won[0].abs(), "{won:?} {lost:?}");
    }

    #[test]
    fn a_new_players_rating_moves_further() {
        let moved = changes(
            &[(0, new()), (0, rated(0, 30)), (1, new()), (1, rated(0, 30))],
            0,
        );
        assert!(moved[0] > 3 * moved[1]);
        assert!(moved[2] < 3 * moved[3]);
    }

    #[test]
    fn one_side_or_no_winner_moves_nothing() {
        assert_eq!(changes(&[(0, new()), (0, new())], 0), [0, 0]);
        assert_eq!(changes(&[(0, new()), (1, new())], 3), [0, 0]);
        // A player of no team is not moved and not counted.
        assert_eq!(
            changes(&[(0, new()), (1, new()), (2, new())], 0),
            [1600, -1600, 0]
        );
    }

    #[test]
    fn the_expected_winner_gains_less() {
        let favourite = changes(&[(0, rated(2 * UNIT, 40)), (1, rated(0, 40))], 0);
        let upset = changes(&[(0, rated(2 * UNIT, 40)), (1, rated(0, 40))], 1);
        assert!(favourite[0] > 0 && favourite[0] < upset[1]);
    }

    #[test]
    fn a_rating_is_trusted_by_its_matches() {
        let weights = Weights::default();
        assert_eq!(weights.trust(0), 0.0);
        assert_eq!(weights.trust(5), 0.5);
        assert!(weights.trust(45) >= 0.9);
        // A new account is its gear and its level: the rating counts for
        // nothing, whatever it says.
        assert_eq!(weights.strength(3, 2, rated(5 * UNIT, 0)), 600);
        // After five matches a point counts for half of 4 gear levels.
        assert_eq!(weights.strength(3, 2, rated(UNIT, 5)), 800);
        assert_eq!(weights.strength(3, 2, rated(-UNIT, 5)), 400);
        assert_eq!(weights.player_value(), 1200);
    }

    #[test]
    fn weights_of_zero_leave_the_gear_level() {
        let weights = Weights {
            player: 0.0,
            player_level: 0.0,
            rating: 0.0,
            trust_matches: 5.0,
        };
        assert_eq!(
            weights.strength(7, 9, rated(3 * UNIT, 100)),
            7 * STRENGTH_SCALE
        );
        assert_eq!(weights.player_value(), 0);
        // No trust setting: in full from the first match.
        let at_once = Weights {
            trust_matches: 0.0,
            ..Weights::default()
        };
        assert_eq!(at_once.trust(1), 1.0);
        assert_eq!(at_once.trust(0), 0.0);
    }
}
