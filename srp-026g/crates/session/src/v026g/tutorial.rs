//! The tutorial's state of an account, and what its steps pay
//! (`docs/notes/lobby.md` §2.21).
//!
//! The client runs the tutorial itself; the server keeps which steps are
//! done, tells the client (`9D`) and pays the awards of the game's own
//! `gameplay/awards/unique_awards.options` ([`survarium::v026g::awards`]).
//!
//! `9D` is `bool show, u32 a, u32 b`. With `show` the client's tutorial
//! leaves its disabled state by `a` (0x283190, PROVEN):
//!
//! | `a` | state |
//! |---|---|
//! | bit 0 clear | `tutorial_state_choose_nickname` |
//! | bit 0 set, `b` == 0 | `tutorial_state_click_inventory` |
//! | bit 0 set, `b` != 0 | `tutorial_state_equip_drop` |
//! | bit 1 or bit 5 set (`a & 0x22`) | `default_state`: no tutorial |
//!
//! The other bits are the steps the client reports with `4B 01 <step>`:
//! **bit `n` is step `n`** (PROVEN for 2 and 3: the survival notes' reward
//! is shown while `a >> 3 & 1` (notes 0, "prepare to battle", which send
//! step 3) or `a >> 2 & 1` (notes 1, "items", step 2) is clear, 0x2833F1;
//! for 4: the match's own tutorial is skipped when `a & 0x10`, 0x27AE46,
//! and sends step 4 when done, 0x24921D; for 1: `a >> 1 & 1` opens
//! `state_choose_faction`, 0x28E9AD, and step 1 is sent where the tutorial
//! goes to its default state, 0x21D461). So:
//!
//! | step | bit | what was done | award |
//! |---|---|---|---|
//! | none | 0 | a nickname chosen (`49 04` in the tutorial) [LIKELY] | `tutorial_change_name`: nothing |
//! | 1 | 1 | the tutorial | `tutorial_completed`: 1000 silver, 250 experience |
//! | 2 | 2 | the notes about items read | `survival_item_notes_completed`: 25 gold |
//! | 3 | 3 | the notes about battle read | `survival_prepare_to_battle_notes_completed`: 25 gold |
//! | 4 | 4 | the tutorial inside a match | `game_world_tutorial_completed`: 50 gold |
//! | none | 5 | skipped (`4B 02`) [LIKELY: the other bit of 0x22] | none |
//!
//! That the server paid exactly these rows for these steps is LIKELY (the
//! client shows them as the steps' rewards: `set_tutorial_rewards`).
//!
//! srp's own choices:
//! - A new account has bit 0 set: its nickname is the name it signed up
//!   with, so the tutorial starts at the inventory. (A rename is
//!   [`super::names`].)
//! - `b` is 1 once the account has finished a match (it has experience),
//!   else 0 [GUESS: the name of the state it selects, "equip drop", and a
//!   new player's first match always dropping an item].
//! - A step is paid once.
//! - `SRP_TUTORIAL=off` ([`super::premium::Settings`]) sends `show` false:
//!   the client's tutorial stays disabled, as srp had it before; the steps
//!   a client reports are still kept and paid.
//!
//! **`46 0A`, the "choose a weapon" step** (`tutorial_state_choose_wpn`
//! 0x2A45D0). The client sends `49 02` and `46 0A` and then waits for the
//! inventory: its `inventory_content_filled` event, raised when an `89` is
//! read (0x2886F5), makes it highlight the PPSh-41 and the Mosin carbine
//! (dict 106 and 107, constants at 0x283470). Until then the tutorial's
//! sheet covers the lobby and no weapon can be dragged (PROVEN; the first
//! live session, 2026-10-12, stopped there: srp answered nothing). So
//! `46 0A` is answered with the inventory and the account
//! ([`choose_weapon`]). What the original server did besides is not in the
//! client [LIKELY: it handed the two weapons over at this step; the
//! step's twin without the two requests is named
//! `tutorial_state_choose_wpn_no_check`]. srp's choice: a new account owns
//! both from the start (`loadouts::STARTER_STASH`); an account still in
//! the tutorial that lacks one gets it here, since the client's highlight
//! needs both.

use survarium::v026g::awards::{Awards, kind, name};
use survarium::v026g::dictionary::Dictionary;
use survarium::v026g::lobby::{self, FULL_CONDITION, ServerMessage};

use super::account::Account;
use super::quests::Notice;
use super::{notices, rewards};

/// The bits of the tutorial's state (`a` of `9D`).
pub mod flag {
    pub const NICKNAME: u32 = 1 << 0;
    pub const COMPLETED: u32 = 1 << 1;
    pub const ITEM_NOTES: u32 = 1 << 2;
    pub const BATTLE_NOTES: u32 = 1 << 3;
    pub const GAME_WORLD: u32 = 1 << 4;
    pub const SKIPPED: u32 = 1 << 5;
    /// With either the client has no tutorial to run (`a & 0x22`).
    pub const OVER: u32 = COMPLETED | SKIPPED;
}

/// The state of a new account.
pub const NEW_ACCOUNT: u32 = flag::NICKNAME;
/// The steps the client reports.
pub const STEPS: std::ops::RangeInclusive<u8> = 1..=4;

/// What a step paid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Paid {
    pub silver: u32,
    pub gold: u32,
    pub experience: u32,
    /// The notices of the levels the experience reached.
    pub notices: Vec<Notice>,
}

/// The award of a step.
fn award_of(step: u8) -> Option<&'static str> {
    Some(match step {
        1 => name::TUTORIAL_COMPLETED,
        2 => name::ITEM_NOTES,
        3 => name::BATTLE_NOTES,
        4 => name::GAME_WORLD_TUTORIAL,
        _ => return None,
    })
}

/// `4B 01 <step>`: the client did a step. Marks it and pays its award the
/// first time; `None` for a step that is none or was done before.
pub fn step(account: &mut Account, step: u8) -> Option<Paid> {
    let award = award_of(step)?;
    let bit = 1_u32 << step;
    if account.extras.tutorial_flags & bit != 0 {
        return None;
    }
    account.extras.tutorial_flags |= bit;
    super::rules::tutorial_step(account, step);
    if step == 1 {
        account.tutorial_done = true;
    }
    let mut paid = Paid::default();
    for row in Awards::shared().get(award) {
        match (row.kind, row.param) {
            (kind::MONEY, 0) => paid.silver = paid.silver.saturating_add(row.amount),
            (kind::MONEY, _) => paid.gold = paid.gold.saturating_add(row.amount),
            (kind::EXPERIENCE, _) => paid.experience = paid.experience.saturating_add(row.amount),
            // No step of the file pays reputation, and a row does not say
            // with whom.
            _ => {}
        }
    }
    account.generic_money = account.generic_money.saturating_add(paid.silver);
    account.premium_money = account.premium_money.saturating_add(paid.gold);
    if paid.experience > 0 {
        let (levels, _, _) = rewards::pay_levels(account, paid.experience);
        paid.notices = notices::levels_reached(&levels);
    }
    Some(paid)
}

/// `4B 02`: the tutorial was skipped. Returns whether that is news.
pub fn skip(account: &mut Account) -> bool {
    super::rules::skip_tutorial(account);
    let news = account.extras.tutorial_flags & flag::SKIPPED == 0;
    account.extras.tutorial_flags |= flag::SKIPPED;
    news
}

/// The weapons the "choose a weapon" step highlights, by the game's item
/// names: dict 106 and 107, the client's constants
/// (`survarium::v026g::lobby::tutorial_model::CHOICE`).
pub const CHOICE: [&str; 2] = ["ppsh", "mosin_nagant"];

/// `46 0A`: the client is at the "choose a weapon" step and waits for the
/// inventory (see the module). An account still in the tutorial that lacks
/// one of the two weapons gets it; returns the dict ids given. The caller
/// sends the inventory in any case.
pub fn choose_weapon(account: &mut Account) -> Vec<u16> {
    if !is_running(account) {
        return Vec::new();
    }
    let dictionary = Dictionary::shared();
    let mut given = Vec::new();
    for name in CHOICE {
        let Some(item) = dictionary.item_named(name) else {
            continue;
        };
        if account
            .inventory
            .iter()
            .all(|owned| owned.dict_id != item.dict_id)
        {
            account.add_item(item.dict_id, FULL_CONDITION);
            given.push(item.dict_id);
        }
    }
    given
}

/// The account chose a nickname.
pub fn nickname_chosen(account: &mut Account) {
    account.extras.tutorial_flags |= flag::NICKNAME;
}

/// Whether the account still has the tutorial before it.
pub fn is_running(account: &Account) -> bool {
    account.extras.tutorial_flags & flag::OVER == 0
}

/// `9D` for the account; `show` is the operator's setting (see the
/// module).
pub fn message(account: &Account, show: bool) -> ServerMessage {
    let has_played = u32::from(account.exp_total > 0);
    lobby::tutorial(show, account.extras.tutorial_flags, has_played)
}

#[cfg(test)]
mod tests {
    use super::*;
    use survarium::v026g::lobby::client_model::{Client, Decoded};

    #[test]
    fn a_new_account_starts_at_the_inventory() {
        let account = Account::new("Alice");
        assert_eq!(account.extras.tutorial_flags, flag::NICKNAME);
        assert!(is_running(&account));
        let message = message(&account, true);
        assert_eq!(message.id_byte(), 0x9D);
        assert_eq!(message.body(), [1, 1, 0, 0, 0, 0, 0, 0, 0]);
        let mut client = Client::signed_in();
        assert!(matches!(
            client.receive(&message).unwrap(),
            Decoded::Tutorial {
                show: true,
                a: 1,
                b: 0
            }
        ));
        // Switched off: the client's tutorial stays disabled.
        assert_eq!(super::message(&account, false).body()[0], 0);
    }

    #[test]
    fn a_step_is_paid_once_by_the_games_awards() {
        let mut account = Account::new("Alice");
        let (silver, gold) = (account.generic_money, account.premium_money);
        // The notes: 25 gold each.
        assert_eq!(
            step(&mut account, 2),
            Some(Paid {
                gold: 25,
                ..Paid::default()
            })
        );
        assert_eq!(step(&mut account, 2), None);
        assert_eq!(step(&mut account, 3).unwrap().gold, 25);
        // The match's own tutorial: 50 gold.
        assert_eq!(step(&mut account, 4).unwrap().gold, 50);
        assert_eq!(account.premium_money, gold + 100);
        assert!(is_running(&account));
        // The tutorial itself: 1000 silver and 250 experience.
        let paid = step(&mut account, 1).unwrap();
        assert_eq!((paid.silver, paid.experience, paid.gold), (1000, 250, 0));
        assert!(paid.notices.is_empty());
        assert_eq!(account.generic_money, silver + 1000);
        assert_eq!(account.exp_total, 250);
        assert!(!is_running(&account));
        assert!(account.tutorial_done);
        assert_eq!(account.extras.tutorial_flags, 0b1_1111);
        // No such steps.
        assert_eq!(step(&mut account, 0), None);
        assert_eq!(step(&mut account, 5), None);
        assert_eq!(step(&mut account, 255), None);
        // `b`: it has experience now.
        assert_eq!(
            message(&account, true).body(),
            [1, 0x1F, 0, 0, 0, 1, 0, 0, 0]
        );
    }

    #[test]
    fn the_tutorials_experience_can_reach_a_level() {
        let mut account = Account::new("Alice");
        account.exp_total = 400;
        let gold = account.premium_money;
        let paid = step(&mut account, 1).unwrap();
        // Level 1 at 500: its 100 gold and its notice.
        assert_eq!(paid.notices, [(14, "[1][1][1][100]".to_owned())]);
        assert_eq!(account.level(), 1);
        assert_eq!(account.premium_money, gold + 100);
    }

    #[test]
    fn the_weapons_to_choose_are_the_clients_two() {
        use survarium::v026g::lobby::tutorial_model;
        let dictionary = Dictionary::shared();
        let ids = CHOICE.map(|name| dictionary.item_named(name).unwrap().dict_id);
        assert_eq!(ids, tutorial_model::CHOICE);
        // A new account owns both: nothing is given.
        let mut account = Account::new("Alice");
        let owned = account.inventory.len();
        assert_eq!(choose_weapon(&mut account), Vec::<u16>::new());
        assert_eq!(account.inventory.len(), owned);
        // One is gone: it comes back, once, whole.
        account.inventory.retain(|item| item.dict_id != 107);
        assert_eq!(choose_weapon(&mut account), [107]);
        assert_eq!(choose_weapon(&mut account), Vec::<u16>::new());
        let mosin = account.inventory.last().unwrap();
        assert_eq!(
            (mosin.dict_id, mosin.condition_or_stack),
            (107, FULL_CONDITION)
        );
        // Not after the tutorial.
        account.inventory.retain(|item| item.dict_id != 106);
        skip(&mut account);
        assert_eq!(choose_weapon(&mut account), Vec::<u16>::new());
    }

    #[test]
    fn skipping_ends_it() {
        let mut account = Account::new("Alice");
        assert!(skip(&mut account));
        assert!(!skip(&mut account));
        assert!(!is_running(&account));
        assert!(account.tutorial_done);
        assert_eq!(
            account.extras.tutorial_flags,
            flag::NICKNAME | flag::SKIPPED
        );
        // The notes can still be read for their gold.
        assert!(step(&mut account, 2).is_some());
    }
}
