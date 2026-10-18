//! The admin's password reset (`docs/notes/password-reset.md`): a player
//! who forgot their password asks the owner, who clears it here while srp
//! runs; the player's next sign-in, with any password, sets the new one
//! (`SessionStore::reset_password`, the owner's decision of 2026-10-18).
//!
//! - The page's Password tab: the user name the player signs in with (not
//!   the character name), then a confirm step showing the account it
//!   matched, then the reset.
//! - A script's reset: `tab=reset_password&login=<user name>` with the
//!   token, as release.sh sends its maintenance (curl from the owner's
//!   server, whose address is in `SRP_ADMIN_IPS`). No confirm step; one line
//!   of text back: `ok <login> <account id>` (200), `no such login` (404),
//!   `no login` (400), `wrong token` (403), `locked: too many wrong tokens`
//!   (429), `error: …` (500). An address not in the list, or a page without
//!   a token, gets the plain "not found" as for every admin request.

use std::net::IpAddr;

use session::LockExt;

use super::{ContentType, Lang, Request, Response, TokenRefused, Web, escape};

/// The tab a script names for the reset without the confirm step.
pub(crate) const DIRECT_TAB: &str = "reset_password";

/// What the password tab shows.
#[derive(Default)]
pub(crate) struct ResetView {
    /// The user name in the field.
    typed: String,
    /// The account it matched, to confirm.
    found: Option<Found>,
}

/// The account a user name matched, as the confirm step shows it.
struct Found {
    id: u32,
    login: String,
    /// The character name other players see.
    name: String,
    last_ip: Option<IpAddr>,
    last_sign_in: Option<u64>,
    /// Its games signed in now: the reset signs them out.
    sessions: usize,
}

/// How a reset went.
enum Reset {
    Done {
        login: String,
        id: u32,
        ended: usize,
    },
    NoSuchLogin,
    /// The account was found, then could not be had (never: accounts are
    /// not deleted).
    Gone,
}

impl Web {
    /// The account that signs in as `typed`, by its user name only, in any
    /// case (`persist::user_name_key`, as the `accounts` table's `NOCASE`).
    fn reset_target(&self, typed: &str) -> Option<u32> {
        self.store.account_id_by_name(typed)
    }

    /// Clear the password of `account_id`, logged.
    fn reset(&self, account_id: u32, peer: &str) -> Reset {
        let Some(login) = self
            .store
            .account(account_id)
            .map(|account| account.locked().login.clone())
        else {
            return Reset::Gone;
        };
        let Some(ended) = self.store.reset_password(account_id) else {
            return Reset::Gone;
        };
        log::warn!(
            "Admin from {peer}: reset the password of {login} (account {account_id:#x}); \
             {ended} session(s) ended; its next sign-in sets the new one"
        );
        Reset::Done {
            login,
            id: account_id,
            ended,
        }
    }

    /// The account as the confirm step shows it.
    fn found(&self, account_id: u32) -> Option<Found> {
        let account = self.store.account(account_id)?;
        let (login, name, last_ip, last_sign_in) = {
            let account = account.locked();
            (
                account.login.clone(),
                account.name.clone(),
                account.last_ip,
                account.last_sign_in,
            )
        };
        // On the 0.26g page, the name its character has there.
        let name = self
            .game
            .as_ref()
            .and_then(|game| game.accounts.get(account_id))
            .map_or(name, |account| account.locked().name.clone());
        Some(Found {
            id: account_id,
            login,
            name,
            last_ip,
            last_sign_in,
            sessions: self.store.live_sessions(account_id),
        })
    }

    /// The page's Password tab: the account a user name matched, to
    /// confirm; with the confirm button (`reset_id`, the id it showed) the
    /// reset, if the user name still means that account.
    pub(crate) fn admin_reset_password(
        &self,
        request: &Request,
        peer: &str,
    ) -> (String, ResetView) {
        let lang = request.lang;
        let typed = request.field("reset_user");
        let mut view = ResetView {
            typed: typed.to_owned(),
            found: None,
        };
        if typed.is_empty() {
            let message = lang.tr(
                "Type the user name the player signs in with.",
                "Введіть ім'я, з яким гравець входить у гру.",
            );
            return (message, view);
        }
        let Some(account_id) = self.reset_target(typed) else {
            let message = lang.tr(
                "No account signs in with that user name (character names are not looked up).",
                "Жоден акаунт не входить з таким ім'ям (імена персонажів не шукаються).",
            );
            return (message, view);
        };
        let confirmed = request.field("reset_id");
        if confirmed.is_empty() {
            view.found = self.found(account_id);
            let message = lang.tr(
                "Check the account below, then press the reset button.",
                "Перевірте акаунт нижче, потім натисніть кнопку скидання.",
            );
            return (message, view);
        }
        if confirmed.parse::<u32>().ok() != Some(account_id) {
            view.found = self.found(account_id);
            let message = lang.tr(
                "That user name now means another account: nothing was changed. Check it below.",
                "Це ім'я тепер означає інший акаунт: нічого не змінено. Перевірте його нижче.",
            );
            return (message, view);
        }
        match self.reset(account_id, peer) {
            Reset::Done { login, ended, .. } => {
                view.typed.clear();
                let message = lang.tr(
                    &format!(
                        "The password of {login} is cleared; {ended} session(s) ended. \
                         Their next sign-in, with any password, sets the new one."
                    ),
                    &format!(
                        "Пароль {login} скинуто; завершено сесій: {ended}. \
                         Наступний вхід, з будь-яким паролем, встановить новий."
                    ),
                );
                (message, view)
            }
            Reset::NoSuchLogin | Reset::Gone => (
                lang.tr(
                    "The account could not be had: nothing was changed.",
                    "Акаунт недоступний: нічого не змінено.",
                ),
                view,
            ),
        }
    }

    /// A script's reset: at once, no confirm step, one line back.
    pub(crate) fn admin_reset_direct(&self, request: &Request, peer: &str) -> Response {
        let answer = |status: &'static str, body: String| Response {
            status,
            content_type: ContentType::Text,
            body: body + "\n",
        };
        match self.check_token(request, peer) {
            Ok(()) => {}
            Err(TokenRefused::Locked) => {
                return answer(
                    "429 Too Many Requests",
                    "locked: too many wrong tokens".into(),
                );
            }
            Err(TokenRefused::Wrong) => return answer("403 Forbidden", "wrong token".into()),
        }
        let typed = request.field("login");
        if typed.is_empty() {
            return answer("400 Bad Request", "no login".into());
        }
        let outcome = match self.reset_target(typed) {
            Some(account_id) => self.reset(account_id, peer),
            None => Reset::NoSuchLogin,
        };
        match outcome {
            Reset::Done { login, id, .. } => answer("200 OK", format!("ok {login} {id}")),
            Reset::NoSuchLogin => answer("404 Not Found", "no such login".into()),
            Reset::Gone => answer(
                "500 Internal Server Error",
                "error: the account could not be had; nothing was changed".into(),
            ),
        }
    }
}

/// The Password tab's panel.
pub(crate) fn panel(lang: Lang, view: &super::AdminView) -> String {
    let reset = &view.reset;
    let confirm = match &reset.found {
        Some(found) if !view.token.is_empty() => {
            let last_sign_in = match found.last_sign_in {
                Some(at) => format!("{} UTC", date_time_utc(at)),
                None => lang.tr("not since srp started", "не відколи srp запущено"),
            };
            let last_ip = found
                .last_ip
                .map_or_else(|| lang.tr("unknown", "невідома"), |ip| ip.to_string());
            let sessions = if found.sessions == 0 {
                lang.tr("no", "ні")
            } else {
                lang.tr(
                    &format!("yes, {}: its game is signed out", found.sessions),
                    &format!("так, {}: гру буде виведено з акаунта", found.sessions),
                )
            };
            format!(
                "<table>\
                 <tr><th>{}</th><td>{}</td></tr>\
                 <tr><th>{}</th><td>{}</td></tr>\
                 <tr><th>{}</th><td>{} ({:#x})</td></tr>\
                 <tr><th>{}</th><td>{}</td></tr>\
                 <tr><th>{}</th><td>{}</td></tr>\
                 <tr><th>{}</th><td>{}</td></tr>\
                 </table>\
                 <button name=reset_id value={}>{}</button>",
                lang.tr("User name", "Ім'я для входу"),
                escape(&found.login),
                lang.tr("Character", "Персонаж"),
                escape(&found.name),
                lang.tr("Account id", "Id акаунта"),
                found.id,
                found.id,
                lang.tr("Last sign-in", "Останній вхід"),
                escape(&last_sign_in),
                lang.tr("Last address", "Остання адреса"),
                escape(&last_ip),
                lang.tr("Signed in now", "Зараз у грі"),
                escape(&sessions),
                found.id,
                escape(&lang.tr(
                    &format!("Reset the password of {}", found.login),
                    &format!("Скинути пароль {}", found.login),
                )),
            )
        }
        _ => String::new(),
    };
    format!(
        "<div class=\"panel p-password\">\
         <label for=pu>{}</label><input id=pu name=reset_user maxlength=99 value=\"{}\">\
         <small>{}</small><br><button>{}</button>{confirm}</div>",
        lang.tr(
            "User name the player signs in with (not the character name)",
            "Ім'я, з яким гравець входить (не ім'я персонажа)"
        ),
        escape(&reset.typed),
        lang.tr(
            "Clears the password: the player's next sign-in, with any password, sets it as the \
             new one (owner's decision, 2026-10-18). Whoever signs in first takes it, so reset \
             when the player is ready to sign in. A game signed in with the account is signed \
             out. Case does not matter.",
            "Скидає пароль: наступний вхід гравця, з будь-яким паролем, встановлює його як \
             новий (рішення власника, 2026-10-18). Хто ввійде першим, той його і встановить, \
             тож скидайте, коли гравець готовий увійти. Гру, що зараз у цьому акаунті, буде \
             виведено. Регістр не важливий."
        ),
        lang.tr("Find", "Знайти"),
    )
}

/// `YYYY-MM-DD HH:MM` (UTC) of a Unix time.
fn date_time_utc(at: u64) -> String {
    let (year, month, day) = civil_from_days(at / 86_400);
    let seconds = at % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60
    )
}

/// The date of a day counted from 1970-01-01 (the proleptic Gregorian
/// calendar; Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_utc_calendar_dates() {
        assert_eq!(date_time_utc(0), "1970-01-01 00:00");
        assert_eq!(date_time_utc(951_782_400), "2000-02-29 00:00");
        // 2026-10-18 12:34 UTC.
        assert_eq!(date_time_utc(1_792_326_840), "2026-10-18 12:34");
    }
}
