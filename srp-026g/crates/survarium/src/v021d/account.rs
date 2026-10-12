//! A 0.21d account as the lobby serves it: up to 8 profiles of 23 slots, one
//! inventory they all equip from, money, reputations and skills
//! (`../srp-021d/docs/notes/lobby_spec.md` §3).

use crate::dictionary::{Dictionary, PROFILE_SLOTS};

/// At most this many profiles: a 9th overwrites the client's inventory
/// (spec R7).
pub const MAX_PROFILES: usize = 8;
/// Modification ids an item can carry (spec R7: more is a fatal assert).
pub const MAX_MODIFICATIONS: usize = 8;
/// Names and nicknames go into 64-byte buffers (spec R3).
pub const MAX_NAME_LEN: usize = 63;

/// The slots that carry an amount besides the condition or stack
/// (`slot_serialize_mode_enum` 2 in the table at 0x877168): every slot but
/// the armour (0..=6) and the two weapons (7, 10).
pub fn slot_carries_amount(slot: usize) -> bool {
    !matches!(slot, 0..=7 | 10)
}

/// One item instance, as both the inventory and the profile slots send it
/// (`inventory_item_descr`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub dict_id: u16,
    /// The instance id; 0 means an empty slot to the client.
    pub item_id: u32,
    /// For a stack item a count, for anything else its condition.
    pub condition_or_stack: u16,
    /// `amount_in_inventory`; the lobby UI never shows it [spec: P].
    pub amount: u32,
    pub modifications: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// Non-zero and distinct among the account's profiles.
    pub id: u32,
    pub name: String,
    pub autobuy: bool,
    /// Also the skill points the profile has unlocked.
    pub level: u8,
    pub exp_total: u32,
    pub exp_level_start: u32,
    pub exp_next_level: u32,
    /// Bumped on every change of equipment: the client rebuilds its 3D
    /// character only when the profile id or this changes.
    pub revision: u32,
    pub slots: [Option<Item>; PROFILE_SLOTS],
    /// `(skill_id, points)`.
    pub skills: Vec<(u8, u8)>,
    pub perks: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub generic_money: u32,
    pub premium_money: u32,
    /// When premium runs out, in Unix seconds; 0 without.
    pub premium_until: u64,
    /// `(faction_id, points)` for the factions the lobby shows, 1..=4.
    pub reputations: Vec<(u8, u16)>,
    /// Loyalty to factions 1..=4, in % (`loyalty`).
    pub loyalty: [u8; 4],
    /// The ranking's rating (`rating`), and what it was before the last
    /// match that moved it.
    pub rating: u16,
    pub rating_before: u16,
    pub profiles: Vec<Profile>,
    /// Everything the account owns, the equipped items included: the client
    /// looks each slot's item up here.
    pub inventory: Vec<Item>,
    /// The id the next new item instance gets.
    pub next_item_id: u32,
    /// Its daily quests, active and proposed.
    pub quests: Vec<super::quests::QuestInstance>,
    /// `(quest_id, until)`: quests resting after being done, in Unix seconds.
    pub quest_cooldowns: Vec<(u32, u64)>,
    pub next_quest_instance: u32,
}

/// The rounds a new account starts with [a choice]: one 7.62x25 pack.
pub const STARTING_ROUNDS: u16 = 50;
/// A new account's only items, `(slot, item)` [the user's choice]: the
/// TT-33, the game's cheapest pistol, in weapon slot 1 and its ammo in that
/// weapon's first ammo slot. No armour, no other weapon, no consumables.
const STARTER: [(usize, &str); 2] = [(7, "tt_33"), (8, "ammo_7.62x25")];
/// What a new account's items are in [a choice: the client checks no range].
pub const FULL_CONDITION: u16 = 100;
/// A new account's silver [a choice of the user's].
pub const STARTING_MONEY: u32 = 2_000;
/// A new account's gold [a choice of the user's].
pub const STARTING_PREMIUM_MONEY: u32 = 1_000;
/// The first instance id; ids below are the starting items'.
const FIRST_ITEM_ID: u32 = 1;

impl Account {
    /// A new account with one profile, wearing only [`STARTER`].
    pub fn new(name: &str, dictionary: &Dictionary) -> Self {
        Self::with_items(
            name,
            dictionary,
            STARTER.iter().map(|&(slot, name)| (slot, name.to_owned())),
        )
    }

    /// A new account wearing the game's own test loadout
    /// (`test_player_profiles.options` profile 0, its list index taken as
    /// the slot): every slot filled, for tests of equipment rules.
    pub fn with_test_loadout(name: &str, dictionary: &Dictionary) -> Self {
        let loadout = dictionary
            .test_profiles
            .first()
            .cloned()
            .unwrap_or_default();
        Self::with_items(name, dictionary, loadout.into_iter().enumerate())
    }

    /// A new account with one profile wearing `(slot, item name)`; items
    /// unknown or not fitting their slot are left out.
    fn with_items(
        name: &str,
        dictionary: &Dictionary,
        items: impl IntoIterator<Item = (usize, String)>,
    ) -> Self {
        let mut account = Self {
            name: cap(name),
            generic_money: STARTING_MONEY,
            premium_money: STARTING_PREMIUM_MONEY,
            premium_until: 0,
            reputations: (1..=4).map(|faction| (faction, 0)).collect(),
            loyalty: [0; 4],
            rating: super::rating::START,
            rating_before: super::rating::START,
            profiles: Vec::new(),
            inventory: Vec::new(),
            next_item_id: FIRST_ITEM_ID,
            quests: Vec::new(),
            quest_cooldowns: Vec::new(),
            next_quest_instance: 1,
        };

        let mut slots: [Option<Item>; PROFILE_SLOTS] = Default::default();
        for (slot, name) in items {
            let Some(entry) = dictionary.item_named(&name) else {
                continue;
            };
            if slot >= PROFILE_SLOTS || !dictionary.fits(entry.dict_id, slot as u8) {
                continue;
            }
            let count = if entry.is_stack {
                STARTING_ROUNDS
            } else {
                FULL_CONDITION
            };
            let item = account.add_item(entry.dict_id, count, entry.is_stack);
            slots[slot] = Some(item);
        }

        account.profiles.push(Profile {
            id: 1,
            name: account.name.clone(),
            autobuy: false,
            level: 1,
            exp_total: 0,
            exp_level_start: 0,
            exp_next_level: super::results::level_at(0).2,
            revision: 1,
            slots,
            skills: Vec::new(),
            perks: Vec::new(),
        });
        account
    }

    /// A new instance into the inventory; returns it as a slot holds it.
    pub fn add_item(&mut self, dict_id: u16, condition_or_stack: u16, is_stack: bool) -> Item {
        let item = Item {
            dict_id,
            item_id: self.next_item_id,
            condition_or_stack,
            amount: if is_stack {
                condition_or_stack.into()
            } else {
                1
            },
            modifications: Vec::new(),
        };
        self.next_item_id += 1;
        self.inventory.push(item.clone());
        item
    }

    pub fn profile(&self, id: u32) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    pub fn inventory_item(&self, item_id: u32) -> Option<&Item> {
        self.inventory.iter().find(|item| item.item_id == item_id)
    }

    /// Bring the profile slots up to the inventory, after its stacks
    /// changed: a slot holds a copy of the item taken when it was put there.
    /// A slot that carries an amount (`slot_carries_amount`) and holds a
    /// stack gets the owned stack as its `amount`; its `condition_or_stack`,
    /// what a life takes, stays the player's choice. A slot whose item is no
    /// longer owned is emptied. A profile whose slots changed gets a new
    /// `revision`.
    pub fn sync_stacks(&mut self) {
        let dictionary = Dictionary::shared();
        let inventory = &self.inventory;
        for profile in &mut self.profiles {
            let mut changed = false;
            for (index, slot) in profile.slots.iter_mut().enumerate() {
                let Some(item) = slot.as_mut() else {
                    continue;
                };
                let Some(owned) = inventory.iter().find(|owned| owned.item_id == item.item_id)
                else {
                    *slot = None;
                    changed = true;
                    continue;
                };
                let is_stack = dictionary
                    .item(item.dict_id)
                    .is_some_and(|entry| entry.is_stack);
                let amount = u32::from(owned.condition_or_stack);
                if is_stack && slot_carries_amount(index) && item.amount != amount {
                    item.amount = amount;
                    changed = true;
                }
            }
            if changed {
                profile.revision = profile.revision.wrapping_add(1);
            }
        }
    }
}

/// A name cut to what the client's buffers take.
pub fn cap(name: &str) -> String {
    let mut end = name.len().min(MAX_NAME_LEN);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_account_has_a_pistol_and_its_ammo_only() {
        let dictionary = Dictionary::v021d();
        let account = Account::new("Player", &dictionary);
        let profile = &account.profiles[0];
        let worn: Vec<(usize, u16, u16)> = profile
            .slots
            .iter()
            .enumerate()
            .filter_map(|(slot, item)| {
                item.as_ref()
                    .map(|item| (slot, item.dict_id, item.condition_or_stack))
            })
            .collect();
        assert_eq!(worn, [(7, 18, FULL_CONDITION), (8, 50, STARTING_ROUNDS)]);
        // Every equipped item is in the inventory, under the same id.
        for item in profile.slots.iter().flatten() {
            assert_eq!(account.inventory_item(item.item_id), Some(item));
        }
        assert_eq!(account.inventory.len(), 2);
        // The ammo fits the pistol.
        assert!(dictionary.compatibility.contains(&(50, 18)));
    }

    #[test]
    fn slots_follow_the_inventory_stacks() {
        let dictionary = Dictionary::v021d();
        let mut account = Account::new("Player", &dictionary);
        let revision = account.profiles[0].revision;
        // Nothing stale: nothing changes.
        account.sync_stacks();
        assert_eq!(account.profiles[0].revision, revision);

        // The stack grows; the slot keeps its per-life count.
        let ammo = account.profiles[0].slots[8].clone().unwrap();
        account.profiles[0].slots[8]
            .as_mut()
            .unwrap()
            .condition_or_stack = 30;
        let owned = account
            .inventory
            .iter_mut()
            .find(|item| item.item_id == ammo.item_id)
            .unwrap();
        owned.condition_or_stack = 120;
        owned.amount = 120;
        account.sync_stacks();
        let slot = account.profiles[0].slots[8].as_ref().unwrap();
        assert_eq!((slot.condition_or_stack, slot.amount), (30, 120));
        assert_eq!(account.profiles[0].revision, revision + 1);
        // The weapon slot carries no amount: untouched.
        assert_eq!(account.profiles[0].slots[7].as_ref().unwrap().amount, 1);

        // The stack gone: its slot empties.
        account
            .inventory
            .retain(|item| item.item_id != ammo.item_id);
        account.sync_stacks();
        assert_eq!(account.profiles[0].slots[8], None);
        assert!(account.profiles[0].slots[7].is_some());
        assert_eq!(account.profiles[0].revision, revision + 2);
    }

    #[test]
    fn names_fit_the_client_buffers() {
        assert_eq!(cap(&"x".repeat(100)).len(), 63);
        // Never inside a character.
        assert_eq!(cap(&"я".repeat(40)).len(), 62);
    }
}
