//! The match server's own bots (`MatchPlayer::driven`, `SRP_SERVER_BOTS`):
//! places with no client. Stage 1 of the player bots
//! (`docs/notes/player_bots.md` §6).
//!
//! A real client simulates every player from the inputs relayed to it,
//! and a relayed input is applied unchecked (`player_bots.md` §1). So the
//! server enters such a place itself (0xE9, as `finish_sync` does for a
//! client) and relays inputs it makes up for it, with nothing to receive:
//! - it enters once a real client simulates and the match has a clock,
//!   [`ROSTER_LEAD_MS`] ahead, so the 0xE9 reaches that client before its
//!   tick, as a second player's does; a client that starts later gets the
//!   entry and the inputs from the history, as it gets anyone's;
//! - its inputs are the script's (`super::super::drive`): one a tick at
//!   most, on the grid, 50 to 100 ms ahead of the match clock, so never
//!   late, moved or dropped; each is relayed and kept in the history as a
//!   client's is (`Game::input`, from the relay on);
//! - it is no witness of the world (`Game::is_bot`): no reports, no
//!   uploads, never counted among the clients that agree;
//! - it never leaves by itself and never times out: a match with only
//!   bots in it is over after [`BOTS_ALONE`] (`check_bots_alone`).

use super::*;
use crate::v026g::drive::{self, Drive};

impl Game {
    /// The scripts of the server's bots, by place.
    pub(super) fn new_drives(formed: &Formed) -> Vec<Option<Drive>> {
        formed
            .places
            .iter()
            .map(|place| place.driven.then(|| Drive::new(drive::stage_0())))
            .collect()
    }

    /// Enter the server's bots once somebody simulates, and relay the
    /// inputs their scripts have due.
    pub(super) fn poll_driven(&mut self, now: Instant) {
        if self.clock_start.is_none()
            || self.finished_at.is_some()
            || self.ending.is_some()
            || self.mode_end_at.is_some()
        {
            return;
        }
        let clock = self.clock(now);
        for index in 0..self.drives.len() {
            if self.drives[index].is_none() {
                continue;
            }
            let entered = self
                .events
                .iter()
                .any(|(_, who, entered)| *who == index && *entered);
            if !entered {
                if self.live() == 0 {
                    continue;
                }
                let at = grid_up(clock + ROSTER_LEAD_MS);
                self.roster_change(None, index, true, at);
                if let Some(drive) = self.drives[index].as_mut() {
                    drive.entered(at);
                }
                log::info!(
                    "Match {:#x}: player {index} (the server's bot) entered at {at} ms",
                    self.id
                );
                continue;
            }
            let due = self.drives[index]
                .as_mut()
                .map(|drive| drive.due(clock))
                .unwrap_or_default();
            for sent in due {
                let what = Relayed::Input(sent.input);
                self.relay_to_others(index, &what, sent.time);
                self.inputs.push((sent.time, index, what));
                if let Some(part) = sent.what {
                    log::debug!(
                        "Match {:#x}: player {index} (the server's bot) at {} ms: {part}",
                        self.id,
                        sent.time
                    );
                }
            }
        }
    }
}
