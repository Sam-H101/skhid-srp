//! No tutorial (the owner's of 2026-10-19): it took a new player through
//! Play into a match, and with rooms nobody is sent into a match they did
//! not choose. The server tells every client that every step is done, so
//! the client runs none of it; a new account starts with more instead
//! ([`signup_grant`]). `docs/notes/lobby.md` §2.21.
//!
//! The client runs the tutorial itself from the state the server sends.
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
//! The other bits are the steps the client reported with `4B 01 <step>`:
//! **bit `n` is step `n`** (PROVEN for 2 and 3: the survival notes' reward
//! is shown while `a >> 3 & 1` (notes 0, "prepare to battle", step 3) or
//! `a >> 2 & 1` (notes 1, "items", step 2) is clear, 0x2833F1; for 4: the
//! match's own tutorial is skipped when `a & 0x10`, 0x27AE46; for 1:
//! `a >> 1 & 1` opens `state_choose_faction`, 0x28E9AD). Bit 0 is a
//! nickname chosen and bit 5 the tutorial skipped (`4B 02`), LIKELY.
//!
//! So srp sends [`ALL_DONE`]: `a & 0x22` puts the lobby in its
//! `default_state`, bit 4 skips the match's own tutorial, bits 2 and 3 hide
//! the notes' rewards. `9D` with `show` off stores the same flags and runs
//! no tutorial (0x27136E): the sign-in's `9D` has `show` on, so the client
//! leaves its disabled state for `default_state`, as after a finished
//! tutorial; the ranking's recounts send it off. A client that sends a
//! step anyway (`4B`, `46 0A`) is not answered.
//!
//! **What the steps paid**, the game's own
//! `gameplay/awards/unique_awards.options` ([`survarium::v026g::awards`]):
//! `tutorial_completed` 1000 silver and 250 experience, the two notes
//! 25 gold each, the match's tutorial 50 gold. **A new account starts**
//! with 10 000 silver and 500 gold ([`SIGNUP_SILVER`], [`SIGNUP_GOLD`]),
//! the steps' experience, and 1000 rounds each of the PPSh-41's and the
//! Mosin's free ammo (the owner's of 2026-10-19). It is the account's
//! state when the client first hears of it: no window, no notice.

use survarium::v026g::awards::{Awards, kind, name};
use survarium::v026g::dictionary::Dictionary;
use survarium::v026g::lobby::{self, ServerMessage};

use super::account::Account;

/// The bits of the tutorial's state (`a` of `9D`).
pub mod flag {
    pub const NICKNAME: u32 = 1 << 0;
    pub const COMPLETED: u32 = 1 << 1;
    pub const ITEM_NOTES: u32 = 1 << 2;
    pub const BATTLE_NOTES: u32 = 1 << 3;
    pub const GAME_WORLD: u32 = 1 << 4;
    pub const SKIPPED: u32 = 1 << 5;
}

/// Every step done: the client runs no tutorial (see the module).
pub const ALL_DONE: u32 = flag::NICKNAME
    | flag::COMPLETED
    | flag::ITEM_NOTES
    | flag::BATTLE_NOTES
    | flag::GAME_WORLD
    | flag::SKIPPED;

/// The awards a new account is given, the tutorial's steps'.
pub const AWARDS: [&str; 4] = [
    name::TUTORIAL_COMPLETED,
    name::ITEM_NOTES,
    name::BATTLE_NOTES,
    name::GAME_WORLD_TUTORIAL,
];

/// What a new account's balance is: silver and gold.
pub const SIGNUP_SILVER: u32 = 10_000;
pub const SIGNUP_GOLD: u32 = 500;

/// The ammo a new account is given besides, `(item, rounds)`: the PPSh-41's
/// (the TT-33's too) and the Mosin carbine's.
pub const SIGNUP_AMMO: [(&str, u32); 2] = [("ammo_7.62x25", 1000), ("ammo_7.62x54r_old", 1000)];

/// The experience the tutorial's steps paid, by the game's file.
pub fn experience() -> u32 {
    AWARDS
        .iter()
        .flat_map(|award| Awards::shared().get(award))
        .filter(|row| row.kind == kind::EXPERIENCE)
        .map(|row| row.amount)
        .sum()
}

/// A new account's start, before its client first hears of it: its
/// balance ([`SIGNUP_SILVER`], [`SIGNUP_GOLD`]), the steps' experience,
/// and the ammo ([`SIGNUP_AMMO`]) onto the stacks it owns.
pub fn signup_grant(account: &mut Account) {
    account.generic_money = account.generic_money.max(SIGNUP_SILVER);
    account.premium_money = account.premium_money.max(SIGNUP_GOLD);
    // Short of a level: nothing more to pay.
    account.add_experience(experience());
    let dictionary = Dictionary::shared();
    for (name, rounds) in SIGNUP_AMMO {
        let item = dictionary
            .item_named(name)
            .expect("the starter ammo exists");
        account.add_to_stack(item.dict_id, rounds);
    }
    account.extras.tutorial_flags = ALL_DONE;
    account.tutorial_done = true;
}

/// `9D`: every step done (see the module). `show` on at sign-in, off for
/// the ranking's recounts.
pub fn message(show: bool) -> ServerMessage {
    lobby::tutorial(show, ALL_DONE, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use survarium::v026g::lobby::client_model::{Client, Decoded};
    use survarium::v026g::lobby::tutorial_model::State;

    #[test]
    fn the_client_runs_no_tutorial() {
        let message = message(true);
        assert_eq!(message.id_byte(), 0x9D);
        assert_eq!(message.body(), [1, 0x3F, 0, 0, 0, 1, 0, 0, 0]);
        let mut client = Client::signed_in();
        assert!(matches!(
            client.receive(&message).unwrap(),
            Decoded::Tutorial {
                show: true,
                a: 0x3F,
                b: 1
            }
        ));
        assert_eq!(client.tutorial().state(), State::Default);
        assert!(!client.tutorial().is_modal());
        assert_eq!(super::message(false).body()[0], 0);
    }

    #[test]
    fn a_new_account_starts_with_its_balance_the_experience_and_ammo() {
        assert_eq!(experience(), 250);
        let mut account = Account::new("Alice");
        signup_grant(&mut account);
        assert_eq!(
            (account.generic_money, account.premium_money),
            (10_000, 500)
        );
        assert_eq!(account.exp_total, 250);
        assert_eq!(account.level(), 0);
        assert_eq!(account.extras.tutorial_flags, ALL_DONE);
        let dictionary = Dictionary::shared();
        for (name, rounds) in SIGNUP_AMMO {
            let dict_id = dictionary.item_named(name).unwrap().dict_id;
            let stacks: Vec<u32> = account
                .inventory
                .iter()
                .filter(|item| item.dict_id == dict_id)
                .map(|item| item.condition_or_stack)
                .collect();
            // One stack: the clip it had and the rounds given.
            assert_eq!(stacks.len(), 1, "{name}");
            assert!(stacks[0] > rounds, "{name}: {stacks:?}");
        }
    }
}
