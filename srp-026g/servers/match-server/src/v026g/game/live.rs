//! What a running match says of itself to those outside it
//! (`MatchRegistry::set_live`): how far it is, who is in it and the score.
//! Nothing here changes the match.

use super::*;
use crate::v026g::registry::{Live, LivePhase};

impl Game {
    /// The match as it is at `now`.
    pub(crate) fn live_now(&self, now: Instant) -> Live {
        let phase = if self.result_made {
            LivePhase::Over
        } else {
            match self.rule.state {
                rules::state::IN_PROCESS => LivePhase::Playing,
                rules::state::COUNTDOWN => LivePhase::Countdown,
                _ => LivePhase::Gathering,
            }
        };
        let mut players = [0; 2];
        for (place, peer) in self.formed.places.iter().zip(&self.peers) {
            let here = match peer {
                Some(peer) => peer.stage != Stage::Gone,
                // Not connected yet: still to come until the countdown.
                None => phase == LivePhase::Gathering,
            };
            if here {
                players[usize::from(place.team.min(1))] += 1;
            }
        }
        let clock = self.clock(now);
        let ms = |ms: u32| Duration::from_millis(ms.into());
        Live {
            at: now,
            phase,
            players,
            played: self
                .rule
                .match_started_at()
                .map(|started| ms(clock.saturating_sub(started))),
            time_left: self
                .rule
                .time_limit_at()
                .map(|limit| ms(limit.saturating_sub(clock))),
            standing: self.state.as_ref().and_then(|state| {
                let standing = state.standing?;
                Some((standing, ms(clock.saturating_sub(state.time))))
            }),
        }
    }
}
