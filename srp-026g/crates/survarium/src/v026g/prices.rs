//! The 0.26g shop: prices from each item's `base_cost` in the dictionary
//! (0.21d's data had none, hence its hand-made `crate::prices`), but for
//! ammo, which costs about half of it by the owner's choice
//! ([`AMMO_PRICES`]).
//!
//! What the 0.26g client does with a price (`lobby_menu.swf`, decompiled
//! with FFDec, and the exe):
//! - The exe hands `base_cost` to the lobby as the item descriptor's `cost`
//!   (0x286950, member name at 0xA015C4). PROVEN.
//! - **A stack's price is for one step of the slider, and ammo's step is a
//!   clip** (`MessageAsk.update_slider`: `_step = clip_size`;
//!   `onSliderUpdate`: `slider / _step * _price`). The lobby uses the
//!   descriptor's `cost` this way when selling
//!   (`InventoryList.sellObject`: `cost / 4` a step), so ammo's `base_cost`
//!   is **per clip**, not per round. PROVEN for the client's use of it;
//!   that the original server charged `base_cost` a clip in its price list
//!   is LIKELY (the same dialog and step show the shop's price).
//! - Selling gives a quarter: `cost / 4`, times `condition / 100` for an item
//!   that is no stack (`InventoryList.sellObject`, `ItemCard`). PROVEN (what
//!   the client shows; the server pays).
//! - A repair costs `(1 - condition / 100) * repair_cost_ratio * cost`, plus
//!   the fix costs of the item's defects (`MessageRepair.init`). PROVEN.
//! - A premium item's `cost` is in gold: the lobby multiplies it by the
//!   premium exchange rate (service price 4, `lobby.md` §2.13) before
//!   taking the quarter.
//!
//! The price list (lobby 0x8A, `lobby.md` §2.10) is records of
//! `u32 cost, u16 dict_id, u8 level, u8 faction_id`. `GangShop.setupShopItems`
//! reads `faction_id` as the seller and `level` as the reputation level with
//! it that buying needs (`canBuy = unlockedLevel[seller] >= level`); seller 0
//! is always buyable. PROVEN.
//!
//! **The client decides nothing here.** The exe keeps the records as a
//! flat array and hands every one to the lobby (0x223190: `faction_id`,
//! `level`, `dictId`, `cost` of each, no grouping). `setupShopItems` makes
//! a shop item of each record under the record's seller, with the record's
//! level as its lock (and the rank's name as the lock's label), and adds the
//! seller to the item descriptor's `sellers`. An item's own config
//! (`faction`, `item_level`) is not consulted for the shop. PROVEN (the
//! ActionScript and the exe's filler). So the seller and the rank of an item
//! are wholly the server's, and **an item may have several records, one a
//! seller**: `descrData.sellers` is an array the lobby grows
//! (`noSeller` / `push`), an owned item shows a seller's icon only when it
//! has one seller (`InventoryList.createItem`), and the shop's seller tabs
//! test every seller of an item (`isSeller`). PROVEN in the ActionScript;
//! not yet seen live. When the window names no seller, the exe names the
//! seller of the first record of the item (0x288BC0).
//!
//! Which level an item needs is not in the client. srp's rule [LIKELY]: the
//! item's `item_level`, less 5 for Renaissance and Edge. The data fits it:
//! Scavenger and Black Market gear has item levels 1..=5 and those factions
//! five levels above 0; Renaissance and Edge gear has 6..=15 and those
//! factions ten. The wiki's lists of January and February 2015 (0.26g to
//! 0.26i; `progression_sources.md` §3) agree with the rule for every weapon
//! and piece of armour but the few in [`EXCEPTIONS`], and they give the
//! sellers and ranks of ammo, drugs, the trap and grenades, which no config
//! has.
//!
//! What is listed [srp's choice where marked]:
//! - weapons and armour whose config names a faction, sold by it at the
//!   rule's level, or as [`EXCEPTIONS`] says;
//! - drugs, the trap and grenades: by the factions at the ranks of
//!   [`EXCEPTIONS`], one record for each faction that sold it;
//! - ammo: by the factions of [`EXCEPTIONS`], **at no rank** by the owner's
//!   choice ([`AMMO_NEEDS_RANK`]). 0.26 sold ammo at a rank (patch note
//!   0.26c) and 0.27a took that back ("Purchase of ammunition no longer
//!   requires reputation"); the ranks of the time stay in the table;
//! - the boosters (the wiki's "Independent Traders") and the one ammo on no
//!   list of the time (9x19 7N21): seller 0 and level 0 [choice for the
//!   7N21: unknown, so sold to everyone as before];
//! - not listed: anything with `base_cost` 0 (the starting weapons and
//!   their ammo, artefacts, money), premium items (their cost is gold; they
//!   belong to the premium shop, 0x95), weapons with no faction (the bundle
//!   weapons IZh-27, Vityaz, VSS, SR-3M, and the SOCOM 16), scopes and
//!   artefacts (as 0.21d), and cosmetics and vouchers (the wiki lists
//!   camouflages and decals at ranks; not applied).

use super::dictionary::{Dictionary, Item};
use super::item_params::{ItemParams, item_params};

/// 0.26g faction ids (`factions_dict`), as 0.21d's.
pub use crate::prices::faction;

/// The categories listed with no seller: ammo (9, 18..=21), drugs and
/// boosters (10), traps (11) and grenades (24).
const SUPPLIES: [u8; 8] = [9, 10, 11, 18, 19, 20, 21, 24];
/// Weapons (8, 13..=17) and armour (1..=7).
const GEAR: [u8; 13] = [1, 2, 3, 4, 5, 6, 7, 8, 13, 14, 15, 16, 17];

/// One record of the price list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PriceItem {
    /// For one item; for ammo, one clip ([`super::item_params::shop_unit`]).
    pub cost: u32,
    pub dict_id: u16,
    /// The reputation level with `faction_id` that buying needs.
    pub level: u8,
    /// The seller; 0 sells to everyone.
    pub faction_id: u8,
}

impl PriceItem {
    /// The wire size of a record.
    pub const SIZE: usize = 8;

    /// `u32 cost, u16 dict_id, u8 level, u8 faction_id`, little-endian.
    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut bytes = [0; Self::SIZE];
        bytes[..4].copy_from_slice(&self.cost.to_le_bytes());
        bytes[4..6].copy_from_slice(&self.dict_id.to_le_bytes());
        bytes[6] = self.level;
        bytes[7] = self.faction_id;
        bytes
    }
}

/// Where an item is sold when the module's rule does not say it:
/// `(item name, [(seller, reputation level)])`. An ammo's levels are the
/// game's of the time and are not asked for ([`AMMO_NEEDS_RANK`]). Sources
/// are wiki revisions written while 0.26g to 0.26i was live
/// (`progression_sources.md`; `W:<id>` is a revision id of
/// survarium.fandom.com) and the official patch notes (`PN`). PROVEN =
/// stated there and not contradicted by the client's data.
pub const EXCEPTIONS: &[(&str, &[(u8, u8)])] = {
    use faction::{BLACK_MARKET as BM, EDGE, RENAISSANCE as REN, SCAVENGERS as SCAV};
    &[
        // --- Weapons and armour ---
        // 9A-91 and VSK-94: "Fringe Settlers ... Added new weapons: H&K
        // UMP45, 9A-91, VSK-94" (PN 0.25a); ranks 8 and 9 (W:8175, W:9794).
        // Both configs say Renaissance. PROVEN.
        ("a9_91", &[(EDGE, 8)]),
        ("vsk_94", &[(EDGE, 9)]),
        // The chemical protection set and the Amber respirator: rank 2
        // "Determined" (W:8170); item level 3 in the data, and 0.27a "moved
        // to equipment tier 3" what was lower before. PROVEN.
        ("scav_torso_2", &[(SCAV, 2)]),
        ("scav_legs_2", &[(SCAV, 2)]),
        ("scav_gloves_2", &[(SCAV, 2)]),
        ("scav_boots_2", &[(SCAV, 2)]),
        ("scav_resp_1", &[(SCAV, 2)]),
        // Fort 17 and APS: rank 2, "Moved Fort 17 and 9x18 PBM to correct
        // rank" (W:8176, W:8175). PROVEN.
        ("fort_17", &[(REN, 2)]),
        ("aps", &[(EDGE, 2)]),
        // The "Marodeur" container: Black Market, rank 4 (W:8172); its
        // config says Scavengers, item level 3. PROVEN.
        ("bm_back_1", &[(BM, 4)]),
        // --- Ammo (W:8170, W:8172, W:8175, W:8176; PROVEN but where
        // said) ---
        ("ammo_12mm_buck", &[(SCAV, 1), (BM, 1)]),
        ("ammo_12mm_slug", &[(SCAV, 3), (BM, 3)]),
        ("ammo_9x18_makarov", &[(SCAV, 2)]),
        ("ammo_9x19p_para", &[(BM, 2)]),
        ("ammo_7.62x51", &[(SCAV, 3)]),
        // 7.62x51 M59.
        ("ammo_7.62x51_ap", &[(SCAV, 5)]),
        ("ammo_.30-30", &[(BM, 3)]),
        ("ammo_.357m", &[(BM, 3)]),
        ("ammo_5.45x39_7n6m", &[(BM, 5)]),
        ("ammo_7.62x39", &[(REN, 1), (EDGE, 1)]),
        ("ammo_9x18_pbm", &[(REN, 2), (EDGE, 2)]),
        // 9x21 SP-10.
        ("ammo_9x21", &[(REN, 3)]),
        ("ammo_45acp", &[(EDGE, 3)]),
        ("ammo_7.62x39_7n23", &[(REN, 4), (EDGE, 4)]),
        // "7.62x39 RPD": LIKELY this item, the only 7.62x39 with a clip of
        // 100 (the RPD's belt).
        ("ammo_7.62x39_m", &[(REN, 4)]),
        // Renaissance: "on sale at the Renaissance Army store on the 7th
        // level of reputation (Captain)" (PN 0.26c, official).
        ("ammo_7.62x54_7n1", &[(REN, 7), (EDGE, 4)]),
        ("ammo_9x19p_7n31", &[(EDGE, 4)]),
        ("ammo_5.45x39_7n10", &[(REN, 5), (EDGE, 5)]),
        ("ammo_pab9", &[(REN, 8), (EDGE, 8)]),
        ("ammo_sp5", &[(REN, 9), (EDGE, 9)]),
        ("ammo_sp6", &[(REN, 9), (EDGE, 9)]),
        ("ammo_5.45x39_7n22", &[(REN, 9)]),
        // 6.5x39 Grendel.
        ("ammo_6.5x39", &[(REN, 10)]),
        ("ammo_12.7x55", &[(EDGE, 10)]),
        // "12.7x54": the list has one name, the data two rounds (PT and
        // VPS); both here [choice].
        ("ammo_12.7x54_pt", &[(EDGE, 10)]),
        ("ammo_12.7x54_vps", &[(EDGE, 10)]),
        // --- Drugs, the trap, grenades (the four faction pages and
        // `Items` W:9996; PROVEN) ---
        ("bandages", &[(SCAV, 1)]),
        ("medkit", &[(SCAV, 3)]),
        ("antitoxin", &[(SCAV, 3)]),
        ("base_trap", &[(BM, 1)]),
        // The anesthetic.
        ("painkiller", &[(BM, 3)]),
        ("rgd-5", &[(SCAV, 3), (BM, 3)]),
        // The RDG-M smoke grenade.
        ("rdg-m", &[(REN, 1), (EDGE, 1)]),
    ]
};

/// The owner's prices for a clip of ammo (2026-10-13): `(item name, the
/// game's `base_cost`, srp's price)`. About half the game's, each set by
/// hand and rounded down to a multiple of 5, as the game's own prices are.
/// Every ammo with a price is here; the two the game gives away
/// (7.62x25, 7.62x54R) stay free.
///
/// What the client does with it (`lobby_menu.swf`):
/// - The shop shows this price: its item's cost is the price record's
///   (`GangShop.setupShopItems`: `item.cost = p[i].cost`). PROVEN.
/// - **The kit's ammo window shows the game's own price for a stack the
///   player owns**: an inventory item's cost is the item descriptor's
///   (`InventoryList`: `item_data.cost = descrData.cost`, the exe's
///   `base_cost`), and only `canBuy`, the lock texts and `premiumCost` are
///   taken from the shop's item. PROVEN. So that window shows, and counts
///   what the player can afford by, twice what srp takes; the balance the
///   server sends after is the true one.
/// - Selling pays a quarter of the game's price ([`sell_price`], which the
///   lobby shows from the descriptor), so no price here may be under that
///   quarter: buying and selling back would make silver.
pub const AMMO_PRICES: &[(&str, u32, u32)] = &[
    ("ammo_12mm_buck", 20, 10),
    ("ammo_12mm_slug", 30, 15),
    ("ammo_9x18_makarov", 20, 10),
    ("ammo_9x19p_para", 20, 10),
    ("ammo_.357m", 50, 25),
    ("ammo_.30-30", 50, 25),
    ("ammo_7.62x51", 150, 75),
    ("ammo_7.62x51_ap", 200, 100),
    ("ammo_5.45x39_7n6m", 60, 30),
    ("ammo_9x19_7n21", 50, 25),
    ("ammo_7.62x39", 60, 30),
    ("ammo_9x18_pbm", 50, 25),
    // Half is 32.5.
    ("ammo_9x21", 65, 30),
    ("ammo_45acp", 65, 30),
    ("ammo_7.62x39_m", 150, 75),
    ("ammo_7.62x54_7n1", 220, 110),
    ("ammo_7.62x39_7n23", 80, 40),
    ("ammo_9x19p_7n31", 70, 35),
    // Half is 42.5.
    ("ammo_5.45x39_7n10", 85, 40),
    ("ammo_pab9", 100, 50),
    ("ammo_sp5", 120, 60),
    ("ammo_sp6", 120, 60),
    ("ammo_5.45x39_7n22", 120, 60),
    ("ammo_6.5x39", 140, 70),
    ("ammo_12.7x55", 150, 75),
    ("ammo_12.7x54_pt", 250, 125),
    ("ammo_12.7x54_vps", 300, 150),
];

/// Whether buying ammo needs the seller's rank of [`EXCEPTIONS`]. The
/// owner's choice (2026-10-13): no, every ammo is sold to everyone, as the
/// game itself did from 0.27a on. The sellers stay, so the ammo stays in
/// its factions' shop tabs.
///
/// What the client does with a record at level 0 (`lobby_menu.swf`): it is
/// buyable whatever the player's level with the seller
/// (`GangShop.setupShopItems`: `canBuy = _unlockedLevel[seller] >=
/// sellerLevel`, levels start at 0), so the shop shows no lock and the
/// kit's ammo window, which copies `canBuy` from the shop's item, lets the
/// stack grow. PROVEN in the ActionScript.
pub const AMMO_NEEDS_RANK: bool = false;

/// What the shop takes for a step of `item`: [`AMMO_PRICES`] for ammo, else
/// the game's `base_cost`.
pub fn shop_cost(item: &Item) -> u32 {
    AMMO_PRICES
        .iter()
        .find(|(name, _, _)| *name == item.name)
        .map_or(item.base_cost, |&(_, _, cost)| cost)
}

/// The reputation level with its faction that buying an item of
/// `item_level` needs (the module's rule), within the faction's levels.
pub fn seller_level(dictionary: &Dictionary, faction_id: u8, item_level: u8) -> u8 {
    let level = match faction_id {
        faction::SCAVENGERS | faction::BLACK_MARKET => item_level,
        faction::RENAISSANCE | faction::EDGE => item_level.saturating_sub(5),
        _ => 0,
    };
    level.min(top_level(dictionary, faction_id))
}

/// The last reputation level of a faction.
fn top_level(dictionary: &Dictionary, faction_id: u8) -> u8 {
    dictionary
        .factions
        .iter()
        .find(|entry| entry.id == faction_id)
        .map_or(0, |entry| entry.levels.len().saturating_sub(1)) as u8
}

/// Who sells `item` and at which reputation level: `(seller, level)`, by
/// seller. [`EXCEPTIONS`] first, else the module's rule; ammo at level 0
/// ([`AMMO_NEEDS_RANK`]).
pub fn sellers_of(dictionary: &Dictionary, item: &Item, params: &ItemParams) -> Vec<(u8, u8)> {
    if item.is_premium {
        return Vec::new();
    }
    let gear = GEAR.contains(&item.category);
    let supply = SUPPLIES.contains(&item.category) && item.is_stack;
    if item.base_cost == 0 {
        // A free supply (the starting weapons' ammo) is listed at no cost by
        // seller 0, who sells to everyone: the client lets a player add to a
        // stack only when the price list has the item (`canBuy`, set by
        // `GangShop.setupShopItems`), and for a price of 0 it allows up to
        // `MAX_FREE_ITEMS_COUNT` (1000) units (`AmmoShareElement.updateStorage`).
        // Without the record the ammo window's storage field is locked (seen
        // in the first live session, 2026-10-12). Free weapons are not listed.
        return if supply { vec![(0, 0)] } else { Vec::new() };
    }
    let exception = EXCEPTIONS.iter().find(|(name, _)| *name == item.name);
    match exception {
        Some(&(_, sellers)) if gear || supply => {
            let ranked = AMMO_NEEDS_RANK || params.kind != "ammo";
            let mut sellers: Vec<(u8, u8)> = sellers
                .iter()
                .map(|&(seller, level)| {
                    let level = if ranked { level } else { 0 };
                    (seller, level.min(top_level(dictionary, seller)))
                })
                .collect();
            sellers.sort_unstable();
            sellers
        }
        _ if gear && params.faction != 0 => {
            let level = seller_level(dictionary, params.faction, params.item_level);
            vec![(params.faction, level)]
        }
        _ if supply => vec![(0, 0)],
        _ => Vec::new(),
    }
}

fn listed(item: &Item, params: &ItemParams) -> impl Iterator<Item = PriceItem> {
    let (cost, dict_id) = (shop_cost(item), item.dict_id);
    sellers_of(Dictionary::shared(), item, params)
        .into_iter()
        .map(move |(faction_id, level)| PriceItem {
            cost,
            dict_id,
            level,
            faction_id,
        })
}

/// The price list the lobby sends, by `dict_id` and then by seller: an
/// item two factions sell has two records.
pub fn price_list() -> &'static [PriceItem] {
    static SHARED: std::sync::OnceLock<Vec<PriceItem>> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| {
        Dictionary::shared()
            .items
            .iter()
            .filter_map(|item| Some(listed(item, item_params(item.dict_id)?)))
            .flatten()
            .collect()
    })
}

/// The price list as the 0x8A body unpacks to: `n × 8` bytes.
pub fn price_list_bytes() -> Vec<u8> {
    price_list()
        .iter()
        .flat_map(|price| price.to_bytes())
        .collect()
}

/// The shop's records for `dict_id`, one a seller; empty if it is not sold.
/// Every record of an item has the same cost.
pub fn sellers(dict_id: u16) -> &'static [PriceItem] {
    let list = price_list();
    let start = list.partition_point(|price| price.dict_id < dict_id);
    let end = list.partition_point(|price| price.dict_id <= dict_id);
    &list[start..end]
}

/// The shop's first record for `dict_id`, if it is sold: its cost is the
/// item's whoever sells it. Who may buy is [`record_for`]'s to say.
pub fn price(dict_id: u16) -> Option<&'static PriceItem> {
    sellers(dict_id).first()
}

/// The record an account buys `dict_id` by: of the sellers that sell to it
/// (`reputation_level(faction)` is its level with a faction; seller 0
/// sells to everyone) the one named `preferred` if it is among them, else
/// the first. `Err` is the record that refused: `preferred`'s, else the
/// first; `Ok(None)` when the item is not sold.
///
/// [choice] A buy naming a seller whose rank the account lacks goes through
/// another seller of the same item whose rank it has: the cost is the same,
/// and the client itself names only the first record's seller when its
/// window names none (0x288BC0).
pub fn record_for(
    dict_id: u16,
    preferred: u8,
    reputation_level: impl Fn(u8) -> u8,
) -> Result<Option<&'static PriceItem>, &'static PriceItem> {
    let records = sellers(dict_id);
    let Some(first) = records.first() else {
        return Ok(None);
    };
    let may = |record: &&PriceItem| {
        record.faction_id == 0 || reputation_level(record.faction_id) >= record.level
    };
    let named = records.iter().find(|record| record.faction_id == preferred);
    match named.filter(may).or_else(|| records.iter().find(may)) {
        Some(record) => Ok(Some(record)),
        None => Err(named.unwrap_or(first)),
    }
}

/// What buying `steps` of `dict_id` costs, or `None` if it is not sold.
/// A step is the shop's unit ([`super::item_params::shop_unit`]): a clip of
/// ammo, else one item. The buy dialog sends its count in steps
/// (`MessageAsk`: `count = slider / _step`; PROVEN, `lobby_menu.swf`), so
/// the buyer gets `steps * shop_unit` units.
pub fn buy_cost(dict_id: u16, steps: u32) -> Option<u64> {
    price(dict_id).map(|price| u64::from(price.cost) * u64::from(steps))
}

/// An item's `base_cost` in generic money: a premium item's gold times
/// `premium_exchange_rate` (the rate the server gave the lobby).
fn cost_in_generic_money(item: &Item, premium_exchange_rate: u32) -> u64 {
    let cost = u64::from(item.base_cost);
    if item.is_premium {
        cost * u64::from(premium_exchange_rate)
    } else {
        cost
    }
}

/// What the lobby shows for selling `dict_id`: a quarter of its cost, for a
/// stack a step (`count` units are `count / shop_unit` steps), else times
/// its condition (0..=100).
pub fn sell_price(
    dictionary: &Dictionary,
    dict_id: u16,
    condition_or_count: u32,
    premium_exchange_rate: u32,
) -> u64 {
    let Some(item) = dictionary.item(dict_id) else {
        return 0;
    };
    let quarter = cost_in_generic_money(item, premium_exchange_rate) / 4;
    if item.is_stack {
        quarter * u64::from(condition_or_count / super::item_params::shop_unit(dict_id))
    } else {
        quarter * u64::from(condition_or_count.min(100)) / 100
    }
}

/// What the lobby shows for repairing `dict_id` from `condition` (0..=100)
/// to full, without its defects' fix costs.
pub fn repair_cost(dictionary: &Dictionary, dict_id: u16, condition: u32) -> u32 {
    let (Some(item), Some(params)) = (dictionary.item(dict_id), item_params(dict_id)) else {
        return 0;
    };
    let worn = 1.0 - condition.min(100) as f32 / 100.0;
    (worn * (params.repair_cost_ratio * item.base_cost as f32)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_eight_bytes() {
        let record = PriceItem {
            cost: 47500,
            dict_id: 3,
            level: 5,
            faction_id: 3,
        };
        assert_eq!(record.to_bytes(), [0x8C, 0xB9, 0, 0, 3, 0, 5, 3]);
        assert_eq!(price_list_bytes().len(), price_list().len() * 8);
        assert_eq!(price_list_bytes()[..8], price_list()[0].to_bytes());
    }

    #[test]
    fn ammo_costs_the_owners_prices() {
        let dictionary = Dictionary::shared();
        // Every ammo the game prices is in the table, with the game's
        // price as the table says it, and no other item is.
        let priced: Vec<&str> = dictionary
            .items
            .iter()
            .filter(|item| {
                item.base_cost > 0
                    && !item.is_premium
                    && item_params(item.dict_id).is_some_and(|params| params.kind == "ammo")
            })
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(priced.len(), AMMO_PRICES.len());
        for &(name, game, cost) in AMMO_PRICES {
            assert!(priced.contains(&name), "{name}");
            let item = dictionary.item_named(name).unwrap();
            assert_eq!(item.base_cost, game, "{name}");
            assert_eq!(shop_cost(item), cost, "{name}");
            // About half, never over it, in the game's own steps of 5.
            assert!(cost * 2 <= game && cost * 5 >= game * 2, "{name}");
            assert_eq!(cost % 5, 0, "{name}");
            // Selling a clip back pays a quarter of the game's price
            // (`sell_price`): never more than the clip cost.
            let clip = super::super::item_params::shop_unit(item.dict_id);
            let sold = sell_price(dictionary, item.dict_id, clip, 100);
            assert!(sold <= u64::from(cost), "{name}: {sold} for {cost}");
            assert_eq!(buy_cost(item.dict_id, 3), Some(u64::from(cost) * 3), "{name}");
        }
        // The 5.45x39 7N6M: 30 a clip, where the game says 60.
        let rounds = dictionary.item_named("ammo_5.45x39_7n6m").unwrap();
        assert_eq!(price(rounds.dict_id).unwrap().cost, 30);
        // Anything else is the game's.
        let medkit = dictionary.item_named("medkit").unwrap();
        assert_eq!(shop_cost(medkit), medkit.base_cost);
    }

    #[test]
    fn prices_are_the_dictionarys_base_costs() {
        let dictionary = Dictionary::shared();
        let list = price_list();
        // By item, then by seller; no record twice.
        assert!(
            list.windows(2)
                .all(|w| { (w[0].dict_id, w[0].faction_id) < (w[1].dict_id, w[1].faction_id) })
        );
        for record in list {
            let item = dictionary.item(record.dict_id).unwrap();
            let ammo = item_params(record.dict_id).is_some_and(|params| params.kind == "ammo");
            if ammo && item.base_cost > 0 {
                assert_eq!(record.cost, shop_cost(item), "{}", item.name);
                assert!(record.cost < item.base_cost, "{}", item.name);
            } else {
                assert_eq!(record.cost, item.base_cost, "{}", item.name);
            }
            assert!(!item.is_premium, "{}", item.name);
            // Only a stack is listed for nothing (the starting ammo).
            assert!(
                record.cost > 0 || (item.is_stack && record.faction_id == 0),
                "{}",
                item.name
            );
            assert!(record.faction_id <= 4, "{}", item.name);
            // The config's faction sells its gear, but for the exceptions.
            let params = item_params(record.dict_id).unwrap();
            let excepted = EXCEPTIONS.iter().any(|(name, _)| *name == item.name);
            if GEAR.contains(&item.category) && !excepted {
                assert_eq!(record.faction_id, params.faction, "{}", item.name);
                assert_eq!(
                    record.level,
                    seller_level(dictionary, params.faction, params.item_level),
                    "{}",
                    item.name
                );
            }
        }
        // The AK-74M: Renaissance, item level 10.
        let ak = price(3).unwrap();
        assert_eq!((ak.cost, ak.faction_id, ak.level), (47500, 3, 5));
        // The AKS-74U: Black Market's last level.
        let aks = price(13).unwrap();
        assert_eq!((aks.cost, aks.faction_id, aks.level), (12920, 2, 5));
        // The ranks of the January 2015 list.
        let rank = |name: &str| {
            let record = price(dictionary.item_named(name).unwrap().dict_id).unwrap();
            (record.faction_id, record.level)
        };
        assert_eq!(rank("toz_34"), (1, 1));
        assert_eq!(rank("kiparis"), (1, 2));
        assert_eq!(rank("toz_122"), (1, 3));
        assert_eq!(rank("mp_153"), (1, 4));
        assert_eq!(rank("rem_700"), (1, 5));
        assert_eq!(rank("toz_66"), (2, 1));
        assert_eq!(rank("uzi"), (2, 2));
        assert_eq!(rank("akmn"), (3, 1));
        assert_eq!(rank("spas_12"), (3, 2));
        assert_eq!(rank("sks"), (4, 1));
        assert_eq!(rank("vssk"), (4, 10));
        // No level is past its faction's last.
        for record in list.iter().filter(|record| record.faction_id != 0) {
            let levels = dictionary.factions[usize::from(record.faction_id) - 1]
                .levels
                .len();
            assert!(usize::from(record.level) < levels, "{}", record.dict_id);
        }
    }

    /// `(seller, level)` of every record of an item.
    fn sold_by(name: &str) -> Vec<(u8, u8)> {
        let dict_id = Dictionary::shared().item_named(name).unwrap().dict_id;
        sellers(dict_id)
            .iter()
            .map(|record| (record.faction_id, record.level))
            .collect()
    }

    #[test]
    fn the_exceptions_are_items_of_the_dictionary_sold_as_listed() {
        let dictionary = Dictionary::shared();
        for (index, &(name, listed)) in EXCEPTIONS.iter().enumerate() {
            let item = dictionary
                .item_named(name)
                .unwrap_or_else(|| panic!("no item `{name}`"));
            assert!(item.base_cost > 0 && !item.is_premium, "{name}");
            assert!(
                GEAR.contains(&item.category) || SUPPLIES.contains(&item.category),
                "{name}"
            );
            assert!(!listed.is_empty() && listed.len() <= 2, "{name}");
            // Once in the table, and each seller once.
            assert!(
                EXCEPTIONS[..index].iter().all(|(other, _)| *other != name),
                "{name}"
            );
            let mut expected = listed.to_vec();
            expected.sort_unstable();
            expected.dedup_by_key(|(seller, _)| *seller);
            assert_eq!(expected.len(), listed.len(), "{name}");
            assert!(
                expected
                    .iter()
                    .all(|&(seller, level)| (1..=4).contains(&seller) && level > 0)
            );
            // Ammo: the listed sellers, at no rank.
            if !AMMO_NEEDS_RANK && item_params(item.dict_id).unwrap().kind == "ammo" {
                expected.iter_mut().for_each(|(_, level)| *level = 0);
            }
            assert_eq!(sold_by(name), expected, "{name}");
        }
        // The names the sources use: the 9A-91 and VSK-94 are dict ids 303
        // and 305.
        assert_eq!(dictionary.item_named("a9_91").unwrap().dict_id, 303);
        assert_eq!(dictionary.item_named("vsk_94").unwrap().dict_id, 305);
    }

    #[test]
    fn the_bandit_balaclava_goes_by_the_rule() {
        // No exception (owner, 2026-10-16): Black Market at its item level,
        // within the faction's ranks 0 to 5. players.db still has the
        // release's item level 3; the next release's has 8, which the rule
        // puts at Black Market's last rank.
        let dictionary = Dictionary::shared();
        assert_eq!(top_level(dictionary, faction::BLACK_MARKET), 5);
        assert_eq!(seller_level(dictionary, faction::BLACK_MARKET, 8), 5);
        let item = dictionary.item_named("bandit_026_mask").unwrap();
        let params = item_params(item.dict_id).unwrap();
        assert_eq!(params.faction, faction::BLACK_MARKET);
        assert_eq!(
            sold_by("bandit_026_mask"),
            [(
                faction::BLACK_MARKET,
                seller_level(dictionary, faction::BLACK_MARKET, params.item_level)
            )]
        );
    }

    #[test]
    fn gear_exceptions_move_the_seller_or_the_rank() {
        // Edge sells the 9A-91 and VSK-94, whose configs say Renaissance.
        assert_eq!(sold_by("a9_91"), [(4, 8)]);
        assert_eq!(sold_by("vsk_94"), [(4, 9)]);
        // The chemical set and the respirator: rank 2, item level 3.
        for name in [
            "scav_torso_2",
            "scav_legs_2",
            "scav_gloves_2",
            "scav_boots_2",
            "scav_resp_1",
        ] {
            assert_eq!(sold_by(name), [(1, 2)], "{name}");
        }
        assert_eq!(sold_by("fort_17"), [(3, 2)]);
        assert_eq!(sold_by("aps"), [(4, 2)]);
        // The Duty exoskeleton by the rule: Renaissance at its level 15.
        assert_eq!(sold_by("duty_exo_back"), [(3, 10)]);
        assert_eq!(sold_by("bm_back_1"), [(2, 4)]);
        // Their neighbours go by the rule.
        assert_eq!(sold_by("scav_torso_1"), [(1, 1)]);
    }

    #[test]
    fn supplies_are_sold_by_the_factions_at_ranks() {
        // Two sellers: two records, by seller. Ammo needs no rank.
        assert_eq!(sold_by("ammo_12mm_buck"), [(1, 0), (2, 0)]);
        assert_eq!(sold_by("ammo_12mm_slug"), [(1, 0), (2, 0)]);
        assert_eq!(sold_by("ammo_7.62x39"), [(3, 0), (4, 0)]);
        assert_eq!(sold_by("ammo_7.62x54_7n1"), [(3, 0), (4, 0)]);
        assert_eq!(sold_by("ammo_sp6"), [(3, 0), (4, 0)]);
        assert_eq!(sold_by("rgd-5"), [(1, 3), (2, 3)]);
        assert_eq!(sold_by("rdg-m"), [(3, 1), (4, 1)]);
        // One seller.
        assert_eq!(sold_by("ammo_5.45x39_7n6m"), [(2, 0)]);
        assert_eq!(sold_by("ammo_5.45x39_7n22"), [(3, 0)]);
        assert_eq!(sold_by("ammo_6.5x39"), [(3, 0)]);
        assert_eq!(sold_by("ammo_12.7x55"), [(4, 0)]);
        assert_eq!(sold_by("bandages"), [(1, 1)]);
        assert_eq!(sold_by("medkit"), [(1, 3)]);
        assert_eq!(sold_by("antitoxin"), [(1, 3)]);
        assert_eq!(sold_by("base_trap"), [(2, 1)]);
        assert_eq!(sold_by("painkiller"), [(2, 3)]);
        // On no list: the 9x19 7N21 stays with seller 0, as the boosters.
        assert_eq!(sold_by("ammo_9x19_7n21"), [(0, 0)]);
        assert_eq!(sold_by("speed_booster"), [(0, 0)]);
        // Every other ammo, drug, trap and grenade has a faction.
        let dictionary = Dictionary::shared();
        let unsold: Vec<&str> = price_list()
            .iter()
            .filter(|record| record.faction_id == 0)
            .map(|record| dictionary.item(record.dict_id).unwrap().name.as_str())
            .filter(|name| !name.ends_with("_booster"))
            .collect();
        // With seller 0: the one ammo on no faction's list, and the two
        // free ones (the starting weapons').
        assert_eq!(
            unsold,
            ["ammo_7.62x25", "ammo_9x19_7n21", "ammo_7.62x54r_old"]
        );
        // The free ammo is listed by seller 0 at no cost and no rank.
        assert_eq!(sold_by("ammo_7.62x25"), [(0, 0)]);
        assert_eq!(sold_by("ammo_7.62x54r_old"), [(0, 0)]);
    }

    #[test]
    fn every_ammo_is_sold_to_everyone() {
        let dictionary = Dictionary::shared();
        let ammo: Vec<&Item> = dictionary
            .items
            .iter()
            .filter(|item| item_params(item.dict_id).is_some_and(|params| params.kind == "ammo"))
            .collect();
        assert_eq!(ammo.len(), 29);
        // Known to nobody.
        let nobody = |_: u8| 0;
        for item in ammo {
            let records = sellers(item.dict_id);
            assert!(!records.is_empty(), "{}", item.name);
            assert!(records.iter().all(|r| r.level == 0), "{}", item.name);
            // Whichever seller the client names.
            for seller in 0..=4 {
                assert!(
                    matches!(record_for(item.dict_id, seller, nobody), Ok(Some(_))),
                    "{} from {seller}",
                    item.name
                );
            }
        }
        // Nothing else lost its rank: of the records with a faction, only
        // ammo is at level 0.
        for record in price_list().iter().filter(|r| r.faction_id != 0) {
            let ammo = item_params(record.dict_id).unwrap().kind == "ammo";
            assert_eq!(record.level == 0, ammo, "{}", record.dict_id);
        }
    }

    #[test]
    fn a_buyer_gets_the_record_of_a_seller_that_sells_to_it() {
        let id = |name: &str| Dictionary::shared().item_named(name).unwrap().dict_id;
        let seller = |result: Result<Option<&'static PriceItem>, &'static PriceItem>| {
            result.map(|record| record.map(|record| record.faction_id))
        };
        // The RGD-5: the Scavengers and the Black Market, level 3 each.
        let grenade = id("rgd-5");
        // Known to nobody: the first record refuses, or the one named.
        let nobody = |_: u8| 0;
        assert_eq!(record_for(grenade, 0, nobody).unwrap_err().faction_id, 1);
        assert_eq!(record_for(grenade, 2, nobody).unwrap_err().faction_id, 2);
        // Known to the Black Market only: it sells, whoever was named.
        let black_market = |faction: u8| if faction == 2 { 3 } else { 0 };
        assert_eq!(seller(record_for(grenade, 0, black_market)), Ok(Some(2)));
        assert_eq!(seller(record_for(grenade, 1, black_market)), Ok(Some(2)));
        assert_eq!(seller(record_for(grenade, 2, black_market)), Ok(Some(2)));
        // Known to both: the one named, else the first.
        let both = |_: u8| 5;
        assert_eq!(seller(record_for(grenade, 2, both)), Ok(Some(2)));
        assert_eq!(seller(record_for(grenade, 0, both)), Ok(Some(1)));
        // Seller 0 sells to everyone; what is not sold has no record.
        assert_eq!(
            seller(record_for(id("speed_booster"), 0, nobody)),
            Ok(Some(0))
        );
        assert_eq!(seller(record_for(id("tt_33"), 0, both)), Ok(None));
    }

    #[test]
    fn ammo_is_priced_a_clip() {
        // 5.45x39 7N6M: 30 for a clip of 30 (the game's 60, `AMMO_PRICES`).
        let ammo = price(7).unwrap();
        assert_eq!((ammo.cost, ammo.faction_id, ammo.level), (30, 2, 0));
        assert_eq!(buy_cost(7, 1), Some(30));
        assert_eq!(buy_cost(7, 3), Some(90));
        // A medkit is 20, one at a time.
        assert_eq!(buy_cost(67, 3), Some(60));
        assert_eq!(buy_cost(3, 1), Some(47500));
        // Both records of an item cost the same.
        for records in price_list().chunk_by(|a, b| a.dict_id == b.dict_id) {
            assert!(records.iter().all(|record| record.cost == records[0].cost));
        }
    }

    #[test]
    fn what_is_not_sold() {
        let dictionary = Dictionary::shared();
        let sold = |name: &str| price(dictionary.item_named(name).unwrap().dict_id).is_some();
        // Free: the starting weapons and their ammo.
        assert!(!sold("tt_33") && !sold("ppsh") && !sold("mosin_nagant"));
        // Their ammo is listed, for nothing.
        assert_eq!(
            price(dictionary.item_named("ammo_7.62x25").unwrap().dict_id).map(|r| r.cost),
            Some(0)
        );
        // Premium and bundle weapons, scopes, artefacts, money.
        assert!(!sold("ak_74n") && !sold("uzi_premium") && !sold("vityaz"));
        assert!(!sold("scope_leupold") && !sold("artefact_onyx"));
        assert!(!sold("generic_money") && !sold("premium_money"));
        // No cosmetics or vouchers.
        assert!(
            price_list()
                .iter()
                .all(|record| { dictionary.item(record.dict_id).unwrap().category < 25 })
        );
        assert_eq!(buy_cost(214, 1), None);
    }

    #[test]
    fn selling_gives_a_quarter_and_repair_goes_by_the_wear() {
        let dictionary = Dictionary::shared();
        // The AK-74M at full condition and at 40.
        assert_eq!(sell_price(dictionary, 3, 100, 100), 11875);
        assert_eq!(sell_price(dictionary, 3, 40, 100), 4750);
        // Two clips of 5.45: 15 a clip.
        assert_eq!(sell_price(dictionary, 7, 60, 100), 30);
        // A premium item's cost is gold: the Uzi, 500 gold at 100 a gold.
        let uzi = dictionary.item_named("uzi_premium").unwrap().dict_id;
        assert_eq!(sell_price(dictionary, uzi, 100, 100), 12500);
        // Repair: ratio 0.1 of 47500 for the whole, a half for a half.
        assert_eq!(repair_cost(dictionary, 3, 0), 4750);
        assert_eq!(repair_cost(dictionary, 3, 50), 2375);
        assert_eq!(repair_cost(dictionary, 3, 100), 0);
        assert_eq!(repair_cost(dictionary, 7, 0), 0);
    }
}
