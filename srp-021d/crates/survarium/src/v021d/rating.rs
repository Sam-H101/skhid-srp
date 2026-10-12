//! The ranking (`docs/notes/rating.md`): one rating an account,
//! moved by every match it plays against another team.
//!
//! The 0.21d client only shows it: the ranking page (`36 0C` rows of place,
//! name, rating and its last change; `36 0B` the account's own place,
//! rating and rating before) and the results screen's rating box (before
//! and after). The original formula is lost, so srp uses Elo between the
//! team averages: each player of a team moves by the same amount.

/// A new account's rating.
pub const START: u16 = 1000;
/// The most a match moves a rating.
const K: f64 = 32.0;

/// `game_team_id` of a draw.
const DRAW: u8 = 2;

/// How a match moves each player's rating: `players` are (team, rating,
/// finished) by roster index, `winner` the winning team (2 a draw, 3
/// unknown). A player who left loses whatever their team did. A match
/// without two teams, or without a known result, moves nothing.
pub fn changes(players: &[(u8, u16, bool)], winner: u8) -> Vec<i32> {
    let average = |team: u8| {
        let ratings: Vec<f64> = players
            .iter()
            .filter(|player| player.0 == team)
            .map(|player| f64::from(player.1))
            .collect();
        (!ratings.is_empty()).then(|| ratings.iter().sum::<f64>() / ratings.len() as f64)
    };
    let (Some(first), Some(second)) = (average(0), average(1)) else {
        return vec![0; players.len()];
    };
    if winner > DRAW {
        return vec![0; players.len()];
    }
    players
        .iter()
        .map(|&(team, _, finished)| {
            let (own, other) = match team {
                0 => (first, second),
                1 => (second, first),
                _ => return 0,
            };
            let expected = 1.0 / (1.0 + 10f64.powf((other - own) / 400.0));
            let score = if !finished {
                0.0
            } else if winner == DRAW {
                0.5
            } else if winner == team {
                1.0
            } else {
                0.0
            };
            (K * (score - expected)).round() as i32
        })
        .collect()
}

/// `rating` moved by `change`, kept in what the client shows (u16).
pub fn moved(rating: u16, change: i32) -> u16 {
    (i32::from(rating) + change).clamp(0, u16::MAX.into()) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_teams_move_by_half_k() {
        let players = [(0, 1000, true), (1, 1000, true), (1, 1000, true)];
        assert_eq!(changes(&players, 0), [16, -16, -16]);
        assert_eq!(changes(&players, 1), [-16, 16, 16]);
        assert_eq!(changes(&players, 2), [0, 0, 0]);
        assert_eq!(changes(&players, 3), [0, 0, 0]);
    }

    #[test]
    fn the_stronger_team_wins_less() {
        let players = [(0, 1200, true), (1, 1000, true)];
        assert_eq!(changes(&players, 0), [8, -8]);
        assert_eq!(changes(&players, 1), [-24, 24]);
    }

    #[test]
    fn a_leaver_loses_and_one_team_moves_nothing() {
        let players = [(0, 1000, false), (0, 1000, true), (1, 1000, true)];
        assert_eq!(changes(&players, 0), [-16, 16, -16]);
        assert_eq!(changes(&[(0, 1000, true)], 0), [0]);
        assert_eq!(moved(10, -16), 0);
        assert_eq!(moved(1000, 16), 1016);
    }
}
