//! "Show player's information" outside the results screen (the owner's of
//! 2026-10-20: "you should be able to go to rankings and rmb to do the
//! same", "for every single player"): the game's own window of a player's
//! kits, asked for with the results screen's request (`3D`, a match id no
//! match has) and answered with its `93`, from the account as it is now.
//! Every account is in memory (loaded from srp.db at the start), so one
//! offline, one that never played since a restart and one with no match
//! are answered alike. Spec: `docs/notes/player_gear.md`.

use session::LockExt;
use session::v026g::{quests, wire};

use super::core::{Core, Who};

impl Core<'_> {
    /// `3D 00 <GEAR_BY_PLACE> <place>`: the kits of the player at that
    /// place of the ranking: the account the asker's page was sent there,
    /// or, for a place it was not sent (a row from the sign-in's pushes),
    /// the ranking's now.
    pub(super) fn ranked_gear(&mut self, who: &Who, place: u32) {
        let shown = self
            .player(who.addr)
            .and_then(|player| player.ranked_shown.get(&place).copied());
        let account_id = shown.or_else(|| {
            let index = usize::try_from(place).ok()?.checked_sub(1)?;
            self.accounts.ranking().get(index).map(|row| row.0)
        });
        match account_id {
            Some(account_id) => self.send_gear(who, account_id, place),
            None => log::info!(
                "{} asked for the kits of ranking place {place}: no such place",
                who.name
            ),
        }
    }

    /// `93` with the kits of that account as it is now; nothing for an
    /// account there is not (the client has no "nothing to show" answer: no
    /// window opens).
    pub(super) fn send_gear(&mut self, who: &Who, account_id: u32, id: u32) {
        let Some(account) = self.accounts.get(account_id) else {
            log::info!(
                "{} asked for the kits of account {account_id}: no such account",
                who.name
            );
            return;
        };
        let gear = wire::player_gear(&account.locked(), id, quests::now());
        self.send_made(who.addr, gear);
    }
}
