//! The premium account: the store's offers, buying one, and what it gives
//! (`../srp-021d/docs/notes/premium.md`).
//!
//! The client shows the offers (`36 11`), lets a player pick one they have
//! the gold for, and sends `24 02 <offer id>`; it learns the time left from
//! `36 0F`. The 0.21d prices are lost: srp's are a choice.

use super::account::Account;

/// One offer of the premium store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offer {
    /// Sent back by the client in `24 02`.
    pub id: u32,
    pub hours: u32,
    /// In gold (premium money).
    pub price: u32,
    /// Localization keys of its name and its duration line.
    pub label: &'static str,
    pub description: &'static str,
}

/// The durations the localization names [V], at srp's prices [a choice]. It
/// also describes 72 hours (`st_premium_3_day_descr`) but has no name for
/// them, so srp leaves that one out.
pub const OFFERS: [Offer; 6] = [
    Offer {
        id: 1,
        hours: 24,
        price: 250,
        label: "st_premium_day",
        description: "st_premium_day_descr",
    },
    Offer {
        id: 2,
        hours: 7 * 24,
        price: 1_200,
        label: "st_premium_week",
        description: "st_premium_week_descr",
    },
    Offer {
        id: 3,
        hours: 30 * 24,
        price: 3_000,
        label: "st_premium_month",
        description: "st_premium_month_descr",
    },
    Offer {
        id: 4,
        hours: 90 * 24,
        price: 7_500,
        label: "st_premium_3_month",
        description: "st_premium_3_month_descr",
    },
    Offer {
        id: 5,
        hours: 180 * 24,
        price: 13_500,
        label: "st_premium_6_month",
        description: "st_premium_6_month_descr",
    },
    Offer {
        id: 6,
        hours: 365 * 24,
        price: 24_000,
        label: "st_premium_year",
        description: "st_premium_year_descr",
    },
];

/// What premium multiplies a match's experience, money and reputation by,
/// as a fraction: the results screen offers non-premium players "(+X)" with
/// X = value × 1.5 − value [V `updatePremiumState`].
pub const BONUS: (u32, u32) = (3, 2);

pub fn offer(id: u32) -> Option<&'static Offer> {
    OFFERS.iter().find(|offer| offer.id == id)
}

/// Whether `account` has premium at `now` (Unix seconds).
pub fn is_active(account: &Account, now: u64) -> bool {
    account.premium_until > now
}

/// The whole hours of premium left, rounded up; 0 without.
pub fn hours_left(account: &Account, now: u64) -> u32 {
    account
        .premium_until
        .saturating_sub(now)
        .div_ceil(3600)
        .min(u32::MAX.into()) as u32
}

/// `value` with the premium bonus.
pub fn with_bonus(value: u32) -> u32 {
    (u64::from(value) * u64::from(BONUS.0) / u64::from(BONUS.1)).min(u32::MAX.into()) as u32
}

/// Why a purchase failed: the client's own texts.
pub const CANNOT_BUY: &str = "st_cannot_buy_item";
pub const NOT_ENOUGH_MONEY: &str = "st_not_enougth_money";

/// Buy offer `id` at `now`: its gold is taken and its hours are added to
/// what is left, or start now.
pub fn buy(account: &mut Account, id: u32, now: u64) -> Result<&'static Offer, &'static str> {
    let offer = offer(id).ok_or(CANNOT_BUY)?;
    if account.premium_money < offer.price {
        return Err(NOT_ENOUGH_MONEY);
    }
    account.premium_money -= offer.price;
    account.premium_until = account.premium_until.max(now) + u64::from(offer.hours) * 3600;
    Ok(offer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::Dictionary;

    fn account() -> Account {
        Account::new("tester", Dictionary::shared())
    }

    #[test]
    fn buying_takes_the_gold_and_adds_the_hours() {
        let mut account = account();
        account.premium_money = 5_000;
        let now = 1_000_000;
        assert!(!is_active(&account, now));
        assert_eq!(buy(&mut account, 1, now), Ok(&OFFERS[0]));
        assert_eq!(account.premium_money, 4_750);
        assert!(is_active(&account, now));
        assert_eq!(hours_left(&account, now), 24);
        // A second purchase adds to what is left.
        buy(&mut account, 2, now + 3600).unwrap();
        assert_eq!(hours_left(&account, now + 3600), 23 + 7 * 24);
        assert_eq!(hours_left(&account, now + 3600 + 1), 23 + 7 * 24);
        // And after it ran out, starts again from then.
        let later = account.premium_until + 100;
        assert!(!is_active(&account, later));
        assert_eq!(hours_left(&account, later), 0);
        buy(&mut account, 1, later).unwrap();
        assert_eq!(account.premium_until, later + 24 * 3600);
    }

    #[test]
    fn a_purchase_needs_the_gold_and_a_real_offer() {
        let mut account = account();
        account.premium_money = 249;
        assert_eq!(buy(&mut account, 1, 0), Err(NOT_ENOUGH_MONEY));
        assert_eq!(buy(&mut account, 99, 0), Err(CANNOT_BUY));
        assert_eq!(account.premium_money, 249);
        assert_eq!(account.premium_until, 0);
    }

    #[test]
    fn the_bonus_is_half_again() {
        assert_eq!(with_bonus(100), 150);
        assert_eq!(with_bonus(45), 67);
        assert_eq!(with_bonus(0), 0);
    }
}
