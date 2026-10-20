//! What players are shown of the matches being played now, on the
//! launcher's page (`web-server`, its "Matches now" tab): which match is
//! listed and which score is told. (The lobby's own list of them, with its
//! JOIN, went with the queue: the owner's of 2026-10-17.)

use std::time::Duration;

/// A score older than this is not shown. The match server takes a score
/// from a world its clients agree on, every few seconds; while they do not
/// agree it has none newer, and the list would name a leader from memory.
pub const SCORE_AGE: Duration = Duration::from_secs(60);

/// How far a match is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Its players load the level; the countdown has not begun.
    Gathering,
    Countdown,
    Playing,
    /// It has its result and is about to go.
    Over,
}

impl Phase {
    /// Whether a match this far is listed: every one but a match that has
    /// its result, which only lingers for its clients to hear the end.
    pub fn is_listed(self) -> bool {
        self != Self::Over
    }

    /// Whether the score of a match this far is shown, `age` being how old
    /// the world is that said it: only of a match in play (before that the
    /// tickets are full and no battery is home, whatever the world says),
    /// and only as fresh as [`SCORE_AGE`].
    pub fn shows_score(self, age: Duration) -> bool {
        self == Self::Playing && age <= SCORE_AGE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_match_is_listed_until_it_has_its_result_and_its_score_told_only_in_play() {
        let fresh = Duration::from_secs(4);
        for (phase, listed, score) in [
            (Phase::Gathering, true, false),
            (Phase::Countdown, true, false),
            (Phase::Playing, true, true),
            (Phase::Over, false, false),
        ] {
            assert_eq!(phase.is_listed(), listed, "{phase:?}");
            assert_eq!(phase.shows_score(fresh), score, "{phase:?}");
        }
        // A score the match server could not renew is not shown.
        assert!(Phase::Playing.shows_score(SCORE_AGE));
        assert!(!Phase::Playing.shows_score(SCORE_AGE + Duration::from_secs(1)));
    }
}
