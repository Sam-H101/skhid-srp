//! A game older than the server asks for does not play, and is told what to
//! do (`SRP_PATCH_REVISION_CHECK`, `SRP_MIN_PATCH_REVISION`).
//!
//! A patched exe says its patch revision in its sign-in (`docs/notes/
//! client_patch.md` §6), and the session keeps it. Games of different
//! revisions do not always compute a match alike (revision 3 changed the
//! rule of a team's last battery), so the server can ask for a least one.
//!
//! The check is at Play, not at the sign-in. A sign-in can only be answered
//! with the game's own "You're using an outdated version of the game."
//! (0x22), which does not say what to do, and an old exe cannot be taught a
//! new text. In the lobby the player is refused the same way, in the same
//! words, and reads how to update in the chat:
//! - Play (`2F`) and clan-team Play (`30`) are refused with `84 <op> 00
//!   <text>`. The client localizes the text and shows it in a message box,
//!   then puts its Play button back (`lobby_menu` 0x21D9D0, `docs/notes/
//!   lobby_requests.md` §4.2). A text that is no key of the game is shown as
//!   it is (`text_translator::translate_text` 0x1FF0F0 gives the text back
//!   when it finds no such key), so `SRP_OUTDATED_TEXT` may be a sentence;
//!   the default is the game's own key, in the player's language;
//! - a line in the general chat, to that player alone, says what to do
//!   (`SRP_UPDATE_HINT`): at the refusal, and when the game opens the
//!   general chat. It is a line as `queue_chat` sends them.
//!
//! `warn` sends the line and refuses nothing. The experimental late-join exe
//! (revision 0x80) is over any least revision. A test bot (`SRP_BOT_ACCOUNTS`)
//! signs in as revision 0 and is never checked.

use std::time::{Duration, Instant};

use survarium::v026g::lobby::channel;

use super::core::{Core, Who};
use super::queue_chat;

/// `SRP_MIN_PATCH_REVISION` when unset: the players' download of 2026-10-13.
pub const DEFAULT_MIN_PATCH_REVISION: u8 = 4;
/// `SRP_OUTDATED_TEXT` when unset: "You're using an outdated version of the
/// game.", in each of the game's languages.
pub const DEFAULT_OUTDATED_TEXT: &str = "st_login_menu_status_invalid_version";
/// The hint is said again to a player no sooner than this.
const REPEAT: Duration = Duration::from_secs(30);

/// `SRP_PATCH_REVISION_CHECK`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PatchCheck {
    /// Nobody is checked.
    #[default]
    Off,
    /// An older game is told to update, and plays.
    Warn,
    /// An older game is told to update, and its Play is refused.
    Refuse,
}

impl PatchCheck {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "0" | "false" | "no" => Some(Self::Off),
            "warn" => Some(Self::Warn),
            "refuse" | "on" | "1" | "true" | "yes" => Some(Self::Refuse),
            _ => None,
        }
    }
}

/// `SRP_UPDATE_HINT` when unset. A server sets its own to name its page.
pub const DEFAULT_UPDATE_HINT: &str = "Your game is outdated. Close it and start Skhid.bat: it \
                                       updates the game. No Skhid.bat? Download the new game \
                                       files from the server's page.";

impl Core<'_> {
    /// The patch revision of `who`'s game, when it is older than the server
    /// asks for.
    fn outdated(&self, who: &Who) -> Option<u8> {
        if self.settings.patch_check == PatchCheck::Off {
            return None;
        }
        self.store
            .client_revision(who.session_id)
            .filter(|revision| *revision < self.settings.min_patch_revision)
            .filter(|_| !self.accounts.is_bot_name(&who.name))
    }

    /// Whether `who`'s Play (`op`: `2F` or `30`) is refused for its game's
    /// revision: then it was answered, with the message box and the hint.
    pub(super) fn refuse_outdated(&mut self, who: &Who, op: u8, now: Instant) -> bool {
        let Some(revision) = self.outdated(who) else {
            return false;
        };
        let refuse = self.settings.patch_check == PatchCheck::Refuse;
        log::info!(
            "{}: its game says patch revision {revision}, the server asks for {}: Play {}",
            who.name,
            self.settings.min_patch_revision,
            if refuse { "refused" } else { "let through" }
        );
        if refuse {
            let text = self.settings.outdated_text.clone();
            self.deny(who.addr, op, &text);
        }
        self.say_update_hint(who, now);
        refuse
    }

    /// The game opened the general chat: an older one reads there what to do.
    pub(super) fn hint_outdated(&mut self, who: &Who, now: Instant) {
        if self.outdated(who).is_some() {
            self.say_update_hint(who, now);
        }
    }

    /// The hint in the general chat, to `who` alone, if its game listens to
    /// that channel and was not told just now.
    fn say_update_hint(&mut self, who: &Who, now: Instant) {
        let hint = self.settings.update_hint.clone();
        let name = self.settings.queue_chat_name.clone();
        let Some(player) = self.player_mut(who.addr) else {
            return;
        };
        let told = player
            .update_hint_at
            .is_some_and(|at| now.saturating_duration_since(at) < REPEAT);
        if hint.is_empty()
            || told
            || !queue_chat::subscribed(player.channel_ids[usize::from(channel::GENERAL)])
        {
            return;
        }
        player.update_hint_at = Some(now);
        for line in queue_chat::fit(&hint) {
            self.send(who.addr, queue_chat::line_of(&name, &line));
        }
    }
}
