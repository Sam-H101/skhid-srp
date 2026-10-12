//! A player's supplies through a match, kept as the client keeps them.
//!
//! The client gives every life from its copy of the profile
//! (`inventory::setup_from_profile`): each stack slot (ammo, drugs, traps)
//! hands out at most its loadout count from what is left of the owned stack
//! (`amount_in_inventory`), and takes that off. A weapon loads its magazine
//! from its first ammo slot (`weapon_core::load_ammo`). When the player is
//! removed, at death before its respawn, what is left goes back
//! (`inventory::unload_to_profile`), the weapon's rounds to its ammo slot. So
//! a match can't use more than the account owns, and a life starts with no
//! more than the loadout. The spawn message sets the amounts in hand
//! (`player::deserialize`), so the server works the same out and sends it.

use crate::game::SpawnLoad;
use survarium::player_profile::is_stack;
use survarium::player_profile::raw::{player_profile, profile_slot_enum};

use std::collections::HashMap;

/// The slots whose items are stacks given out per life.
pub const STACK_SLOTS: [profile_slot_enum; 10] = [
    profile_slot_enum::ammo1_weapon1_slot,
    profile_slot_enum::ammo2_weapon1_slot,
    profile_slot_enum::ammo1_weapon2_slot,
    profile_slot_enum::ammo2_weapon2_slot,
    profile_slot_enum::quick_slot1,
    profile_slot_enum::quick_slot2,
    profile_slot_enum::quick_slot3,
    profile_slot_enum::quick_slot4,
    profile_slot_enum::quick_slot5,
    profile_slot_enum::quick_slot6,
];

const WEAPONS: [(profile_slot_enum, [profile_slot_enum; 2]); 2] = [
    (
        profile_slot_enum::weapon1_slot,
        [
            profile_slot_enum::ammo1_weapon1_slot,
            profile_slot_enum::ammo2_weapon1_slot,
        ],
    ),
    (
        profile_slot_enum::weapon2_slot,
        [
            profile_slot_enum::ammo1_weapon2_slot,
            profile_slot_enum::ammo2_weapon2_slot,
        ],
    ),
];

/// One life's supplies: the profile with each stack slot's
/// `amount_in_inventory` set to what it carries outside the weapons, and each
/// weapon's load, by weapon slot.
#[derive(Clone, Copy, Debug)]
pub struct Life {
    pub profile: player_profile,
    pub loads: [Option<SpawnLoad>; 2],
}

impl Life {
    /// The load of the weapon `dict_id`, if it is one of this life's.
    pub fn load_of(&self, dict_id: u16) -> Option<SpawnLoad> {
        WEAPONS
            .iter()
            .zip(self.loads)
            .find(|((slot, _), _)| {
                let item = self.profile.slots[*slot];
                item.id != 0 && item.dict_id == dict_id
            })
            .and_then(|(_, load)| load)
    }
}

/// What each stack slot has left for the rest of the match.
#[derive(Clone, Debug, Default)]
pub struct Supplies {
    pools: HashMap<u8, u32>,
}

impl Supplies {
    /// Put back what a life left (`Combat::leftovers`).
    pub fn give_back(&mut self, leftovers: &[(profile_slot_enum, u32)]) {
        for (slot, count) in leftovers {
            if let Some(pool) = self.pools.get_mut(&(*slot as u8)) {
                *pool = pool.saturating_add(*count);
            }
        }
    }

    /// The next life: each stack slot gives up to its loadout count from what
    /// is left, and each weapon loads up to its spawn load (`spawn_load`, the
    /// game data's full load) from its first ammo slot.
    pub fn next_life(
        &mut self,
        profile: &player_profile,
        spawn_load: impl Fn(u16) -> SpawnLoad,
    ) -> Life {
        let mut life = *profile;
        for slot in STACK_SLOTS {
            let item = &mut life.slots[slot];
            if item.id == 0 || !is_stack(item.dict_id) {
                continue;
            }
            let pool = self
                .pools
                .entry(slot as u8)
                .or_insert(item.amount_in_inventory);
            let given = item.condition_or_stack.min(*pool);
            *pool -= given;
            item.amount_in_inventory = given;
        }

        let loads = WEAPONS.map(|(weapon_slot, ammo_slots)| {
            let weapon = life.slots[weapon_slot];
            if weapon.id == 0 {
                return None;
            }
            let (magazine, chamber) = spawn_load(weapon.dict_id);
            // `weapon_core::deserialize` takes its ammo from the first slot
            // that holds any; without one the weapon is empty.
            let Some(ammo_slot) = ammo_slots
                .into_iter()
                .find(|slot| life.slots[*slot].id != 0)
            else {
                return Some((0, chamber.map(|_| false)));
            };
            let given = life.slots[ammo_slot].amount_in_inventory;
            let full = u32::from(magazine) + u32::from(chamber == Some(true));
            let loaded = full.min(given);
            life.slots[ammo_slot].amount_in_inventory = given - loaded;
            let chambered = chamber.map(|chamber| chamber && loaded > 0);
            let in_magazine = loaded - u32::from(chambered == Some(true));
            Some((in_magazine as u16, chambered))
        });
        Life {
            profile: life,
            loads,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> player_profile {
        // The dummy loadout: an AKS-74U with 60 + 30 rounds of 5.45 (one
        // stack of 10,000), a Remington 700 with 10 + 10, 2 bandages, 2 traps.
        player_profile::new_dummy(1, 1, "supplies")
    }

    const AK_LOAD: SpawnLoad = (30, None);
    const REMINGTON_LOAD: SpawnLoad = (4, Some(true));

    fn load(dict_id: u16) -> SpawnLoad {
        if dict_id == 13 {
            AK_LOAD
        } else {
            REMINGTON_LOAD
        }
    }

    fn with_stack(
        mut profile: player_profile,
        slot: profile_slot_enum,
        owned: u32,
    ) -> player_profile {
        profile.slots[slot].amount_in_inventory = owned;
        profile
    }

    #[test]
    fn a_life_takes_the_magazine_from_the_first_ammo_slot() {
        let life = Supplies::default().next_life(&profile(), load);
        let ammo1 = life.profile.slots[profile_slot_enum::ammo1_weapon1_slot];
        let ammo2 = life.profile.slots[profile_slot_enum::ammo2_weapon1_slot];
        // 60 given: 30 in the magazine, 30 carried; the second slot's 30 all
        // carried.
        assert_eq!(life.loads[0], Some(AK_LOAD));
        assert_eq!(
            (ammo1.amount_in_inventory, ammo2.amount_in_inventory),
            (30, 30)
        );
        // The Remington's 10: 4 in the magazine and 1 chambered, 5 carried.
        assert_eq!(life.loads[1], Some(REMINGTON_LOAD));
        let carried = life.profile.slots[profile_slot_enum::ammo1_weapon2_slot];
        assert_eq!(carried.amount_in_inventory, 5);
        assert_eq!(life.load_of(13), Some(AK_LOAD));
        // The loadout counts stay: the client sizes a trap set by it.
        assert_eq!(
            life.profile.slots[profile_slot_enum::quick_slot2].condition_or_stack,
            2
        );
    }

    #[test]
    fn what_a_life_used_is_gone_and_what_it_left_comes_back() {
        // 70 rounds owned in the first slot: a life gets 60.
        let profile = with_stack(profile(), profile_slot_enum::ammo1_weapon1_slot, 70);
        let mut supplies = Supplies::default();
        supplies.next_life(&profile, load);

        // It fires all 60: the next life has the last 10, all in the magazine.
        supplies.give_back(&[(profile_slot_enum::ammo1_weapon1_slot, 0)]);
        let life = supplies.next_life(&profile, load);
        assert_eq!(life.loads[0], Some((10, None)));
        assert_eq!(
            life.profile.slots[profile_slot_enum::ammo1_weapon1_slot].amount_in_inventory,
            0
        );

        // It dies without firing: the 10 come back, and come again.
        supplies.give_back(&[(profile_slot_enum::ammo1_weapon1_slot, 10)]);
        let life = supplies.next_life(&profile, load);
        assert_eq!(life.loads[0], Some((10, None)));

        // All gone: an empty weapon.
        supplies.give_back(&[(profile_slot_enum::ammo1_weapon1_slot, 0)]);
        let life = supplies.next_life(&profile, load);
        assert_eq!(life.loads[0], Some((0, None)));
    }

    #[test]
    fn drugs_and_traps_run_out_over_lives() {
        let profile = with_stack(profile(), profile_slot_enum::quick_slot1, 3);
        let mut supplies = Supplies::default();
        let bandages =
            |life: &Life| life.profile.slots[profile_slot_enum::quick_slot1].amount_in_inventory;

        assert_eq!(bandages(&supplies.next_life(&profile, load)), 2);
        supplies.give_back(&[(profile_slot_enum::quick_slot1, 0)]);
        assert_eq!(bandages(&supplies.next_life(&profile, load)), 1);
        supplies.give_back(&[(profile_slot_enum::quick_slot1, 1)]);
        assert_eq!(bandages(&supplies.next_life(&profile, load)), 1);
    }

    #[test]
    fn a_chambered_weapon_with_one_round_has_it_chambered() {
        let profile = with_stack(profile(), profile_slot_enum::ammo1_weapon2_slot, 1);
        let life = Supplies::default().next_life(&profile, load);
        assert_eq!(life.loads[1], Some((0, Some(true))));
    }
}
