//! Lines in the general chat about the search: who joined it, who left it,
//! and with whom a match starts (`SRP_QUEUE_CHAT`, on by default).
//!
//! The game has no text for any of this, so the lines are the server's own:
//! an `A2` chat line of a sender of type 5 (an account) with id 0 and, by
//! default, no name (`SRP_QUEUE_CHAT_NAME`), on the general channel
//! (`resources/notes/v026g/social.md` §1.2). The client shows the body as it
//! is (censored, never localized). A line about the search goes to every
//! client that listens to the general channel; the line of a match, to the
//! players put in it.
//!
//! Nothing is said twice for a player who presses Play and leaves in a loop:
//! a join is announced at most once in [`REPEAT`], and a leave only for a
//! join that was announced.

use std::time::{Duration, Instant};

use survarium::v026g::lobby::{self, ServerMessage, channel, sender};

use session::LockExt;

use super::core::Core;

/// A player's join is announced again no sooner than this.
const REPEAT: Duration = Duration::from_secs(30);
/// The most bytes of a chat line the client reads (`lobby::limits::TEXT`).
const LINE: usize = 255;

impl Core<'_> {
    /// Say `text` in the general chat to everybody who listens to it.
    fn say_to_all(&mut self, text: &str) {
        let lines = fit(text);
        let name = self.settings.queue_chat_name.clone();
        let listeners: Vec<_> = self
            .players()
            .into_iter()
            .filter(|addr| {
                self.player(*addr).is_some_and(|player| {
                    subscribed(player.channel_ids[usize::from(channel::GENERAL)])
                })
            })
            .collect();
        for addr in listeners {
            for line in &lines {
                self.send(addr, line_of(&name, line));
            }
        }
    }

    /// Say `text` in the general chat to these accounts.
    fn say_to(&mut self, accounts: &[u32], text: &str) {
        let lines = fit(text);
        let name = self.settings.queue_chat_name.clone();
        for account_id in accounts {
            let Some(addr) = self.by_account.get(account_id).copied() else {
                continue;
            };
            for line in &lines {
                self.send(addr, line_of(&name, line));
            }
        }
    }

    /// Players pressed Play (`members`: the account ids, one unless it is a
    /// squad; `who` is the one whose press queued them).
    pub(super) fn announce_joined(
        &mut self,
        who: &super::core::Who,
        members: &[u32],
        now: Instant,
    ) {
        if !self.settings.queue_chat {
            return;
        }
        let recent = self.player(who.addr).is_some_and(|player| {
            player
                .queue_line
                .is_some_and(|at| now.saturating_duration_since(at) < REPEAT)
        });
        if recent {
            return;
        }
        let waiting = self.store.matchmaking().queued_players();
        let text = if members.len() > 1 {
            let names: Vec<String> = members
                .iter()
                .filter_map(|id| {
                    self.accounts
                        .get(*id)
                        .map(|account| account.locked().name.clone())
                })
                .collect();
            format!(
                "{} joined the search as a squad ({waiting} waiting)",
                names.join(", ")
            )
        } else {
            format!("{} joined the search ({waiting} waiting)", who.name)
        };
        for account_id in members {
            if let Some(addr) = self.by_account.get(account_id).copied()
                && let Some(player) = self.player_mut(addr)
            {
                player.queue_line = Some(now);
                player.queue_announced = true;
            }
        }
        self.say_to_all(&text);
    }

    /// The queue loses `session_id` (and with it its squad): say so when
    /// the join was said. Used for every way out of the queue.
    pub(super) fn leave_queue_announced(&mut self, session_id: u32, account_id: u32, name: &str) {
        let announced = self
            .by_account
            .get(&account_id)
            .copied()
            .and_then(|addr| self.player_mut(addr))
            .is_some_and(|player| std::mem::take(&mut player.queue_announced));
        self.leave_queue_said(session_id, name, announced);
    }

    /// [`Self::leave_queue_announced`] for a player whose flag is known (one
    /// that is being taken out of its link).
    pub(super) fn leave_queue_said(&mut self, session_id: u32, name: &str, announced: bool) {
        let before = self.store.matchmaking().queued_players();
        self.store.matchmaking().leave_queue(session_id);
        let removed = before.saturating_sub(self.store.matchmaking().queued_players());
        if removed == 0 || !announced || !self.settings.queue_chat {
            return;
        }
        let waiting = self.store.matchmaking().queued_players();
        let text = if removed > 1 {
            format!("{name} left the search with their squad ({waiting} waiting)")
        } else {
            format!("{name} left the search ({waiting} waiting)")
        };
        self.say_to_all(&text);
    }

    /// A match was made: tell its players with whom they play, team 1
    /// against team 2. `teams[0]` and `teams[1]` are the players' names.
    pub(super) fn announce_match(&mut self, accounts: &[u32], teams: [Vec<String>; 2]) {
        for account_id in accounts {
            if let Some(addr) = self.by_account.get(account_id).copied()
                && let Some(player) = self.player_mut(addr)
            {
                player.queue_announced = false;
            }
        }
        if !self.settings.queue_chat {
            return;
        }
        let [first, second] = teams;
        let text = format!("Match found: {} vs {}", first.join(", "), second.join(", "));
        if text.len() <= LINE {
            self.say_to(accounts, &text);
        } else {
            // Too long for one line: one for each team.
            self.say_to(
                accounts,
                &format!("Match found. Team 1: {}", first.join(", ")),
            );
            self.say_to(accounts, &format!("Team 2: {}", second.join(", ")));
        }
    }
}

fn subscribed(id: u32) -> bool {
    id != 0 && id != u32::MAX
}

fn line_of(name: &str, body: &str) -> ServerMessage {
    lobby::chat_text(sender::ACCOUNT, 0, name, "", channel::GENERAL, body)
}

/// `text` as lines of at most [`LINE`] bytes (one, cut with "..." at a
/// character boundary, if it is longer).
fn fit(text: &str) -> Vec<String> {
    if text.len() <= LINE {
        return vec![text.to_owned()];
    }
    let mut end = LINE - 3;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    vec![format!("{}...", &text[..end])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_line_is_cut_at_a_character_and_fits() {
        let long = "ж".repeat(200);
        let lines = fit(&long);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].len() <= LINE && lines[0].ends_with("..."));
        assert_eq!(fit("short"), ["short"]);
    }
}
