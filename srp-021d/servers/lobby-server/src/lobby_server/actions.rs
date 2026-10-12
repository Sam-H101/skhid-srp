//! What the 0.21d client's lobby actions do to its account. Each checks the
//! rules the client also checks (`lobby_spec.md` §7.2) and changes nothing
//! when refused; a refusal is the localization key the client shows.

use survarium::dictionary::{Dictionary, PROFILE_SLOTS};
use survarium::prices;
use survarium::v021d::account::{Account, Item};
use survarium::v021d::item_params::shop_unit;
use survarium::v021d::results::reputation_level;

/// The slot id that means "the inventory" in a move.
pub const INVENTORY_SLOT: u8 = 100;
/// Most points one skill takes [spec: the skills tree has 20 levels].
pub const MAX_SKILL_POINTS: u8 = 20;
/// Most items one purchase of a non-stack item makes [a choice].
pub const MAX_BOUGHT_AT_ONCE: u32 = 8;
/// What a sold item brings back: half its price [a choice; no source].
pub const SELL_BACK_PERCENT: u64 = 50;

pub const CANNOT_RELOCATE: &str = "st_cannot_relocate_item";
pub const CANNOT_BUY: &str = "st_cannot_buy_item";
pub const NOT_ENOUGH_MONEY: &str = "st_not_enougth_money";
pub const NOT_ENOUGH_MONEY_FOR_REROLL: &str = "st_not_enougth_money_for_reroll";

/// One entry of `0x23`: `relocate_item_descr`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relocation {
    pub item_id: u32,
    pub dict_id: u16,
    pub source_slot: u8,
    pub target_slot: u8,
    pub count: u32,
    pub count_in_inventory: u32,
}

fn slot_index(slot: u8) -> Option<usize> {
    let slot = usize::from(slot);
    (slot < PROFILE_SLOTS).then_some(slot)
}

/// `0x23`: move items between a profile's slots and the inventory, all of
/// them or none.
///
/// The ammo dialog sets a stack's rounds in the slot and in storage, shows
/// the price of what that is beyond the stack owned, and sends the total as
/// `count_in_inventory` (item id 0 for a stack not owned at all). The exe
/// sends no purchase: the missing clips are bought here, as `24 00` would buy
/// them from any seller, before the move.
pub fn move_items(
    account: &mut Account,
    dictionary: &Dictionary,
    profile_id: u32,
    relocations: &[Relocation],
) -> Result<(), &'static str> {
    let mut changed = account.clone();
    let mut resolved = Vec::with_capacity(relocations.len());
    for &relocation in relocations {
        let mut relocation = relocation;
        let is_stack = dictionary
            .item(relocation.dict_id)
            .is_some_and(|item| item.is_stack);
        if is_stack && relocation.target_slot != INVENTORY_SLOT {
            let have = owned_stack(&changed, &relocation)
                .map_or(0, |item| u32::from(item.condition_or_stack));
            let wanted = relocation.count_in_inventory.max(relocation.count);
            if wanted > have {
                let clips = (wanted - have).div_ceil(shop_unit(relocation.dict_id));
                buy(&mut changed, dictionary, relocation.dict_id, clips, 0)?;
            }
            relocation.item_id = owned_stack(&changed, &relocation)
                .ok_or(CANNOT_RELOCATE)?
                .item_id;
        }
        resolved.push(relocation);
    }
    relocate(&mut changed, dictionary, profile_id, &resolved)?;
    changed.sync_stacks();
    *account = changed;
    Ok(())
}

/// The owned stack a relocation names: by its id, or by its item for id 0.
fn owned_stack<'a>(account: &'a Account, relocation: &Relocation) -> Option<&'a Item> {
    account.inventory.iter().find(|item| {
        item.dict_id == relocation.dict_id
            && (relocation.item_id == 0 || item.item_id == relocation.item_id)
    })
}

/// [`move_items`] with every item owned.
fn relocate(
    account: &mut Account,
    dictionary: &Dictionary,
    profile_id: u32,
    relocations: &[Relocation],
) -> Result<(), &'static str> {
    let inventory = account.inventory.clone();
    let profile = account
        .profiles
        .iter_mut()
        .find(|profile| profile.id == profile_id)
        .ok_or(CANNOT_RELOCATE)?;
    let mut slots = profile.slots.clone();

    for relocation in relocations {
        let owned = inventory
            .iter()
            .find(|item| item.item_id == relocation.item_id && item.dict_id == relocation.dict_id)
            .ok_or(CANNOT_RELOCATE)?;
        let is_stack = dictionary
            .item(owned.dict_id)
            .ok_or(CANNOT_RELOCATE)?
            .is_stack;

        if relocation.source_slot != INVENTORY_SLOT {
            let source = slot_index(relocation.source_slot).ok_or(CANNOT_RELOCATE)?;
            if slots[source].as_ref().map(|item| item.item_id) != Some(owned.item_id) {
                return Err(CANNOT_RELOCATE);
            }
            slots[source] = None;
        }

        if relocation.target_slot != INVENTORY_SLOT {
            let target = slot_index(relocation.target_slot).ok_or(CANNOT_RELOCATE)?;
            if !dictionary.fits(owned.dict_id, relocation.target_slot) {
                return Err(CANNOT_RELOCATE);
            }
            let mut placed = owned.clone();
            if is_stack {
                // The rounds put in the slot, at most what is owned. None
                // empties it: the client's ammo dialog takes a stack out of
                // a slot by moving it back in with 0 rounds.
                let count = u16::try_from(relocation.count).unwrap_or(u16::MAX);
                if count == 0 {
                    continue;
                }
                placed.condition_or_stack = count.min(owned.condition_or_stack);
            } else {
                // One item is in one slot.
                for slot in slots.iter_mut() {
                    if slot
                        .as_ref()
                        .is_some_and(|item| item.item_id == owned.item_id)
                    {
                        *slot = None;
                    }
                }
            }
            slots[target] = Some(placed);
        }
    }

    if slots != profile.slots {
        profile.slots = slots;
        profile.revision = profile.revision.wrapping_add(1);
    }
    Ok(())
}

/// `0x24 00`: buy `count` of an item (clips for ammo, `shop_unit`) from
/// `faction`, 0 meaning any seller. Returns what `34 24 00` lists, in rounds.
pub fn buy(
    account: &mut Account,
    dictionary: &Dictionary,
    dict_id: u16,
    count: u32,
    faction: u8,
) -> Result<Vec<(u16, u32, u32)>, &'static str> {
    let price = prices::price(dict_id).ok_or(CANNOT_BUY)?;
    // A seller that sells it at this account's rank with it.
    let sells = |seller: u8| {
        let points = account
            .reputations
            .iter()
            .find(|&&(id, _)| id == seller)
            .map_or(0, |&(_, points)| points);
        reputation_level(seller, points.into()) >= prices::rank(dict_id, seller)
    };
    let sold = if faction == 0 {
        price.sellers.iter().any(|&seller| sells(seller))
    } else {
        price.sellers.contains(&faction) && sells(faction)
    };
    if !sold {
        return Err(CANNOT_BUY);
    }
    let item = dictionary.item(dict_id).ok_or(CANNOT_BUY)?;
    if count == 0 || (!item.is_stack && count > MAX_BOUGHT_AT_ONCE) {
        return Err(CANNOT_BUY);
    }
    let units = u64::from(count) * u64::from(shop_unit(dict_id));
    // The price is for a round of ammo (see `prices::Price::cost`).
    let cost = u64::from(price.cost) * units;
    if cost > u64::from(account.generic_money) {
        return Err(NOT_ENOUGH_MONEY);
    }

    let bought = if item.is_stack {
        let added = u16::try_from(units).map_err(|_| CANNOT_BUY)?;
        match account
            .inventory
            .iter_mut()
            .find(|owned| owned.dict_id == dict_id)
        {
            Some(owned) => {
                owned.condition_or_stack = owned
                    .condition_or_stack
                    .checked_add(added)
                    .ok_or(CANNOT_BUY)?;
                owned.amount = owned.condition_or_stack.into();
                vec![(dict_id, owned.item_id, added.into())]
            }
            None => {
                let new = account.add_item(dict_id, added, true);
                vec![(dict_id, new.item_id, added.into())]
            }
        }
    } else {
        (0..count)
            .map(|_| {
                let new =
                    account.add_item(dict_id, survarium::v021d::account::FULL_CONDITION, false);
                (dict_id, new.item_id, u32::from(new.condition_or_stack))
            })
            .collect()
    };
    account.generic_money -= cost as u32;
    // The slots holding the stack carry its new size.
    account.sync_stacks();
    Ok(bought)
}

/// `0x24 01`: sell `amount` of an item (clips for ammo, `shop_unit`; all of
/// a non-stack item). Returns the amount sold, in rounds, and the profiles
/// whose slots changed.
pub fn sell(
    account: &mut Account,
    dictionary: &Dictionary,
    item_id: u32,
    amount: u32,
) -> Result<(u32, Vec<u32>), &'static str> {
    let index = account
        .inventory
        .iter()
        .position(|item| item.item_id == item_id)
        .ok_or(CANNOT_RELOCATE)?;
    let owned = account.inventory[index].clone();
    let is_stack = dictionary
        .item(owned.dict_id)
        .ok_or(CANNOT_RELOCATE)?
        .is_stack;

    let (sold, left) = if is_stack {
        let rounds = u64::from(amount) * u64::from(shop_unit(owned.dict_id));
        let sold = rounds.min(owned.condition_or_stack.into()) as u32;
        if sold == 0 {
            return Err(CANNOT_RELOCATE);
        }
        (sold, owned.condition_or_stack - sold as u16)
    } else {
        (1, 0)
    };

    if left == 0 {
        account.inventory.remove(index);
    } else {
        let item = &mut account.inventory[index];
        item.condition_or_stack = left;
        item.amount = left.into();
    }

    let mut changed = Vec::new();
    for profile in &mut account.profiles {
        let mut touched = false;
        for slot in profile.slots.iter_mut() {
            let Some(item) = slot.as_mut().filter(|item| item.item_id == item_id) else {
                continue;
            };
            touched = true;
            if left == 0 {
                *slot = None;
            } else {
                item.condition_or_stack = item.condition_or_stack.min(left);
                item.amount = left.into();
            }
        }
        if touched {
            profile.revision = profile.revision.wrapping_add(1);
            changed.push(profile.id);
        }
    }

    if let Some(price) = prices::price(owned.dict_id) {
        // Per round for ammo, as bought.
        let back = u64::from(price.cost) * u64::from(sold) * SELL_BACK_PERCENT / 100;
        account.generic_money = account.generic_money.saturating_add(back as u32);
    }
    Ok((sold, changed))
}

/// `0x25 00`: a profile's skills and perks. Points at most the profile's
/// level in all, each skill at most [`MAX_SKILL_POINTS`]; only ids the
/// client's dictionary has.
pub fn set_skills(
    account: &mut Account,
    dictionary: &Dictionary,
    profile_id: u32,
    skills: &[(u8, u8)],
    perks: &[u8],
) -> Result<(), &'static str> {
    let profile = account
        .profiles
        .iter_mut()
        .find(|profile| profile.id == profile_id)
        .ok_or(CANNOT_RELOCATE)?;
    let total: u32 = skills.iter().map(|&(_, points)| u32::from(points)).sum();
    let valid = total <= u32::from(profile.level)
        && skills
            .iter()
            .all(|&(id, points)| dictionary.skills.contains(&id) && points <= MAX_SKILL_POINTS)
        && perks.iter().all(|perk| dictionary.perks.contains(perk));
    if !valid {
        return Err(CANNOT_RELOCATE);
    }
    if profile.skills != skills || profile.perks != perks {
        profile.skills = skills.to_vec();
        profile.perks = perks.to_vec();
        // Its modifiers come with its contents.
        profile.revision = profile.revision.wrapping_add(1);
    }
    Ok(())
}

/// `0x25 01`: clear a profile's skills for `cost`.
pub fn reroll_skills(
    account: &mut Account,
    profile_id: u32,
    cost: u32,
) -> Result<(), &'static str> {
    if account.generic_money < cost {
        return Err(NOT_ENOUGH_MONEY_FOR_REROLL);
    }
    let profile = account
        .profiles
        .iter_mut()
        .find(|profile| profile.id == profile_id)
        .ok_or(CANNOT_RELOCATE)?;
    profile.skills.clear();
    profile.perks.clear();
    profile.revision = profile.revision.wrapping_add(1);
    account.generic_money -= cost;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        Account::with_test_loadout("Player", Dictionary::shared())
    }

    fn slot_item(account: &Account, slot: usize) -> Option<Item> {
        account.profiles[0].slots[slot].clone()
    }

    /// The client's ammo dialog: the stack goes to the inventory and comes
    /// back with the rounds now wanted in the slot; 0 leaves it empty.
    #[test]
    fn an_ammo_slot_set_to_no_rounds_is_emptied() {
        let dictionary = Dictionary::shared();
        let mut account = account();
        let pbm = slot_item(&account, 9).unwrap();
        let relocation = |source_slot, target_slot, count| Relocation {
            item_id: pbm.item_id,
            dict_id: pbm.dict_id,
            source_slot,
            target_slot,
            count,
            count_in_inventory: 0,
        };
        let relocations = [relocation(9, 100, 50), relocation(100, 9, 0)];
        move_items(&mut account, dictionary, 1, &relocations).unwrap();
        assert_eq!(slot_item(&account, 9), None);
        // Still owned.
        assert!(account.inventory_item(pbm.item_id).is_some());

        // And back with 20 of them.
        move_items(&mut account, dictionary, 1, &[relocation(100, 9, 20)]).unwrap();
        assert_eq!(slot_item(&account, 9).unwrap().condition_or_stack, 20);
    }

    #[test]
    fn the_ammo_dialog_buys_what_is_missing() {
        let dictionary = Dictionary::shared();
        let mut account = account();
        let money = account.generic_money;
        // Buckshot: 50 owned, clips of 20 at 1 a round.
        let buck = slot_item(&account, 11).unwrap();
        assert_eq!(buck.condition_or_stack, 50);
        let relocation = |count, count_in_inventory| Relocation {
            item_id: buck.item_id,
            dict_id: buck.dict_id,
            source_slot: INVENTORY_SLOT,
            target_slot: 11,
            count,
            count_in_inventory,
        };
        // 90 in the slot, none stored: 40 more, two clips.
        move_items(&mut account, dictionary, 1, &[relocation(90, 90)]).unwrap();
        assert_eq!(slot_item(&account, 11).unwrap().condition_or_stack, 90);
        assert_eq!(
            account
                .inventory_item(buck.item_id)
                .unwrap()
                .condition_or_stack,
            90
        );
        assert_eq!(account.generic_money, money - 40);
        assert_eq!(slot_item(&account, 11).unwrap().amount, 90);
        // Enough owned: nothing bought.
        move_items(&mut account, dictionary, 1, &[relocation(60, 90)]).unwrap();
        assert_eq!(account.generic_money, money - 40);
        // Too dear: nothing changes.
        account.generic_money = 0;
        let before = account.clone();
        assert_eq!(
            move_items(&mut account, dictionary, 1, &[relocation(200, 200)]),
            Err(NOT_ENOUGH_MONEY)
        );
        assert_eq!(account, before);
    }

    #[test]
    fn the_ammo_dialog_buys_a_stack_not_owned() {
        let dictionary = Dictionary::shared();
        let mut account = Account::new("bob", dictionary);
        // The TT-33's ammo slot (8) emptied and its stack sold.
        let tt_ammo = slot_item(&account, 8).unwrap();
        sell(&mut account, dictionary, tt_ammo.item_id, 1).unwrap();
        assert!(account.inventory_item(tt_ammo.item_id).is_none());
        assert_eq!(slot_item(&account, 8), None);
        let money = account.generic_money;
        let wanted = Relocation {
            item_id: 0,
            dict_id: tt_ammo.dict_id,
            source_slot: INVENTORY_SLOT,
            target_slot: 8,
            count: 100,
            count_in_inventory: 100,
        };
        move_items(&mut account, dictionary, 1, &[wanted]).unwrap();
        let placed = slot_item(&account, 8).unwrap();
        assert_eq!(
            (placed.dict_id, placed.condition_or_stack),
            (tt_ammo.dict_id, 100)
        );
        assert_eq!(account.generic_money, money - 100);
    }

    #[test]
    fn unequipping_and_equipping_a_weapon() {
        let dictionary = Dictionary::shared();
        let mut account = account();
        let spas = slot_item(&account, 10).unwrap();
        let revision = account.profiles[0].revision;

        let to_inventory = Relocation {
            item_id: spas.item_id,
            dict_id: spas.dict_id,
            source_slot: 10,
            target_slot: INVENTORY_SLOT,
            count: 1,
            count_in_inventory: 1,
        };
        move_items(&mut account, dictionary, 1, &[to_inventory]).unwrap();
        assert_eq!(slot_item(&account, 10), None);
        assert_eq!(account.profiles[0].revision, revision + 1);

        // Into the first weapon slot: the APS there goes nowhere by itself,
        // but the shotgun takes the slot.
        let to_weapon1 = Relocation {
            source_slot: INVENTORY_SLOT,
            target_slot: 7,
            ..to_inventory
        };
        move_items(&mut account, dictionary, 1, &[to_weapon1]).unwrap();
        assert_eq!(slot_item(&account, 7), Some(spas));
    }

    #[test]
    fn a_move_against_the_slot_rules_changes_nothing() {
        let dictionary = Dictionary::shared();
        let mut account = account();
        let before = account.clone();
        let helmet = slot_item(&account, 0).unwrap();
        let moves = [
            // Fine on its own...
            Relocation {
                item_id: helmet.item_id,
                dict_id: helmet.dict_id,
                source_slot: 0,
                target_slot: INVENTORY_SLOT,
                count: 1,
                count_in_inventory: 1,
            },
            // ...but a helmet is no weapon.
            Relocation {
                item_id: helmet.item_id,
                dict_id: helmet.dict_id,
                source_slot: INVENTORY_SLOT,
                target_slot: 7,
                count: 1,
                count_in_inventory: 1,
            },
        ];
        assert_eq!(
            move_items(&mut account, dictionary, 1, &moves),
            Err(CANNOT_RELOCATE)
        );
        assert_eq!(account, before);
        // Nor can an item the account doesn't own be moved.
        let stranger = Relocation {
            item_id: 9999,
            ..moves[0]
        };
        assert!(move_items(&mut account, dictionary, 1, &[stranger]).is_err());
    }

    #[test]
    fn buying_a_weapon_and_ammo() {
        let dictionary = Dictionary::shared();
        let mut account = account();
        account.generic_money = 20_000;
        let money = account.generic_money;

        // AKS-74U, 10500, from the Black Market only, at its Elite rank (5,
        // 3000 points) and above.
        assert_eq!(buy(&mut account, dictionary, 13, 1, 1), Err(CANNOT_BUY));
        account.reputations[1].1 = 2999;
        assert_eq!(buy(&mut account, dictionary, 13, 1, 2), Err(CANNOT_BUY));
        assert_eq!(buy(&mut account, dictionary, 13, 1, 0), Err(CANNOT_BUY));
        assert_eq!(account.generic_money, money);
        account.reputations[1].1 = 3000;
        let bought = buy(&mut account, dictionary, 13, 1, 2).unwrap();
        assert_eq!(bought.len(), 1);
        assert_eq!(account.generic_money, money - 10_500);
        let aks = account.inventory_item(bought[0].1).unwrap();
        assert_eq!((aks.dict_id, aks.condition_or_stack), (13, 100));

        // Buckshot the account has: two clips of 20, at 1 a round.
        let buck = slot_item(&account, 11).unwrap();
        let bought = buy(&mut account, dictionary, 22, 2, 0).unwrap();
        assert_eq!(bought, [(22, buck.item_id, 40)]);
        assert_eq!(
            account
                .inventory_item(buck.item_id)
                .unwrap()
                .condition_or_stack,
            90
        );
        assert_eq!(account.generic_money, money - 10_540);
        // The slot holding it carries the new stack.
        assert_eq!(slot_item(&account, 11).unwrap().amount, 90);

        // No such item, and too dear.
        assert_eq!(buy(&mut account, dictionary, 9999, 1, 0), Err(CANNOT_BUY));
        account.generic_money = 10;
        assert_eq!(
            buy(&mut account, dictionary, 13, 1, 2),
            Err(NOT_ENOUGH_MONEY)
        );
    }

    #[test]
    fn selling_takes_the_item_out_of_every_profile() {
        let dictionary = Dictionary::shared();
        let mut account = account();
        let money = account.generic_money;
        let buck = slot_item(&account, 11).unwrap();

        // Part of a stack, in clips of 20: the slot keeps what is left, at
        // most.
        assert_eq!(
            sell(&mut account, dictionary, buck.item_id, 2).unwrap(),
            (40, vec![1])
        );
        assert_eq!(slot_item(&account, 11).unwrap().condition_or_stack, 10);
        assert_eq!(account.generic_money, money + 20);
        // More clips than are left: what is left.
        assert_eq!(
            sell(&mut account, dictionary, buck.item_id, 1).unwrap(),
            (10, vec![1])
        );

        // A weapon: gone from the inventory and its slot.
        let spas = slot_item(&account, 10).unwrap();
        assert_eq!(
            sell(&mut account, dictionary, spas.item_id, 1).unwrap(),
            (1, vec![1])
        );
        assert_eq!(slot_item(&account, 10), None);
        assert!(account.inventory_item(spas.item_id).is_none());
    }

    #[test]
    fn skills_fit_the_level() {
        let dictionary = Dictionary::shared();
        let mut account = account();
        assert_eq!(account.profiles[0].level, 1);
        assert!(set_skills(&mut account, dictionary, 1, &[(1, 2)], &[]).is_err());
        assert!(set_skills(&mut account, dictionary, 1, &[(9, 1)], &[]).is_err());
        let revision = account.profiles[0].revision;
        set_skills(&mut account, dictionary, 1, &[(1, 1)], &[]).unwrap();
        assert_eq!(account.profiles[0].skills, [(1, 1)]);
        // The profile's modifiers changed with them; the same again changes
        // nothing.
        assert_eq!(account.profiles[0].revision, revision.wrapping_add(1));
        set_skills(&mut account, dictionary, 1, &[(1, 1)], &[]).unwrap();
        assert_eq!(account.profiles[0].revision, revision.wrapping_add(1));

        let money = account.generic_money;
        reroll_skills(&mut account, 1, 100).unwrap();
        assert!(account.profiles[0].skills.is_empty());
        assert_eq!(account.generic_money, money - 100);
        assert_eq!(account.profiles[0].revision, revision.wrapping_add(2));
    }
}
