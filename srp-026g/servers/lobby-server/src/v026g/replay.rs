//! "Watch replay" from the History of srp's changed lobby movie
//! (`docs/notes/replay_watch.md`, `session::v026g::replays`).
//!
//! - The ask: `3D 00 <REPLAY_ASK> <match>` (`victory_screen_request_player_
//!   data` with a player no account has). Not answered at once: the answer
//!   is kept as the asker's latest, for its polls.
//! - The polls: `3B 00 <REPLAY_START> <count>`, every second while the
//!   dialog waits. Each is answered with the asker's own latest answer, one
//!   `91` row `R1;<status>;<match id 8 hex>;<n>` (`lobby::replay_answer`;
//!   `<n>` the size when `ready`, the place in the conversion line while
//!   `preparing`). A `ready` answer has a `9E` just before it:
//!   `video_link(false, "replay <id> <token> <sha256> <size>")`, which the
//!   game keeps hidden and patch revision 14 hands to the updater when the
//!   player presses "Download and watch" (`video_tutorial_link_clicked`).
//!
//! Only the asker's own answer is ever sent (a poll names no account), and
//! the token is never logged whole (its first six hex digits are; the
//! packet log redacts every `9E`, `core`).

use std::time::Instant;

use session::v026g::replays::{Answer, Known, Status};
use survarium::v026g::lobby;

use super::core::{Core, Who};

impl Core<'_> {
    /// `3D 00 <REPLAY_ASK> <match_id>`: the asker wants that match's replay.
    pub(super) fn replay_ask(&mut self, who: &Who, match_id: u32, now: Instant) {
        let Some(replays) = self.replays else {
            log::info!(
                "{} asked for the replay of match {match_id:#x}: replays are off",
                who.name
            );
            return;
        };
        // While the lobby keeps the match's report: whether the asker played
        // it, and its places. A courtesy, not the line that keeps anything
        // private (only replays are ever served).
        let known = self.results.find(match_id).map(|report| Known {
            played: report.player(who.account_id).is_some(),
            places: report.players.len(),
        });
        let revision = self.store.client_revision(who.session_id);
        let status = replays.ask(who.account_id, match_id, revision, known, now);
        log::info!(
            "{} asked for the replay of match {match_id:#x} (game revision {}): {}",
            who.name,
            revision.map_or_else(|| "unknown".to_owned(), |revision| revision.to_string()),
            status.word()
        );
    }

    /// `3B 00 <REPLAY_START> <count>`: the asker's latest answer, with the
    /// link before it when the replay is ready.
    pub(super) fn replay_poll(&mut self, who: &Who, now: Instant) {
        let answer = match self.replays {
            Some(replays) => replays.answer(who.account_id, now),
            None => Answer {
                status: Status::Off,
                match_id: 0,
                size: 0,
                place: 0,
                ready: None,
            },
        };
        if let Some(link) = answer.link() {
            self.send(who.addr, lobby::video_link(false, &link));
        }
        // The size when `ready`, the place in the conversion line while
        // `preparing`.
        let row = lobby::replay_answer(answer.status.word(), answer.match_id, answer.figure());
        self.send_made(who.addr, row);
    }
}
