//! The page over the 0.26g stack (`srp::v026g`): the exchange and the admin
//! act on the 0.26g accounts ([`GameAccounts`]) and tell the lobby, so an
//! open game shows the change.
//!
//! - The password a player types is checked as the 0.26g login server keeps
//!   it: the client sends the SHA-1 of the password as 40 lower-case hex
//!   characters, and the store keeps the argon2id hash of that digest
//!   (`SessionStore::sign_in_with_digest_from`). So the page makes the same
//!   digest of what was typed and gives it to `check_password_from`. Neither
//!   the password nor the digest is logged or kept.
//! - Gold for silver is the game's own action (`47 04`,
//!   [`rules::exchange_premium`]). Silver for gold is srp's own, at the same
//!   rate: the client has no such button.
//! - The admin's tabs: money, the account (premium time, experience or
//!   level, reputation), an item by its `items_dict` id or name, and the
//!   operator's text (`A5` / `A6`) for everyone in the lobby. Bans,
//!   maintenance and registration are the store's, which the 0.26g login
//!   server and lobby read: they are the same code as for 0.21d.

use std::sync::{Arc, Mutex};

use session::v026g::account::{self, Account, MAX_LEVEL};
use session::v026g::{GameAccounts, rules};
use session::{LockExt, SignInError};
use survarium::v026g::dictionary::Dictionary;
use survarium::v026g::lobby::FULL_CONDITION;

use super::{
    AdminView, Form, Lang, MAX_EXCHANGE_GOLD, Request, Response, Stats, Web, apply, delta, page,
    unix_now,
};

/// What the page tells the 0.26g lobby. srp joins it to the lobby's inbox
/// (`lobby_server::v026g::Inbox`); this crate does not know the lobby.
pub trait LobbyLink: Send + Sync {
    /// The account was changed here: its client, if it is in the lobby, is
    /// sent the account again.
    fn account_changed(&self, account_id: u32);
    /// The operator's two lines for every client (`A5`), until cleared.
    fn set_admin_text(&self, first: &str, second: &str);
    /// Takes them away (`A6`).
    fn clear_admin_text(&self);
    /// The lines that stand.
    fn admin_text(&self) -> Option<(String, String)>;
}

/// The match server's count of matches created and not yet ended.
pub trait MatchCounter: Send + Sync {
    fn active_matches(&self) -> usize;
}

/// What makes the page a 0.26g one.
#[derive(Clone)]
pub struct Game {
    pub accounts: Arc<GameAccounts>,
    pub lobby: Arc<dyn LobbyLink>,
    pub matches: Arc<dyn MatchCounter>,
}

/// The longest password the 0.26g client sends (its buffer is 48 bytes).
const MAX_PASSWORD_LEN: usize = 47;
/// The longest line of the operator's text: the client's buffers are 64
/// bytes (`social.md` §5).
pub(crate) const MAX_TEXT_LEN: usize = 63;
/// The most of an item the admin gives at once: items that are no stack,
/// and units of a stack.
const MAX_GIVEN_ITEMS: u32 = 8;
const MAX_GIVEN_UNITS: u32 = 100_000;
/// The factions the lobby shows a reputation for.
const FACTIONS: std::ops::RangeInclusive<u8> = 1..=4;

/// SHA-1 as the 0.26g client prints it: 40 lower-case hex characters.
fn sha1_hex(bytes: &[u8]) -> [u8; 40] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = [0; 40];
    for (pair, byte) in hex.chunks_mut(2).zip(openssl::sha::sha1(bytes)) {
        pair[0] = HEX[usize::from(byte >> 4)];
        pair[1] = HEX[usize::from(byte & 0xF)];
    }
    hex
}

/// Where the experience of `level` starts.
fn level_start(level: u8) -> u32 {
    let mut total = 0;
    loop {
        let (at, start, next) = account::level_at(total);
        if at >= level || next <= total {
            return start;
        }
        total = next;
    }
}

fn spent_skill_points(account: &Account) -> u32 {
    account
        .skills
        .iter()
        .map(|&(_, points)| u32::from(points))
        .sum()
}

impl Web {
    pub(crate) fn route_026g(&self, request: &Request, peer: &str) -> Response {
        let lang = request.lang;
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/") => page(
                lang,
                Form::Home(Stats {
                    registered: self.store.registered_accounts(),
                    online: self.store.players_present(),
                    maintenance: self.store.minutes_to_maintenance(),
                    registrations_open: self.store.registrations_open(),
                    v026g: true,
                    launcher: crate::launcher::offers_launcher(),
                }),
                "",
            ),
            ("GET", "/exchange") => page(lang, Form::Exchange26, ""),
            ("POST", "/exchange") => self.exchange_026g(request, peer),
            ("GET", path) if self.is_admin_page(path, request) => page(
                lang,
                Form::Admin(AdminView {
                    g26: true,
                    ..AdminView::default()
                }),
                "",
            ),
            ("POST", path) if self.is_admin_page(path, request) => self.admin(request, peer),
            _ => Response {
                status: "404 Not Found",
                content_type: super::ContentType::Html,
                body: page(lang, Form::None26, &lang.tr("Not found.", "Не знайдено.")).body,
            },
        }
    }

    /// The account `user` has with the typed `password`, checked as the
    /// 0.26g login keeps it. An unknown name is refused, never registered.
    fn check_typed_password(&self, request: &Request) -> Result<u32, SignInError> {
        let user = request.field("user");
        // As typed: the game hashes every byte, spaces too.
        let password = request.raw("password").as_bytes();
        if password.is_empty() || password.len() > MAX_PASSWORD_LEN {
            return Err(SignInError::WrongPassword);
        }
        let digest = sha1_hex(password);
        match request.client {
            Some(ip) => self.store.check_password_from(user, &digest, ip),
            None => self.store.check_password(user, &digest),
        }
    }

    fn exchange_026g(&self, request: &Request, peer: &str) -> Response {
        let lang = request.lang;
        let answer = |message: String| page(lang, Form::Exchange26, &message);
        let Some(game) = &self.game else {
            return answer(String::new());
        };
        let user = request.field("user");
        let wrong_login = lang.tr(
            "Wrong user name or password.",
            "Неправильне ім'я користувача або пароль.",
        );
        let gold: u32 = match request.field("gold").parse() {
            Ok(gold) if (1..=MAX_EXCHANGE_GOLD).contains(&gold) => gold,
            _ => {
                return answer(lang.tr(
                    &format!("Gold must be 1 to {MAX_EXCHANGE_GOLD}."),
                    &format!("Золота має бути від 1 до {MAX_EXCHANGE_GOLD}."),
                ));
            }
        };
        let sell = request.field("way") == "sell";
        let account_id = match self.check_typed_password(request) {
            Ok(id) => id,
            Err(SignInError::Banned) => {
                log::warn!("Exchange for banned account {user:?} from {peer} refused");
                return answer(lang.tr("This account is banned.", "Цей акаунт заблоковано."));
            }
            Err(SignInError::TooManyAttempts | SignInError::TooManyFromAddress) => {
                return answer(lang.tr(
                    "Too many wrong passwords; wait a minute.",
                    "Забагато хибних паролів; зачекайте хвилину.",
                ));
            }
            Err(_) => {
                log::warn!("Exchange for {user:?} from {peer}: wrong user name or password");
                return answer(wrong_login);
            }
        };
        let Some(account) = game.accounts.get(account_id) else {
            return answer(lang.tr(
                "Sign in to the game once first: this account has nothing to exchange yet.",
                "Спершу увійдіть у гру: цьому акаунту ще нічого обмінювати.",
            ));
        };
        let rate = rules::PREMIUM_EXCHANGE_RATE;
        let silver = gold.saturating_mul(rate);
        let result = {
            let mut account = account.locked();
            let done = if sell {
                // The game's own exchange (`47 04`).
                match rules::exchange_premium(&mut account, gold) {
                    Ok(_) => Ok(()),
                    Err(rules::ActionError::NotEnoughPremiumMoney) => Err(lang.tr(
                        &format!("Not enough gold: you have {}.", account.premium_money),
                        &format!("Недостатньо золота: у вас {}.", account.premium_money),
                    )),
                    Err(_) => Err(lang.tr(
                        "That would overflow the silver balance.",
                        "Баланс срібла переповниться.",
                    )),
                }
            } else {
                match (
                    account.generic_money.checked_sub(silver),
                    account.premium_money.checked_add(gold),
                ) {
                    (Some(left), Some(total)) => {
                        account.generic_money = left;
                        account.premium_money = total;
                        Ok(())
                    }
                    (None, _) => Err(lang.tr(
                        &format!(
                            "Not enough silver: {gold} gold costs {silver}, you have {}.",
                            account.generic_money
                        ),
                        &format!(
                            "Недостатньо срібла: {gold} золота коштує {silver}, у вас {}.",
                            account.generic_money
                        ),
                    )),
                    _ => Err(lang.tr(
                        "That would overflow the gold balance.",
                        "Баланс золота переповниться.",
                    )),
                }
            };
            done.map(|()| (account.generic_money, account.premium_money))
        };
        match result {
            Ok((silver_now, gold_now)) => {
                game.accounts.mark_dirty(account_id);
                game.lobby.account_changed(account_id);
                let (en, uk) = if sell {
                    (
                        format!("{gold} gold for {silver} silver"),
                        format!("{gold} золота за {silver} срібла"),
                    )
                } else {
                    (
                        format!("{silver} silver for {gold} gold"),
                        format!("{silver} срібла за {gold} золота"),
                    )
                };
                log::info!(
                    "Exchange for {user:?} from {peer}: {en} (now {silver_now} silver, \
                     {gold_now} gold)"
                );
                answer(lang.tr(
                    &format!(
                        "Done: {en}. Now {silver_now} silver and {gold_now} gold. An open game \
                         shows it within a second."
                    ),
                    &format!(
                        "Готово: {uk}. Тепер {silver_now} срібла і {gold_now} золота. Відкрита \
                         гра покаже це за секунду."
                    ),
                ))
            }
            Err(error) => answer(error),
        }
    }

    /// The 0.26g account the admin means by `text`: a user name it signs in
    /// with, or the name other players see if only one account has it.
    fn find_026g(
        &self,
        game: &Game,
        lang: Lang,
        text: &str,
    ) -> Result<(u32, Arc<Mutex<Account>>), String> {
        let text = text.trim();
        let by_nickname = || {
            let mut found = game
                .accounts
                .ranking()
                .into_iter()
                .filter(|row| row.1.eq_ignore_ascii_case(text))
                .map(|row| row.0);
            let first = found.next()?;
            found.next().is_none().then_some(first)
        };
        let Some(account_id) = self.store.find_account(text).or_else(by_nickname) else {
            return Err(lang.tr("No such user.", "Такого користувача немає."));
        };
        match game.accounts.get(account_id) {
            Some(account) => Ok((account_id, account)),
            None => Err(lang.tr(
                "That account has not signed in to the 0.26g lobby yet: it has nothing to change.",
                "Цей акаунт ще не входив у лобі 0.26g: змінювати нічого.",
            )),
        }
    }

    /// The account was changed: written soon, and its open game is told.
    fn changed_026g(&self, game: &Game, account_id: u32) {
        game.accounts.mark_dirty(account_id);
        game.lobby.account_changed(account_id);
    }

    pub(crate) fn admin_money_026g(&self, request: &Request, peer: &str) -> String {
        let lang = request.lang;
        let Some(game) = &self.game else {
            return String::new();
        };
        let user = request.field("user");
        let (account_id, account) = match self.find_026g(game, lang, user) {
            Ok(found) => found,
            Err(message) => return message,
        };
        let (Some(silver), Some(gold)) =
            (delta(request.field("silver")), delta(request.field("gold")))
        else {
            return lang.tr(
                "Silver and gold are whole numbers, + or -, or empty.",
                "Срібло і золото — цілі числа, + або -, або порожньо.",
            );
        };
        let (silver_now, gold_now) = {
            let mut account = account.locked();
            account.generic_money = apply(account.generic_money, silver);
            account.premium_money = apply(account.premium_money, gold);
            (account.generic_money, account.premium_money)
        };
        self.changed_026g(game, account_id);
        log::info!(
            "Admin from {peer}: {user:?} silver {silver:+}, gold {gold:+} \
             (now {silver_now} silver, {gold_now} gold)"
        );
        lang.tr(
            &format!("{user}: now {silver_now} silver and {gold_now} gold."),
            &format!("{user}: тепер {silver_now} срібла і {gold_now} золота."),
        )
    }

    /// Premium time, experience or level, reputation: each field that is
    /// filled is applied; nothing is when one of them is wrong.
    pub(crate) fn admin_account_026g(&self, request: &Request, peer: &str) -> String {
        let lang = request.lang;
        let Some(game) = &self.game else {
            return String::new();
        };
        let user = request.field("acc_user");
        let (account_id, account) = match self.find_026g(game, lang, user) {
            Ok(found) => found,
            Err(message) => return message,
        };
        let bad = lang.tr(
            &format!(
                "Premium hours, experience and reputation are whole numbers, + or -; the level \
                 is 0 to {MAX_LEVEL}; the faction is 1 to 4."
            ),
            &format!(
                "Години преміуму, досвід і репутація — цілі числа, + або -; рівень від 0 до \
                 {MAX_LEVEL}; фракція від 1 до 4."
            ),
        );
        let (Some(hours), Some(experience), Some(reputation)) = (
            delta(request.field("premium_hours")),
            delta(request.field("experience")),
            delta(request.field("reputation")),
        ) else {
            return bad;
        };
        let level = match request.field("level") {
            "" => None,
            text => match text.parse::<u8>() {
                Ok(level) if level <= MAX_LEVEL => Some(level),
                _ => return bad,
            },
        };
        let faction = match request.field("faction") {
            "" if reputation == 0 => None,
            text => match text.parse::<u8>() {
                Ok(faction) if FACTIONS.contains(&faction) => Some(faction),
                _ => return bad,
            },
        };
        let now = unix_now();
        let summary = {
            let mut account = account.locked();
            let mut changed = account.clone();
            // The level, then the experience on top of it.
            if let Some(level) = level {
                changed.exp_total = level_start(level);
            }
            changed.exp_total = apply(changed.exp_total, experience);
            if u32::from(changed.level()) < spent_skill_points(&changed) {
                return lang.tr(
                    "That level is under the skill points the account has spent; reset its \
                     skills in the game first.",
                    "Цей рівень нижчий за витрачені очки навичок; спершу скиньте навички в грі.",
                );
            }
            if hours != 0 {
                let until = i64::try_from(changed.premium_until.max(now)).unwrap_or(i64::MAX);
                let until = until.saturating_add(hours.saturating_mul(3600));
                changed.premium_until = u64::try_from(until)
                    .ok()
                    .filter(|at| *at > now)
                    .unwrap_or(0);
                // Its premium kit is no longer usable: back to the first.
                changed.settle_active_kit(now);
            }
            if let Some(faction) = faction {
                if reputation >= 0 {
                    changed.add_reputation(faction, reputation as u32);
                } else if let Some(entry) = changed
                    .reputations
                    .iter_mut()
                    .find(|(id, _)| *id == faction)
                {
                    entry.1 = apply(entry.1, reputation);
                }
            }
            *account = changed;
            (
                account.level(),
                account.exp_total,
                account.premium_hours(now).unwrap_or(0),
                faction.map(|faction| (faction, account.reputation(faction))),
            )
        };
        self.changed_026g(game, account_id);
        let (level_now, exp_now, premium_now, reputation_now) = summary;
        log::info!(
            "Admin from {peer}: {user:?} premium {hours:+} h, experience {experience:+}, level \
             {level:?}, reputation {reputation:+} with {faction:?} (now level {level_now}, \
             {exp_now} experience, {premium_now} h premium)"
        );
        let mut message = lang.tr(
            &format!(
                "{user}: level {level_now} ({exp_now} experience), {premium_now} h of premium."
            ),
            &format!("{user}: рівень {level_now} ({exp_now} досвіду), {premium_now} год преміуму."),
        );
        if let Some((faction, points)) = reputation_now {
            message += &lang.tr(
                &format!(" Reputation with faction {faction}: {points}."),
                &format!(" Репутація у фракції {faction}: {points}."),
            );
        }
        message
    }

    /// An item of the 0.26g dictionary, by its id or its name: new and
    /// whole, into the inventory.
    pub(crate) fn admin_item_026g(&self, request: &Request, peer: &str) -> String {
        let lang = request.lang;
        let Some(game) = &self.game else {
            return String::new();
        };
        let user = request.field("item_user");
        let (account_id, account) = match self.find_026g(game, lang, user) {
            Ok(found) => found,
            Err(message) => return message,
        };
        let dictionary = Dictionary::shared();
        let wanted = request.field("item");
        // Only an id the dictionary has: the client crashes on any other.
        let item = match wanted.parse::<u16>() {
            Ok(dict_id) => dictionary.item(dict_id),
            Err(_) => dictionary.item_named(wanted),
        };
        let Some(item) = item else {
            return lang.tr(
                "No such item in the 0.26g dictionary (give its id or its exact name).",
                "Такого предмета немає в словнику 0.26g (вкажіть id або точну назву).",
            );
        };
        let most = if item.is_stack {
            MAX_GIVEN_UNITS
        } else {
            MAX_GIVEN_ITEMS
        };
        let count = match request.field("count") {
            "" => 1,
            text => match text.parse::<u32>() {
                Ok(count) if (1..=most).contains(&count) => count,
                _ => {
                    return lang.tr(
                        &format!("The count for this item is 1 to {most}."),
                        &format!("Кількість для цього предмета від 1 до {most}."),
                    );
                }
            },
        };
        {
            let mut account = account.locked();
            if item.is_stack {
                // A stack is one instance: its count grows.
                let dict_id = item.dict_id;
                let owned = account
                    .inventory
                    .iter_mut()
                    .find(|owned| owned.dict_id == dict_id);
                match owned {
                    Some(owned) => {
                        let Some(total) = owned.condition_or_stack.checked_add(count) else {
                            return lang.tr("That stack would overflow.", "Ця купа переповниться.");
                        };
                        owned.condition_or_stack = total;
                    }
                    None => {
                        account.add_item(dict_id, count);
                    }
                }
            } else {
                for _ in 0..count {
                    account.add_item(item.dict_id, FULL_CONDITION);
                }
            }
            account.sync_kits();
        }
        self.changed_026g(game, account_id);
        log::info!(
            "Admin from {peer}: {user:?} given {count} of item {} ({})",
            item.dict_id,
            item.name
        );
        lang.tr(
            &format!(
                "{user}: given {count} of {} (id {}).",
                item.name, item.dict_id
            ),
            &format!(
                "{user}: видано {count} × {} (id {}).",
                item.name, item.dict_id
            ),
        )
    }

    /// The operator's text for everyone in the lobby: set, or cleared.
    pub(crate) fn admin_text_026g(&self, request: &Request, peer: &str) -> String {
        let lang = request.lang;
        let Some(game) = &self.game else {
            return String::new();
        };
        if !request.field("clear_text").is_empty() {
            game.lobby.clear_admin_text();
            log::warn!("Admin from {peer}: cleared the text for all players");
            return lang.tr("The text is cleared.", "Текст прибрано.");
        }
        let (first, second) = (request.field("line1"), request.field("line2"));
        if first.is_empty() && second.is_empty() {
            return String::new();
        }
        let fits = |line: &str| line.len() <= MAX_TEXT_LEN && !line.chars().any(char::is_control);
        if !fits(first) || !fits(second) {
            return lang.tr(
                &format!(
                    "Each line is at most {MAX_TEXT_LEN} bytes (a Cyrillic letter is 2) with no \
                     control characters."
                ),
                &format!(
                    "Кожен рядок — щонайбільше {MAX_TEXT_LEN} байти (кирилична літера — 2), без \
                     керівних символів."
                ),
            );
        }
        game.lobby.set_admin_text(first, second);
        log::warn!("Admin from {peer}: text for all players: {first:?} / {second:?}");
        lang.tr(
            "The text is shown to everyone in the lobby, and to who signs in, until cleared.",
            "Текст показано всім у лобі й тим, хто входить, доки його не приберуть.",
        )
    }
}

/// The exchange form of the 0.26g page: both ways.
pub(crate) fn exchange_form(lang: Lang) -> String {
    let code = lang.code();
    let rate = rules::PREMIUM_EXCHANGE_RATE;
    format!(
        "<h1>{}</h1><form method=post action=/exchange autocomplete=off>\
         <input type=hidden name=lang value={code}>\
         <label for=u>{}</label><input id=u name=user required maxlength=99>\
         <label for=p>{}</label><input id=p name=password type=password required maxlength=47>\
         <label class=check><input type=radio name=way value=buy checked> {}</label>\
         <label class=check><input type=radio name=way value=sell> {}</label>\
         <label for=g>{}</label><input id=g name=gold type=number min=1 max={MAX_EXCHANGE_GOLD} required>\
         <small>{}</small><br><button>{}</button></form>",
        lang.tr("Silver and gold", "Срібло і золото"),
        lang.tr("User name", "Ім'я користувача"),
        lang.tr(
            "Password (the one you type in the game)",
            "Пароль (той, що ви вводите у грі)"
        ),
        lang.tr("Buy gold with silver", "Купити золото за срібло"),
        lang.tr("Sell gold for silver", "Продати золото за срібло"),
        lang.tr("Gold", "Золото"),
        lang.tr(
            &format!("{rate} silver for each gold, either way."),
            &format!("{rate} срібла за 1 золота, в обидва боки."),
        ),
        lang.tr("Exchange", "Обміняти"),
    )
}

#[cfg(test)]
mod tests;
