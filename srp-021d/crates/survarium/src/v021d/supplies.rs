//! Ammo, drugs, traps and grenades over a match
//! (`docs/notes/supplies.md`).
//!
//! The 0.21d client keeps a pool per profile slot for the whole match: the
//! slot's `amount` as the match start (0x51) gives it. Each life takes at
//! most the slot's `condition_or_stack` out of the pool
//! (`inventory::setup_from_profile` 0x1FADB6), and a respawn puts back what
//! the life left, the magazine and a chambered round included
//! (`inventory::unload_to_profile` 0x1FAF4C). So a match can use at most the
//! pools, and srp sets them to what the account owns. Each player's pools
//! and what is in hand are in the world snapshots (`base_player::serialize`
//! 0x1D5E96), which tells srp what was used.

use std::collections::BTreeMap;

use super::account::{Account, Profile, slot_carries_amount};
use super::item_params::shop_unit;
use super::results::reputation_level;
use crate::dictionary::Dictionary;
use crate::prices;

/// What one profile slot carries into a match, in the order the client
/// serializes the player's items (ascending slot; armour, slots 0..=6, is
/// never written).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Carried {
    pub slot: u8,
    pub dict_id: u16,
    /// The slot's `condition_or_stack`: for a stack, what one life takes.
    pub count: u16,
    /// The slot's pool for the match (`amount`); 0 for slots without one
    /// (`account::slot_carries_amount`).
    pub pool: u32,
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Weapon slots 7 and 10. `chambered`: its config has
    /// `chamber_a_round_time`, so its state carries a chambered-round bool.
    Weapon { chambered: bool },
    /// Drugs (category 10): their state adds an "active" bool, and more
    /// when active.
    Drug,
    /// Any other stack: ammo, traps, grenades.
    Stack,
    /// Anything else written as a bare item header.
    Other,
}

/// A player's supplies at a match's start.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Loadout {
    /// Slots 7..=22 holding an item, ascending.
    pub carried: Vec<Carried>,
    /// What the account owned of each stack item carried, by dict id: the
    /// pools add up to it.
    pub owned: BTreeMap<u16, u32>,
}

/// Weapons whose config has `chamber_a_round_time` (0.21d items): the
/// Mosin-Nagant, the Remington 700 and 870, the TOZ-122 and the Winchester
/// 1894.
pub const CHAMBERED: [u16; 5] = [107, 14, 15, 12, 90];

/// Drugs' item category.
const DRUG_CATEGORY: u8 = 10;
/// The weapon slots.
const WEAPON_SLOTS: [usize; 2] = [7, 10];
/// The slots a match's supplies come from: the weapons and what follows
/// them; armour (0..=6) is never written.
const CARRIED_SLOTS: std::ops::RangeInclusive<usize> = 7..=22;

/// `profile` as a match takes it: every slot that carries an amount
/// (`account::slot_carries_amount`) and holds a stack gets its pool from the
/// stack `account` owns now. The slots holding one item share it out in
/// ascending order, each its `condition_or_stack` at most, and the last of
/// them gets the rest: the pools add up to the stack and no round counts
/// twice [V: a pool never moves between slots]. A stack no longer owned
/// gives pools of 0.
pub fn match_profile(account: &Account, profile: &Profile) -> Profile {
    let dictionary = Dictionary::shared();
    let mut profile = profile.clone();
    // `(slot, item_id)` of the slots with a pool, ascending.
    let pooled: Vec<(usize, u32)> = profile
        .slots
        .iter()
        .enumerate()
        .filter_map(|(slot, item)| {
            let item = item.as_ref()?;
            let is_stack = dictionary
                .item(item.dict_id)
                .is_some_and(|entry| entry.is_stack);
            (is_stack && slot_carries_amount(slot)).then_some((slot, item.item_id))
        })
        .collect();
    let mut left: BTreeMap<u32, u32> = BTreeMap::new();
    for (index, &(slot, item_id)) in pooled.iter().enumerate() {
        let rest = left.entry(item_id).or_insert_with(|| {
            account
                .inventory_item(item_id)
                .map_or(0, |owned| owned.condition_or_stack.into())
        });
        let last = !pooled[index + 1..].iter().any(|&(_, id)| id == item_id);
        let item = profile.slots[slot].as_mut().expect("a pooled slot");
        let pool = if last {
            *rest
        } else {
            u32::from(item.condition_or_stack).min(*rest)
        };
        *rest -= pool;
        item.amount = pool;
    }
    profile
}

/// The supplies of a match profile ([`match_profile`]).
pub fn loadout(dictionary: &Dictionary, profile: &Profile) -> Loadout {
    let mut loadout = Loadout::default();
    for slot in CARRIED_SLOTS {
        let Some(item) = profile.slots.get(slot).and_then(Option::as_ref) else {
            continue;
        };
        let entry = dictionary.item(item.dict_id);
        let is_stack = entry.is_some_and(|entry| entry.is_stack);
        let kind = if WEAPON_SLOTS.contains(&slot) {
            Kind::Weapon {
                chambered: CHAMBERED.contains(&item.dict_id),
            }
        } else if is_stack && entry.is_some_and(|entry| entry.category == DRUG_CATEGORY) {
            Kind::Drug
        } else if is_stack {
            Kind::Stack
        } else {
            Kind::Other
        };
        let pool = if slot_carries_amount(slot) {
            item.amount
        } else {
            0
        };
        if is_stack && !matches!(kind, Kind::Weapon { .. }) {
            *loadout.owned.entry(item.dict_id).or_default() += pool;
        }
        loadout.carried.push(Carried {
            slot: slot as u8,
            dict_id: item.dict_id,
            count: item.condition_or_stack,
            pool,
            kind,
        });
    }
    loadout
}

/// What a match's supplies did to an account ([`settle`]), by dict id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settled {
    /// Rounds (or items) used up.
    pub used: Vec<(u16, u32)>,
    /// Rounds bought back by autobuy.
    pub bought: Vec<(u16, u32)>,
    /// What those cost, in silver.
    pub cost: u32,
}

/// Take what a match used off the account's stacks: `owned` as the match
/// started ([`Loadout::owned`]), `left` as it ended (a dict id missing from
/// it used nothing). With the profile's `autobuy`, what was used is bought
/// back first, in whole clips (`item_params::shop_unit`) from a seller whose
/// rank the account has (as the shop's `24 00` from any seller), as many as
/// the silver pays for. A stack used up leaves the inventory and its slots.
pub fn settle(
    account: &mut Account,
    profile_id: u32,
    owned: &BTreeMap<u16, u32>,
    left: &BTreeMap<u16, u32>,
) -> Settled {
    let autobuy = account
        .profile(profile_id)
        .is_some_and(|profile| profile.autobuy);
    let mut settled = Settled::default();
    for (&dict_id, &had) in owned {
        let used = had.saturating_sub(left.get(&dict_id).copied().unwrap_or(had));
        if used == 0 {
            continue;
        }
        settled.used.push((dict_id, used));
        let (bought, cost) = if autobuy {
            rebuy(account, dict_id, used)
        } else {
            (0, 0)
        };
        if bought > 0 {
            settled.bought.push((dict_id, bought));
            settled.cost += cost;
        }
        let Some(index) = account
            .inventory
            .iter()
            .position(|item| item.dict_id == dict_id)
        else {
            if bought > 0 {
                let count = u16::try_from(bought).unwrap_or(u16::MAX);
                account.add_item(dict_id, count, true);
            }
            continue;
        };
        let stack = account.inventory[index].condition_or_stack;
        let now = u32::from(stack).saturating_sub(used) + bought;
        if now == 0 {
            account.inventory.remove(index);
        } else {
            let item = &mut account.inventory[index];
            item.condition_or_stack = u16::try_from(now).unwrap_or(u16::MAX);
            item.amount = item.condition_or_stack.into();
        }
    }
    account.sync_stacks();
    settled
}

/// Autobuy of `used` rounds of `dict_id`: whole clips, as many as the
/// account's silver pays for, from a seller whose rank it has. Takes the
/// silver; returns the rounds bought and their cost.
fn rebuy(account: &mut Account, dict_id: u16, used: u32) -> (u32, u32) {
    let Some(price) = prices::price(dict_id) else {
        return (0, 0);
    };
    let sold = price.sellers.iter().any(|&seller| {
        let points = account
            .reputations
            .iter()
            .find(|&&(id, _)| id == seller)
            .map_or(0, |&(_, points)| points);
        reputation_level(seller, points.into()) >= prices::rank(dict_id, seller)
    });
    if !sold {
        return (0, 0);
    }
    let unit = shop_unit(dict_id);
    let clip_cost = u32::from(price.cost) * unit;
    let clips = used
        .div_ceil(unit)
        .min(account.generic_money / clip_cost.max(1));
    let cost = clips * clip_cost;
    account.generic_money -= cost;
    (clips * unit, cost)
}

#[cfg(test)]
mod tests {
    use super::super::account::{Item, STARTING_MONEY};
    use super::*;

    /// The game's test loadout: the APS (7) with 9x18 in 8 and 9x18 PBM in
    /// 9, the SPAS-12 (10) with buckshot in 11 and slugs in 12, 50 rounds
    /// of each.
    fn account() -> Account {
        Account::with_test_loadout("Player", Dictionary::shared())
    }

    fn stack(account: &Account, dict_id: u16) -> Option<u16> {
        account
            .inventory
            .iter()
            .find(|item| item.dict_id == dict_id)
            .map(|item| item.condition_or_stack)
    }

    #[test]
    fn a_stack_in_two_slots_is_shared_out() {
        let dictionary = Dictionary::shared();
        let mut account = account();
        // Buckshot in slot 12 too, for the slugs: 30 a life in 11 and 40
        // in 12; 50 owned.
        let mut buck = account.profiles[0].slots[11].clone().unwrap();
        assert!(dictionary.fits(buck.dict_id, 12));
        account.profiles[0].slots[11]
            .as_mut()
            .unwrap()
            .condition_or_stack = 30;
        buck.condition_or_stack = 40;
        account.profiles[0].slots[12] = Some(buck);
        let profile = match_profile(&account, &account.profiles[0]);
        let pool = |slot: usize| profile.slots[slot].as_ref().unwrap().amount;
        assert_eq!((pool(11), pool(12)), (30, 20));
        // Alone in their slots: the whole stack.
        assert_eq!((pool(8), pool(9)), (50, 50));
        // The weapons keep theirs.
        assert_eq!(pool(7), 1);

        // A stack no longer owned has no pool.
        account.inventory.retain(|item| item.dict_id != 103);
        let profile = match_profile(&account, &account.profiles[0]);
        assert_eq!(profile.slots[9].as_ref().unwrap().amount, 0);
    }

    #[test]
    fn the_loadout_of_a_match_profile() {
        let dictionary = Dictionary::shared();
        let mut account = account();
        let medkit = account.add_item(67, 3, true);
        let slot = (13..=22)
            .find(|&slot| dictionary.fits(67, slot))
            .expect("a slot for the medkit");
        account.profiles[0].slots[usize::from(slot)] = Some(Item {
            condition_or_stack: 1,
            ..medkit
        });
        let profile = match_profile(&account, &account.profiles[0]);
        let loadout = loadout(dictionary, &profile);
        let carried: Vec<(u8, u16, u16, u32, Kind)> = loadout
            .carried
            .iter()
            .map(|c| (c.slot, c.dict_id, c.count, c.pool, c.kind))
            .collect();
        assert_eq!(
            carried,
            [
                (7, 126, 100, 0, Kind::Weapon { chambered: false }),
                (8, 70, 50, 50, Kind::Stack),
                (9, 103, 50, 50, Kind::Stack),
                (10, 124, 100, 0, Kind::Weapon { chambered: false }),
                (11, 22, 50, 50, Kind::Stack),
                (12, 72, 50, 50, Kind::Stack),
                (slot, 67, 1, 3, Kind::Drug),
            ]
        );
        assert_eq!(
            loadout.owned,
            BTreeMap::from([(22, 50), (67, 3), (70, 50), (72, 50), (103, 50)])
        );
    }

    #[test]
    fn what_was_used_is_taken_off() {
        let mut account = account();
        let owned = BTreeMap::from([(22, 50), (72, 50)]);
        // Slugs missing: none used.
        let left = BTreeMap::from([(22, 15)]);
        let money = account.generic_money;
        let settled = settle(&mut account, 1, &owned, &left);
        assert_eq!(settled.used, [(22, 35)]);
        assert_eq!((settled.bought, settled.cost), (vec![], 0));
        assert_eq!(account.generic_money, money);
        assert_eq!(
            (stack(&account, 22), stack(&account, 72)),
            (Some(15), Some(50))
        );
        // The slot keeps its per-life count, its pool follows the stack.
        let slot = account.profiles[0].slots[11].as_ref().unwrap();
        assert_eq!((slot.condition_or_stack, slot.amount), (50, 15));
    }

    #[test]
    fn autobuy_buys_back_in_clips_within_the_money() {
        let mut account = account();
        account.profiles[0].autobuy = true;
        let owned = BTreeMap::from([(22, 50)]);
        // 35 used: two clips of 20 at 1 a round.
        let settled = settle(&mut account, 1, &owned, &BTreeMap::from([(22, 15)]));
        assert_eq!(settled.used, [(22, 35)]);
        assert_eq!(settled.bought, [(22, 40)]);
        assert_eq!(settled.cost, 40);
        assert_eq!(account.generic_money, STARTING_MONEY - 40);
        assert_eq!(stack(&account, 22), Some(55));

        // Money for one clip only.
        account.generic_money = 25;
        let owned = BTreeMap::from([(22, 55)]);
        let settled = settle(&mut account, 1, &owned, &BTreeMap::from([(22, 0)]));
        assert_eq!(settled.bought, [(22, 20)]);
        assert_eq!(account.generic_money, 5);
        assert_eq!(stack(&account, 22), Some(20));
    }

    #[test]
    fn a_stack_used_up_leaves_the_inventory_and_its_slot() {
        let mut account = account();
        let revision = account.profiles[0].revision;
        let owned = BTreeMap::from([(72, 50)]);
        let settled = settle(&mut account, 1, &owned, &BTreeMap::from([(72, 0)]));
        assert_eq!(settled.used, [(72, 50)]);
        assert_eq!(stack(&account, 72), None);
        assert_eq!(account.profiles[0].slots[12], None);
        assert_eq!(account.profiles[0].revision, revision + 1);
    }

    #[test]
    fn no_rebuy_without_the_rank_or_a_price() {
        let mut account = account();
        account.profiles[0].autobuy = true;
        let money = account.generic_money;
        // The RGD-5 needs rank 3 with either seller.
        account.add_item(5, 3, true);
        // The Leupold scope has no price.
        account.add_item(69, 1, true);
        let owned = BTreeMap::from([(5, 3), (69, 1)]);
        let left = BTreeMap::from([(5, 1), (69, 0)]);
        let settled = settle(&mut account, 1, &owned, &left);
        assert_eq!(settled.used, [(5, 2), (69, 1)]);
        assert!(settled.bought.is_empty());
        assert_eq!(account.generic_money, money);
        assert_eq!((stack(&account, 5), stack(&account, 69)), (Some(1), None));
    }
}
