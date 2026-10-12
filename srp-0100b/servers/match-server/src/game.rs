// Canned game content for the relay.
//
// SRP is a mock (see the repo CLAUDE.md): the match server does NOT simulate the
// full game. It relays player updates between clients and translates each
// lobby-selected profile into the runtime state the client needs to spawn.

use crate::lifecycle::MatchRules;
use crate::map;
use crate::message::ServerGameMessageKind;
use crate::message::server_message::raw::game_mode_type;
use survarium::player_input::{
    player, player_inventory_slot, player_stamina, weapon_core, weapon_core_state, weapon_targets,
};
use survarium::player_profile::raw::{
    game_team_id, inventory_item_instance, player_profile as raw_player_profile, profile_slot_enum,
};
use vostok::math::float3;

/// `MatchOptions` describing the match. `player_count` reflects the live roster
/// so the client knows how many `PlayerProfile`s to expect. The respawn and
/// match times and the battery count are the ones the server enforces.
pub fn match_options(player_count: u8, rules: &MatchRules) -> ServerGameMessageKind {
    ServerGameMessageKind::MatchOptions {
        map_id: 0,
        map_name: "level_03_evn".to_string(),
        match_mode: game_mode_type::gather_victory_items,
        player_count,
        victory_item_count: rules.battery_count,
        respawn_time: rules.respawn_secs,
        match_time: rules.match_time_secs,
    }
}

/// Adapt the exact lobby-selected loadout for one recipient's match roster.
pub fn player_profile(
    mut profile: raw_player_profile,
    player_id: u8,
    team: game_team_id,
    name: &str,
    is_local: bool,
) -> raw_player_profile {
    let name = wire_profile_name(name, player_id);
    profile.account_id = u32::from(player_id);
    profile.team = team;
    profile.is_local = is_local;
    profile.profile_name = [0; 32];
    profile.profile_name[..name.len()].copy_from_slice(name.as_bytes());
    profile
}

/// The stock profile payload requires 4–29 bytes and cannot carry an embedded
/// NUL. Login identity is intentionally permissive, so make a safe display name
/// at this wire boundary instead of letting a user-provided email panic the
/// entire match thread.
fn wire_profile_name(name: &str, player_id: u8) -> String {
    let mut end = name.len().min(29);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    let name = &name[..end];
    if name.len() > 3 && !name.contains('\0') {
        name.to_owned()
    } else {
        format!("player_{player_id}")
    }
}

/// Every respawn point of `team`, as indexes into `map::RESPAWN_POINTS`.
pub fn team_respawn_points(team: game_team_id) -> Vec<usize> {
    map::RESPAWN_POINTS
        .iter()
        .enumerate()
        .filter(|(_, point)| point.team == team)
        .map(|(index, _)| index)
        .collect()
}

pub fn active_weapon_slot(profile: &raw_player_profile) -> Option<profile_slot_enum> {
    [
        profile_slot_enum::weapon1_slot,
        profile_slot_enum::weapon2_slot,
    ]
    .into_iter()
    .find(|slot| profile.slots[*slot].id != 0)
}

fn ammo_slots_for_weapon(weapon_slot: profile_slot_enum) -> [profile_slot_enum; 2] {
    match weapon_slot {
        profile_slot_enum::weapon1_slot => [
            profile_slot_enum::ammo1_weapon1_slot,
            profile_slot_enum::ammo2_weapon1_slot,
        ],
        profile_slot_enum::weapon2_slot => [
            profile_slot_enum::ammo1_weapon2_slot,
            profile_slot_enum::ammo2_weapon2_slot,
        ],
        _ => unreachable!("not a weapon slot"),
    }
}

/// What a freshly spawned weapon holds: its magazine, and its chamber if it
/// has one (`Some(true)`: a round is chambered).
pub type SpawnLoad = (u16, Option<bool>);

/// The spawn loads of the 0.100b weapons, as the game data gives them, for
/// when `resources.db` cannot be read (a test asserts they still agree).
pub fn fallback_spawn_load(dict_id: u16) -> SpawnLoad {
    match dict_id {
        12 => (4, Some(true)), // TOZ-122
        13 => (30, None),      // AK-74u
        14 => (4, Some(true)), // Remington 700
        15 => (5, Some(true)), // Remington 870
        16 | 17 => (2, None),  // TOZ-34 / TOZ-66
        18 => (8, None),       // TT-33
        19 => (30, None),      // Vityaz
        55 => (30, None),      // Uzi
        56 => (6, None),       // Magnum
        64 => (13, None),      // Fort-17
        _ => (1, None),
    }
}

/// The quick-slot items the client reads no spawn state for: the lifebone
/// artefacts (`artefact_lifebone_core` leaves `deserialize` empty), as the
/// game data gives them (a test asserts they still agree). An amount sent for
/// one would be read as the next item's.
pub const NO_SPAWN_STATE: [u16; 2] = [54, 57];

fn runtime_amount(item: inventory_item_instance) -> u16 {
    item.condition_or_stack
        .min(item.amount_in_inventory)
        .min(u32::from(u16::MAX)) as u16
}

/// A weapon as a spawn message gives it (`weapon_core::deserialize`): where
/// its random generators (`random32`, `normal_random`) draw next, its
/// magazine and chamber, the ammo slot it fires from (`None`: its first
/// filled one) and its fire mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponLoad {
    pub seeds: (u32, i32),
    pub load: SpawnLoad,
    pub ammo_slot: Option<profile_slot_enum>,
    pub fire_queue_type: u8,
}

/// What a spawn message says besides the profile (`player::deserialize`):
/// where the player stands and looks, the weapon in its hands, its stamina,
/// what each weapon holds, and what the stack slots (ammo, drugs, traps)
/// hold in hand where that is not the profile's amount.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnState {
    pub position: float3,
    pub yaw: f32,
    pub look_pitch: f32,
    /// The weapon in hand once the client reads it: the one its freshly
    /// inserted player holds (`active_weapon_slot`). Any other would be
    /// activated while the message is read (`player::deserialize`,
    /// 0x5E5394), and its weapon would then read a logic state the message
    /// doesn't carry and crash the client (0x5A46C4).
    pub active_slot: profile_slot_enum,
    /// The weapon the player switches to after (the client's target active
    /// object, set without activating).
    pub target_slot: profile_slot_enum,
    pub stamina: player_stamina,
    /// By weapon slot.
    pub weapons: [Option<WeaponLoad>; 2],
    pub amounts: Vec<(profile_slot_enum, u32)>,
}

/// The stamina a (re)spawn shows: full, its times long past.
pub const SPAWN_STAMINA: player_stamina = player_stamina {
    value: 100.,
    last_spending_time_in_ms: 10,
    last_tick_time_in_ms: 10,
    lower_threshold_was_reached: false,
};

/// The look pitch a (re)spawn shows.
pub const SPAWN_LOOK_PITCH: f32 = 0.2;

fn runtime_weapon(
    weapon_slot: profile_slot_enum,
    item: inventory_item_instance,
    profile: &raw_player_profile,
    active_slot: profile_slot_enum,
    weapon: WeaponLoad,
) -> weapon_core {
    let ammo_slots = ammo_slots_for_weapon(weapon_slot);
    let ammo_slot = weapon
        .ammo_slot
        .filter(|slot| ammo_slots.contains(slot))
        .into_iter()
        .chain(ammo_slots)
        .find(|slot| profile.slots[*slot].id != 0)
        .unwrap_or(profile_slot_enum::max_slots_count);
    let is_active = weapon_slot == active_slot;
    let (ammo_in_magazine, chamber) = weapon.load;

    weapon_core {
        // `inventory_item::serialize`: current weapon condition.
        inventory_item: item.condition_or_stack.min(u32::from(u16::MAX)) as u16,
        // The server replays the same spread from these.
        random_seed: weapon.seeds.0,
        normal_random_seed: weapon.seeds.1 as u32,
        weapon_target: if is_active {
            weapon_targets::idle
        } else {
            weapon_targets::inactive
        },
        old_actions_mask: 0,
        ammo_in_magazine,
        bullets_in_queue: 0,
        fire_queue_type: weapon.fire_queue_type,
        ammo_slot,
        is_there_chamber_a_round_state: chamber,
        // Read only for a weapon with a logic state (0x5A4627): at a spawn,
        // the one `player::insert` activated (0x5E4DD5). Sent for another,
        // it would shift every item after it by 12 bytes (a trap slot read
        // the 10 of a transition time).
        weapon_core_state: is_active.then_some(weapon_core_state {
            is_shown: true,
            active_hands: 1,
            start_transition_time_in_ms_lhs: 10,
            start_transition_time_in_ms_rhs: 10,
            // All weapon FSMs put idle at 3.
            weapon_sound_target_state: (3, None),
            logic_sprint_target_state: (0, None),
        }),
    }
}

/// Runtime state matching the lobby-selected profile, at the chosen spawn.
/// The spawned player at `map::RESPAWN_POINTS[respawn_point]`, facing the way
/// the map points it, its weapons seeded with `seeds` (per weapon slot) and
/// loaded as `spawn_load` says (the game data), else by the fallback table.
pub fn spawn_content(
    respawn_point: usize,
    profile: &raw_player_profile,
    seeds: [(u32, i32); 2],
    spawn_load: impl Fn(u16) -> Option<SpawnLoad>,
) -> player {
    let point = &map::RESPAWN_POINTS[respawn_point % map::RESPAWN_POINTS.len()];
    let weapons = [
        profile_slot_enum::weapon1_slot,
        profile_slot_enum::weapon2_slot,
    ]
    .map(|slot| {
        let item = profile.slots[slot];
        (item.id != 0).then(|| WeaponLoad {
            seeds: seeds[usize::from(slot == profile_slot_enum::weapon2_slot)],
            load: spawn_load(item.dict_id).unwrap_or_else(|| fallback_spawn_load(item.dict_id)),
            ammo_slot: None,
            fire_queue_type: 0,
        })
    });
    player_content(
        profile,
        &SpawnState {
            position: point.position,
            yaw: point.yaw,
            look_pitch: SPAWN_LOOK_PITCH,
            active_slot: active_weapon_slot(profile).unwrap_or(profile_slot_enum::weapon1_slot),
            target_slot: active_weapon_slot(profile).unwrap_or(profile_slot_enum::weapon1_slot),
            stamina: SPAWN_STAMINA,
            weapons,
            amounts: Vec::new(),
        },
    )
}

/// A spawn message's player: `state` over the profile's items, in the order
/// `inventory::deserialize` reads them (every filled slot from the first
/// weapon on, but the lifebone artefacts). A weapon `state` says nothing
/// about gets the fallback table's load.
pub fn player_content(profile: &raw_player_profile, state: &SpawnState) -> player {
    let active_slot = state.active_slot;
    let mut player_inventory = Vec::new();
    for slot_index in
        profile_slot_enum::weapon1_slot as usize..profile_slot_enum::max_slots_count as usize
    {
        let slot = bytemuck::checked::cast::<u8, profile_slot_enum>(slot_index as u8);
        let item = profile.slots[slot];
        if item.id == 0 {
            continue;
        }
        if matches!(
            slot,
            profile_slot_enum::weapon1_slot | profile_slot_enum::weapon2_slot
        ) {
            let weapon = state.weapons[usize::from(slot == profile_slot_enum::weapon2_slot)]
                .unwrap_or_else(|| WeaponLoad {
                    seeds: (0, 1),
                    load: fallback_spawn_load(item.dict_id),
                    ammo_slot: None,
                    fire_queue_type: 0,
                });
            player_inventory.push(player_inventory_slot::weapon_slot(runtime_weapon(
                slot,
                item,
                profile,
                active_slot,
                weapon,
            )));
        } else if !NO_SPAWN_STATE.contains(&item.dict_id) {
            let amount = state
                .amounts
                .iter()
                .find(|(listed, _)| *listed == slot)
                .map_or_else(
                    || runtime_amount(item),
                    |(_, amount)| (*amount).min(u32::from(u16::MAX)) as u16,
                );
            player_inventory.push(player_inventory_slot::item_amount(amount));
        }
    }

    player {
        position: state.position,
        orientation: state.yaw,
        look_pitch: state.look_pitch,
        is_alive: true,
        server_current_active_slot: active_slot,
        server_target_active_slot: state.target_slot,
        player_stamina: state.stamina,
        player_inventory,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    fn profile_name(name: &str) -> String {
        let selected = raw_player_profile::new_dummy(1, 10, "selected");
        let profile = player_profile(selected, 7, game_team_id::team_1, name, true);
        CStr::from_bytes_until_nul(&profile.profile_name)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn sanitizes_profile_names_for_the_stock_wire_format() {
        assert_eq!(profile_name("abc"), "player_7");
        assert_eq!(
            profile_name("abcdefghijklmnopqrstuvwxyz0123456789"),
            "abcdefghijklmnopqrstuvwxyz012"
        );
        assert_eq!(profile_name("valid@example.com"), "valid@example.com");
        assert_eq!(profile_name("bad\0name"), "player_7");
    }

    #[test]
    fn spawn_loadout_serializes_every_selected_runtime_item() {
        let profile = raw_player_profile::new_dummy(1, 10, "selected");
        let player = spawn_content(0, &profile, [(5, 6), (7, 8)], |_| None);
        assert_eq!(player.player_inventory.len(), 8);

        let player_inventory_slot::weapon_slot(weapon) = player.player_inventory[0] else {
            panic!("weapon state must precede its ammo slot states");
        };
        assert_eq!(weapon.inventory_item, 100);
        assert_eq!((weapon.random_seed, weapon.normal_random_seed), (5, 6));
        assert_eq!(weapon.ammo_in_magazine, 30);
        assert_eq!(weapon.is_there_chamber_a_round_state, None);
        assert_eq!(weapon.bullets_in_queue, 0);
        assert_eq!(weapon.ammo_slot, profile_slot_enum::ammo1_weapon1_slot);
        assert_eq!(
            player.player_inventory[1],
            player_inventory_slot::item_amount(60)
        );
        assert_eq!(
            player.player_inventory[2],
            player_inventory_slot::item_amount(30)
        );

        let player_inventory_slot::weapon_slot(second_weapon) = player.player_inventory[3] else {
            panic!("second selected weapon must have runtime state");
        };
        assert_eq!(
            (second_weapon.random_seed, second_weapon.normal_random_seed),
            (7, 8)
        );
        // The Remington 700 chambers a round from its 5-round magazine.
        assert_eq!(second_weapon.ammo_in_magazine, 4);
        assert_eq!(
            second_weapon.ammo_slot,
            profile_slot_enum::ammo1_weapon2_slot
        );
        assert_eq!(second_weapon.is_there_chamber_a_round_state, Some(true));
        assert_eq!(
            player.player_inventory[4],
            player_inventory_slot::item_amount(10)
        );
        assert_eq!(
            player.player_inventory[5],
            player_inventory_slot::item_amount(10)
        );
    }

    #[test]
    fn a_lifebone_artefact_has_no_spawn_state() {
        let mut profile = raw_player_profile::new_dummy(1, 10, "selected");
        let traps = profile.slots[profile_slot_enum::quick_slot2];
        assert_eq!(traps.dict_id, 68);
        profile.slots[profile_slot_enum::quick_slot1] = inventory_item_instance {
            id: 1_000_054,
            dict_id: 54,
            ..traps
        };
        let player = spawn_content(0, &profile, [(5, 6), (7, 8)], |_| None);
        assert_eq!(player.player_inventory.len(), 7);
        assert_eq!(
            player.player_inventory.last(),
            Some(&player_inventory_slot::item_amount(runtime_amount(traps)))
        );
    }

    /// A player shown as it is now: only the values differ from a spawn's.
    #[test]
    fn a_live_state_changes_the_values_and_keeps_the_layout() {
        use crate::message::ServerGameMessageKind;
        use vostok::network_packet::{Packet, UdpPacket};

        let profile = raw_player_profile::new_dummy(1, 10, "selected");
        let spawned = spawn_content(0, &profile, [(5, 6), (7, 8)], |_| None);
        let stamina = player_stamina {
            value: 42.5,
            last_spending_time_in_ms: 9_000,
            last_tick_time_in_ms: 9_016,
            lower_threshold_was_reached: true,
        };
        let state = SpawnState {
            position: float3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            yaw: 0.5,
            look_pitch: -0.25,
            active_slot: profile_slot_enum::weapon1_slot,
            target_slot: profile_slot_enum::weapon2_slot,
            stamina,
            weapons: [
                Some(WeaponLoad {
                    seeds: (0xABCD, 0x1234),
                    load: (17, None),
                    ammo_slot: Some(profile_slot_enum::ammo2_weapon1_slot),
                    fire_queue_type: 1,
                }),
                Some(WeaponLoad {
                    seeds: (9, 10),
                    load: (2, Some(false)),
                    // Another weapon's ammo slot is not taken.
                    ammo_slot: Some(profile_slot_enum::ammo1_weapon1_slot),
                    fire_queue_type: 0,
                }),
            ],
            amounts: vec![
                (profile_slot_enum::ammo1_weapon1_slot, 7),
                (profile_slot_enum::quick_slot2, 3),
            ],
        };
        let live = player_content(&profile, &state);

        assert_eq!(live.position, state.position);
        assert_eq!((live.orientation, live.look_pitch), (0.5, -0.25));
        assert_eq!(
            live.server_current_active_slot,
            profile_slot_enum::weapon1_slot
        );
        assert_eq!(
            live.server_target_active_slot,
            profile_slot_enum::weapon2_slot
        );
        assert_eq!(live.player_stamina, stamina);
        let player_inventory_slot::weapon_slot(first) = live.player_inventory[0] else {
            panic!("{:?}", live.player_inventory[0]);
        };
        assert_eq!(
            (first.random_seed, first.normal_random_seed),
            (0xABCD, 0x1234)
        );
        assert_eq!((first.ammo_in_magazine, first.fire_queue_type), (17, 1));
        assert_eq!(first.ammo_slot, profile_slot_enum::ammo2_weapon1_slot);
        // In hand as the client reads it; the second is switched to after.
        assert_eq!(first.weapon_target, weapon_targets::idle);
        let player_inventory_slot::weapon_slot(second) = live.player_inventory[3] else {
            panic!("{:?}", live.player_inventory[3]);
        };
        assert_eq!(second.weapon_target, weapon_targets::inactive);
        assert_eq!(second.ammo_slot, profile_slot_enum::ammo1_weapon2_slot);
        assert_eq!(second.is_there_chamber_a_round_state, Some(false));
        assert_eq!(
            live.player_inventory[1],
            player_inventory_slot::item_amount(7)
        );
        // Slots without an amount keep the profile's.
        assert_eq!(live.player_inventory[2], spawned.player_inventory[2]);
        assert_eq!(
            live.player_inventory[7],
            player_inventory_slot::item_amount(3)
        );

        let size = |player| {
            let mut packet = UdpPacket::new();
            vostok::serde::Serialize::serialize(
                ServerGameMessageKind::SpawnPlayer {
                    player_id: 0,
                    player,
                },
                &mut packet,
            );
            packet.cursor_index()
        };
        assert_eq!(size(live), size(spawned));
    }

    #[test]
    fn spawn_loads_come_from_the_game_data() {
        let profile = raw_player_profile::new_dummy(1, 10, "selected");
        let player = spawn_content(0, &profile, [(0, 1); 2], |_| Some((17, Some(false))));
        let player_inventory_slot::weapon_slot(weapon) = player.player_inventory[0] else {
            panic!("expected the first weapon");
        };
        assert_eq!(
            (
                weapon.ammo_in_magazine,
                weapon.is_there_chamber_a_round_state
            ),
            (17, Some(false))
        );
    }

    #[test]
    fn fallback_spawn_loads_match_the_game_data() {
        use crate::combat::system::{Combat, CombatAssets};
        let Some(assets) = CombatAssets::for_tests() else {
            return;
        };
        let combat = Combat::new(assets);
        let weapons = combat.weapon_dict_ids();
        assert!(!weapons.is_empty());
        for dict_id in weapons {
            assert_eq!(
                combat.spawn_load(dict_id),
                Some(fallback_spawn_load(dict_id)),
                "weapon {dict_id}"
            );
        }
    }

    #[test]
    fn match_options_advertise_the_enforced_times() {
        let rules = MatchRules {
            match_time_secs: 120,
            respawn_secs: 5,
            battery_count: 3,
            ..MatchRules::default()
        };
        let ServerGameMessageKind::MatchOptions {
            respawn_time,
            match_time,
            victory_item_count,
            ..
        } = match_options(2, &rules)
        else {
            panic!("expected MatchOptions");
        };
        assert_eq!((respawn_time, match_time, victory_item_count), (5, 120, 3));
    }

    #[test]
    fn match_profile_preserves_selected_slots() {
        let mut selected = raw_player_profile::new_dummy(1, 10, "selected");
        selected.slots[profile_slot_enum::weapon1_slot].dict_id = 55;
        let adapted = player_profile(selected, 1, game_team_id::team_2, "player", true);

        assert_eq!(adapted.slots, selected.slots);
        assert_eq!(adapted.team, game_team_id::team_2);
        assert!(adapted.is_local);
    }
}
