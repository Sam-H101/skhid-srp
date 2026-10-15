//! A game older than the server takes does not get in
//! (`SRP_PATCH_REVISION_CHECK=refuse`, `SRP_MIN_PATCH_REVISION`).
//!
//! A patched exe says its patch revision in its sign-in (`docs/notes/
//! client_patch.md` §6), and the session keeps it. With `refuse` an older
//! game's sign-in is answered with the game's own "You're using an outdated
//! version of the game." (0x22, the login server's `RevisionGate`, which srp
//! sets from these settings), and a lobby sign-in with a session it had
//! before is refused like an ended one ([`Core::outdated_session`]), the
//! session ended with it. A game that reconnects after a restart of the
//! server is so sent to its login screen once its five tries fail, about
//! half a minute (`transport.md` §6.1), and there it is refused. It never
//! gets a sign-in's pushes: they name items (the price list 0x8A lists all
//! that is sold), and a game crashes on an item its resources.db lacks. The
//! player restarts the game through the launcher, which updates it.
//!
//! A test bot (`SRP_BOT_ACCOUNTS`) signs in as revision 0 and is never
//! checked.

use session::LockExt;

use super::core::Core;

/// `SRP_MIN_PATCH_REVISION` when unset: the players' download of 2026-10-13.
pub const DEFAULT_MIN_PATCH_REVISION: u8 = 4;

/// `SRP_PATCH_REVISION_CHECK`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PatchCheck {
    /// Nobody is checked.
    #[default]
    Off,
    /// An older game is refused at its sign-in, and its sessions from before.
    Refuse,
}

impl PatchCheck {
    /// `off` or `refuse` (any case); empty is off. Nothing else: the `warn`
    /// of before is gone, and a setting the server would read otherwise
    /// than meant must stop it ([`from_env`]).
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "off" => Some(Self::Off),
            "refuse" => Some(Self::Refuse),
            _ => None,
        }
    }
}

/// `SRP_PATCH_REVISION_CHECK` and `SRP_MIN_PATCH_REVISION`. A value that is
/// neither is an error: srp checks it before it starts a server, and stops.
pub fn from_env() -> Result<(PatchCheck, u8), String> {
    let var = |name: &str| std::env::var(name).ok();
    let check = match var("SRP_PATCH_REVISION_CHECK") {
        Some(value) => PatchCheck::parse(&value).ok_or_else(|| {
            format!(
                "SRP_PATCH_REVISION_CHECK is `{value}`; it must be off or refuse (`warn` is no \
                 more: an older game is refused at its sign-in, or let in)"
            )
        })?,
        None => PatchCheck::Off,
    };
    let min = match var("SRP_MIN_PATCH_REVISION") {
        Some(value) => value.trim().parse().map_err(|_| {
            format!("SRP_MIN_PATCH_REVISION is `{value}`; it must be a number from 0 to 255")
        })?,
        None => DEFAULT_MIN_PATCH_REVISION,
    };
    Ok((check, min))
}

impl Core<'_> {
    /// Whether a lobby sign-in with `session_id` is refused for its game's
    /// revision (`refuse`): the account's name and the revision, if so. Only
    /// a session from before a restart can be one: the login server lets no
    /// older game in.
    pub(super) fn outdated_session(&self, session_id: u32) -> Option<(String, u8)> {
        if self.settings.patch_check != PatchCheck::Refuse {
            return None;
        }
        let revision = self.store.client_revision(session_id)?;
        if revision >= self.settings.min_patch_revision {
            return None;
        }
        let name = self
            .store
            .account_for_session(session_id)?
            .locked()
            .name
            .clone();
        (!self.accounts.is_bot_name(&name)).then_some((name, revision))
    }
}
