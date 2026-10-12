//! What srp needs from each 0.21d item's own config
//! (`gameplay/items/**.options` in 0.21d's `resources.db`): its faction and
//! faction affinity (for the gear faction, `get_profile_faction_affinity`),
//! its drop value and weight (`drops.options` rolls), and for ammo the pack a
//! drop gives. Generated from the game's files; an item without a field has 0.

/// One item's parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemParams {
    pub dict_id: u16,
    /// 0 none, 1..=4 the lobby factions.
    pub faction: u8,
    pub faction_affinity: u32,
    pub drop_value: u32,
    pub drop_weight: u32,
    /// Rounds a clip of this ammo holds, and clips a pack has.
    pub clip_size: u32,
    pub clips_in_bag: u32,
}

/// By `dict_id`.
#[rustfmt::skip]
pub const ITEM_PARAMS: &[ItemParams] = &[
    ItemParams { dict_id: 5, faction: 0, faction_affinity: 0, drop_value: 2000, drop_weight: 5, clip_size: 0, clips_in_bag: 0 }, // rgd-5
    ItemParams { dict_id: 7, faction: 0, faction_affinity: 0, drop_value: 100, drop_weight: 10, clip_size: 50, clips_in_bag: 1 }, // ammo_5.45x39_7n6m
    ItemParams { dict_id: 9, faction: 1, faction_affinity: 10, drop_value: 3000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_oxygen_1
    ItemParams { dict_id: 12, faction: 1, faction_affinity: 30, drop_value: 3000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_toz_122
    ItemParams { dict_id: 13, faction: 2, faction_affinity: 30, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_ak_74u
    ItemParams { dict_id: 14, faction: 1, faction_affinity: 30, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_rem_700
    ItemParams { dict_id: 15, faction: 2, faction_affinity: 30, drop_value: 1300, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_rem_870
    ItemParams { dict_id: 16, faction: 1, faction_affinity: 30, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_toz_34
    ItemParams { dict_id: 17, faction: 2, faction_affinity: 30, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_toz_66
    ItemParams { dict_id: 18, faction: 0, faction_affinity: 0, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_tt_33
    ItemParams { dict_id: 19, faction: 0, faction_affinity: 0, drop_value: 5000, drop_weight: 5, clip_size: 0, clips_in_bag: 0 }, // wpn_vityaz
    ItemParams { dict_id: 20, faction: 0, faction_affinity: 0, drop_value: 100, drop_weight: 10, clip_size: 20, clips_in_bag: 1 }, // ammo_.357m
    ItemParams { dict_id: 22, faction: 0, faction_affinity: 0, drop_value: 100, drop_weight: 10, clip_size: 20, clips_in_bag: 1 }, // ammo_12mm_buck
    ItemParams { dict_id: 24, faction: 2, faction_affinity: 5, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // blackmarket_boots_1
    ItemParams { dict_id: 25, faction: 2, faction_affinity: 5, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // blackmarket_gloves_1
    ItemParams { dict_id: 27, faction: 2, faction_affinity: 20, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // blackmarket_helmet_1
    ItemParams { dict_id: 28, faction: 2, faction_affinity: 20, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // blackmarket_legs_1
    ItemParams { dict_id: 29, faction: 2, faction_affinity: 30, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // blackmarket_torso_1
    ItemParams { dict_id: 31, faction: 1, faction_affinity: 30, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_torso_1
    ItemParams { dict_id: 32, faction: 2, faction_affinity: 30, drop_value: 3000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // blackmarket_torso_2
    ItemParams { dict_id: 33, faction: 2, faction_affinity: 30, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // blackmarket_torso_3
    ItemParams { dict_id: 34, faction: 1, faction_affinity: 30, drop_value: 3000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_torso_2
    ItemParams { dict_id: 35, faction: 2, faction_affinity: 5, drop_value: 3000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // blackmarket_boots_2
    ItemParams { dict_id: 36, faction: 2, faction_affinity: 5, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // blackmarket_boots_3
    ItemParams { dict_id: 37, faction: 1, faction_affinity: 5, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_boots_1
    ItemParams { dict_id: 38, faction: 1, faction_affinity: 5, drop_value: 3000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_boots_2
    ItemParams { dict_id: 39, faction: 1, faction_affinity: 5, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_boots_3
    ItemParams { dict_id: 40, faction: 2, faction_affinity: 5, drop_value: 3000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // blackmarket_gloves_2
    ItemParams { dict_id: 41, faction: 1, faction_affinity: 5, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_gloves_1
    ItemParams { dict_id: 42, faction: 1, faction_affinity: 5, drop_value: 3000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_gloves_2
    ItemParams { dict_id: 43, faction: 1, faction_affinity: 10, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_resp_1
    ItemParams { dict_id: 44, faction: 2, faction_affinity: 20, drop_value: 2000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // blackmarket_legs_2
    ItemParams { dict_id: 45, faction: 1, faction_affinity: 20, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_legs_1
    ItemParams { dict_id: 46, faction: 1, faction_affinity: 20, drop_value: 3000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_legs_2
    ItemParams { dict_id: 47, faction: 1, faction_affinity: 20, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_legs_3
    ItemParams { dict_id: 48, faction: 1, faction_affinity: 30, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_torso_3
    ItemParams { dict_id: 49, faction: 1, faction_affinity: 15, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // scavenger_back_1
    ItemParams { dict_id: 50, faction: 0, faction_affinity: 0, drop_value: 50, drop_weight: 10, clip_size: 50, clips_in_bag: 1 }, // ammo_7.62x25
    ItemParams { dict_id: 51, faction: 0, faction_affinity: 0, drop_value: 200, drop_weight: 10, clip_size: 10, clips_in_bag: 1 }, // ammo_7.62x51
    ItemParams { dict_id: 52, faction: 0, faction_affinity: 0, drop_value: 200, drop_weight: 5, clip_size: 10, clips_in_bag: 1 }, // ammo_7.62x51_ap
    ItemParams { dict_id: 53, faction: 0, faction_affinity: 0, drop_value: 50, drop_weight: 10, clip_size: 50, clips_in_bag: 1 }, // ammo_9x19p_para
    ItemParams { dict_id: 54, faction: 0, faction_affinity: 0, drop_value: 0, drop_weight: 0, clip_size: 0, clips_in_bag: 0 }, // artefact_lifebone
    ItemParams { dict_id: 55, faction: 2, faction_affinity: 30, drop_value: 2000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_uzi
    ItemParams { dict_id: 56, faction: 2, faction_affinity: 30, drop_value: 3000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_magnum
    ItemParams { dict_id: 58, faction: 3, faction_affinity: 30, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // renesanse_torso_1
    ItemParams { dict_id: 59, faction: 3, faction_affinity: 20, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // renesanse_legs_1
    ItemParams { dict_id: 60, faction: 3, faction_affinity: 20, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // renesanse_helmet_1
    ItemParams { dict_id: 62, faction: 3, faction_affinity: 5, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // renesanse_boots_1
    ItemParams { dict_id: 64, faction: 3, faction_affinity: 30, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_fort_17
    ItemParams { dict_id: 65, faction: 0, faction_affinity: 0, drop_value: 2000, drop_weight: 5, clip_size: 0, clips_in_bag: 0 }, // painkiller
    ItemParams { dict_id: 66, faction: 0, faction_affinity: 0, drop_value: 2000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // bandages
    ItemParams { dict_id: 67, faction: 0, faction_affinity: 0, drop_value: 2000, drop_weight: 5, clip_size: 0, clips_in_bag: 0 }, // medkit
    ItemParams { dict_id: 68, faction: 0, faction_affinity: 0, drop_value: 2000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // blackmarket_trap
    ItemParams { dict_id: 69, faction: 0, faction_affinity: 0, drop_value: 0, drop_weight: 0, clip_size: 0, clips_in_bag: 0 }, // scope_leupold
    ItemParams { dict_id: 70, faction: 0, faction_affinity: 0, drop_value: 50, drop_weight: 10, clip_size: 50, clips_in_bag: 1 }, // ammo_9x18_makarov
    ItemParams { dict_id: 71, faction: 0, faction_affinity: 0, drop_value: 75, drop_weight: 5, clip_size: 50, clips_in_bag: 1 }, // ammo_9x19p_7n21
    ItemParams { dict_id: 72, faction: 0, faction_affinity: 0, drop_value: 150, drop_weight: 5, clip_size: 20, clips_in_bag: 1 }, // ammo_12mm_slug
    ItemParams { dict_id: 73, faction: 0, faction_affinity: 0, drop_value: 100, drop_weight: 0, clip_size: 20, clips_in_bag: 1 }, // ammo_12mm_buck2
    ItemParams { dict_id: 74, faction: 0, faction_affinity: 0, drop_value: 0, drop_weight: 0, clip_size: 0, clips_in_bag: 0 }, // artefact_spring
    ItemParams { dict_id: 76, faction: 0, faction_affinity: 0, drop_value: 0, drop_weight: 0, clip_size: 0, clips_in_bag: 0 }, // artefact_onyx
    ItemParams { dict_id: 80, faction: 3, faction_affinity: 30, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_akmn
    ItemParams { dict_id: 81, faction: 0, faction_affinity: 0, drop_value: 100, drop_weight: 10, clip_size: 30, clips_in_bag: 1 }, // ammo_7.62x39
    ItemParams { dict_id: 82, faction: 0, faction_affinity: 0, drop_value: 2000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // antitoxin
    ItemParams { dict_id: 83, faction: 4, faction_affinity: 30, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // edge_torso_1
    ItemParams { dict_id: 84, faction: 4, faction_affinity: 20, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // edge_legs_1
    ItemParams { dict_id: 85, faction: 4, faction_affinity: 5, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // edge_boots_1
    ItemParams { dict_id: 86, faction: 4, faction_affinity: 20, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // edge_helmet_1
    ItemParams { dict_id: 87, faction: 4, faction_affinity: 15, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // edge_back_1
    ItemParams { dict_id: 88, faction: 1, faction_affinity: 30, drop_value: 2000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_kiparis
    ItemParams { dict_id: 89, faction: 1, faction_affinity: 30, drop_value: 2000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_pm
    ItemParams { dict_id: 90, faction: 2, faction_affinity: 30, drop_value: 3000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_winchester_1894
    ItemParams { dict_id: 91, faction: 0, faction_affinity: 0, drop_value: 100, drop_weight: 10, clip_size: 20, clips_in_bag: 1 }, // ammo_.30-30
    ItemParams { dict_id: 102, faction: 4, faction_affinity: 10, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // edge_resp_1
    ItemParams { dict_id: 103, faction: 0, faction_affinity: 0, drop_value: 75, drop_weight: 5, clip_size: 50, clips_in_bag: 1 }, // ammo_9x18_pbm
    ItemParams { dict_id: 104, faction: 0, faction_affinity: 0, drop_value: 200, drop_weight: 10, clip_size: 10, clips_in_bag: 1 }, // ammo_7.62x54r_old
    ItemParams { dict_id: 106, faction: 0, faction_affinity: 0, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_ppsh
    ItemParams { dict_id: 107, faction: 0, faction_affinity: 0, drop_value: 1000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_mosin_nagant
    ItemParams { dict_id: 108, faction: 0, faction_affinity: 0, drop_value: 2000, drop_weight: 5, clip_size: 0, clips_in_bag: 0 }, // wpn_izh_27_lancaster
    ItemParams { dict_id: 109, faction: 4, faction_affinity: 30, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_sks
    ItemParams { dict_id: 110, faction: 1, faction_affinity: 30, drop_value: 4000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_mp_153
    ItemParams { dict_id: 124, faction: 3, faction_affinity: 30, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_spas_12
    ItemParams { dict_id: 125, faction: 4, faction_affinity: 30, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_benelli_m4
    ItemParams { dict_id: 126, faction: 4, faction_affinity: 30, drop_value: 5000, drop_weight: 10, clip_size: 0, clips_in_bag: 0 }, // wpn_aps
];

/// The parameters of `dict_id`, if it is an item.
pub fn item_params(dict_id: u16) -> Option<&'static ItemParams> {
    ITEM_PARAMS.iter().find(|item| item.dict_id == dict_id)
}

/// The units of `dict_id` one step of the 0.21d shop's slider buys or sells:
/// a clip of ammo (`clip_size`), else one item. The shop shows a price for
/// such a step and sends counts in steps (`MessageAsk.update_slider`,
/// `ohYes`: count = slider / clip_size).
pub fn shop_unit(dict_id: u16) -> u32 {
    item_params(dict_id)
        .map(|params| params.clip_size)
        .filter(|&size| size > 0)
        .unwrap_or(1)
}
