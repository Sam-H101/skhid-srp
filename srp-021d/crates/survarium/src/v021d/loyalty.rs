//! Faction loyalty (`docs/notes/loyalty.md`): a bonus to all the
//! reputation earned with a faction, raised by doing its daily quests.
//!
//! The 0.21d client only shows it: `36 0F` types 1..=4 put "+N%" by each
//! faction's reputation bar in the faction shop. The rules are the 0.20a
//! announcement's: each completed quest of the faction adds 5 %, up to 25 %;
//! changing a quest takes 5 % off; a failed quest (one that ran out) resets
//! it.

use super::account::Account;

/// What one completed quest adds and one changed quest takes off, in %.
pub const STEP: u8 = 5;
/// The most loyalty gives, in %.
pub const MAX: u8 = 25;

fn slot(account: &mut Account, faction: u8) -> Option<&mut u8> {
    account
        .loyalty
        .get_mut(usize::from(faction).checked_sub(1)?)
}

/// The account's loyalty with `faction` (1..=4), in %; 0 for any other.
pub fn of(account: &Account, faction: u8) -> u8 {
    usize::from(faction)
        .checked_sub(1)
        .and_then(|index| account.loyalty.get(index).copied())
        .unwrap_or(0)
}

/// `amount` of reputation with `faction`, with the account's loyalty to it.
pub fn with_bonus(account: &Account, faction: u8, amount: u32) -> u32 {
    let bonus = u64::from(amount) * u64::from(of(account, faction)) / 100;
    amount.saturating_add(bonus.min(u32::MAX.into()) as u32)
}

/// A quest of `faction` was done.
pub fn raise(account: &mut Account, faction: u8) {
    if let Some(loyalty) = slot(account, faction) {
        *loyalty = loyalty.saturating_add(STEP).min(MAX);
    }
}

/// A quest of `faction` was changed for another.
pub fn lower(account: &mut Account, faction: u8) {
    if let Some(loyalty) = slot(account, faction) {
        *loyalty = loyalty.saturating_sub(STEP);
    }
}

/// A quest of `faction` failed.
pub fn reset(account: &mut Account, faction: u8) {
    if let Some(loyalty) = slot(account, faction) {
        *loyalty = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::Dictionary;

    #[test]
    fn loyalty_climbs_by_five_to_twenty_five() {
        let mut account = Account::new("tester", Dictionary::shared());
        assert_eq!(of(&account, 1), 0);
        for _ in 0..7 {
            raise(&mut account, 1);
        }
        assert_eq!(of(&account, 1), MAX);
        assert_eq!(of(&account, 2), 0);
        lower(&mut account, 1);
        assert_eq!(of(&account, 1), 20);
        reset(&mut account, 1);
        assert_eq!(of(&account, 1), 0);
        lower(&mut account, 1);
        assert_eq!(of(&account, 1), 0);
        // No such faction.
        raise(&mut account, 0);
        raise(&mut account, 9);
        assert_eq!(of(&account, 0), 0);
        assert_eq!(of(&account, 9), 0);
    }

    #[test]
    fn the_bonus_adds_its_percent() {
        let mut account = Account::new("tester", Dictionary::shared());
        assert_eq!(with_bonus(&account, 3, 200), 200);
        raise(&mut account, 3);
        raise(&mut account, 3);
        assert_eq!(with_bonus(&account, 3, 200), 220);
        assert_eq!(with_bonus(&account, 3, 15), 16);
        assert_eq!(with_bonus(&account, 4, 200), 200);
    }
}
