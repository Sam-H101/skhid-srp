//! What the lobby does to one item: its cosmetics ("upgrades": a
//! camouflage and decals), changing a modification ("reroll"), upgrading it
//! with a donor ("craft"), the whole repair with its defects, and what a
//! match and a drop do to it. The data is the game's
//! ([`survarium::v026g::modifications`]); the requests and answers are in
//! `docs/notes/lobby_requests.md` (`46 01`, `02`, `05`..`09`,
//! `47 03`).
//!
//! What the client shows and checks itself (PROVEN, `lobby_menu.swf` and
//! the exe at the addresses the notes give) is kept as it is. The rest is
//! the server's, and where the data does not say, srp's own choice, marked
//! [srp]:
//!
//! - **Cosmetics.** A camouflage goes into slot 3 and a decal into slot 0
//!   or 1 of an item whose config has `camo_available` /
//!   `decal_available` (vests and trousers). The cosmetic stays an item of
//!   the inventory; one is on one item at a time, and putting it on
//!   another takes it off the first (the answer's `detach_first`). [srp]
//!   Any camouflage fits any such item: `items_compatibility` pairs none.
//! - **Reroll.** One modification of an item may be changed, any number of
//!   times (`st_modifier_reroll_description`); the first change picks which
//!   (the window locks the others, `was_rerolled_before`). A change offers
//!   three modifications and the player takes one
//!   (`MessageRerollItem`: the window can not be closed before). Paid in
//!   silver it can be done once in six hours
//!   (`st_modifier_reroll_timer_description`; the window disables silver
//!   while the item's `reroll_time` runs), in gold at any time. A damaged
//!   item can not be changed (`st_reroll_message_broken_item`). The prices
//!   are `reroll.options`' by item level, [GUESS] times
//!   `reroll_modificator_multiplicator` for each change made
//!   ([`Reroll::price`](survarium::v026g::modifications::Reroll::price)).
//!   [srp] The payment is taken when the three are offered; an offer not
//!   answered (the client went away) is kept and shown again.
//! - **Craft.** The item and a donor: neither damaged nor in repair, the
//!   item with fewer than three modifications and no fewer than the donor
//!   (`MessageCraftItem`). The price is `craft.options`' by the item's
//!   level; the chances by how many more modifications the item has than
//!   the donor, in silver or in gold: one more modification
//!   (`st_item_craft_success_label`), one fewer
//!   (`st_item_craft_fail_label`), or nothing. The donor is used up
//!   whatever happens (the answer names it, and the client drops it).
//!   [LIKELY] The donor is another of the same item: the client's result
//!   window shows the *donor's* dictionary entry as the upgraded item
//!   (0x275B84 → 0x29F770 `dict_id`). [srp] The modification gained is
//!   rolled as a drop's are, the one lost is any of the item's own.
//! - **Repair.** In silver: what the window shows,
//!   `(1 - condition) * repair_cost_ratio * cost` and each defect's
//!   `fix_cost_general`, and the item is locked for the defects'
//!   `fix_time_general` together (six minutes each); with no defect it is
//!   done at once. In gold ("quick repair"): the defects' `fix_cost_gold`,
//!   at once. With repair kits (the voucher `repair_voucher`): one a
//!   defect, at once. The client offers gold and kits only for an item with
//!   defects. Every repair makes the item whole (the client sets its
//!   condition to full on the answer) and [srp] takes its defects off at
//!   once: the wait only locks the item in the lobby. When it ends the
//!   client is told (`#ir[<item>]`, an important message of type 6).
//! - **Defects.** [LIKELY, the reading of `defect_slots` in
//!   `modifications`] After a match that wore an item, one roll for its
//!   first free defect slot. `SRP_ITEM_DEFECTS=off` switches them off.
//! - **Drops.** A dropped weapon or armour of rarity `n` has `n`
//!   modifications (the lobby's rank is their count).

use survarium::v026g::dictionary::Dictionary;
use survarium::v026g::drops::Rng;
use survarium::v026g::item_params::item_params;
use survarium::v026g::lobby::requests::RepairPayment;
use survarium::v026g::lobby::{Crafted, FULL_CONDITION, Repaired, limits};
use survarium::v026g::modifications::{
    self, Bonuses, CraftOutcome, MAX_RANK, Modifications, REROLL_WAIT_SECONDS,
};
use survarium::v026g::prices;

use super::account::{Account, Item, Upgrade};
use super::rules::shown_condition;

/// The important message that tells the client a repair is over
/// (`social.md` §1.4, type 6; the text is `#ir[<item id>]`, 0x226848).
pub const REPAIRED_MESSAGE: u8 = 6;
/// The setting that switches defects off.
pub const DEFECTS_VAR: &str = "SRP_ITEM_DEFECTS";

/// Why an item action is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemError {
    /// A cosmetic can not go on, come off or be placed; why, for the log.
    Upgrade(&'static str),
    /// A modification can not be changed or chosen; why, for the log.
    Reroll(&'static str),
    /// The item has defects.
    RerollBroken,
    NotEnoughMoney,
    NotEnoughGold,
    /// Nothing to repair, or not that way; why, for the log.
    Repair(&'static str),
    /// Fewer repair kits than defects.
    NoRepairKit,
    NotEnoughMoneyForRepair,
}

impl ItemError {
    /// The game's own text for the refusal: a key of its localization.
    pub fn text_key(&self) -> &'static str {
        match self {
            Self::Upgrade(_) => "st_cannot_upgrade_item",
            Self::Reroll(_) => "st_cannot_select_rerolled_modification",
            Self::RerollBroken => "st_reroll_message_broken_item",
            Self::NotEnoughMoney => "st_not_enougth_money",
            Self::NotEnoughGold => "st_cant_buy_premium_message",
            Self::Repair(_) => "st_cannot_repair_item",
            Self::NoRepairKit => "st_you_have_no_voucher_for_repair",
            Self::NotEnoughMoneyForRepair => "st_not_enought_money_for_repair",
        }
    }
}

impl std::fmt::Display for ItemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?} ({})", self.text_key())
    }
}

impl std::error::Error for ItemError {}

/// Whether defects appear ([`DEFECTS_VAR`]; on unless `off`, `0`, `no` or
/// `false`).
pub fn defects_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        let value = std::env::var(DEFECTS_VAR).unwrap_or_default();
        !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "no" | "false"
        )
    })
}

// ------------------------------------------------------------ the item

/// The item's own modifications (not its defects, not its defaults).
pub fn own_modifications(item: &Item) -> Vec<u32> {
    item.modifications
        .iter()
        .filter(|&&(_, state)| !state)
        .map(|&(id, _)| id)
        .collect()
}

/// The item's defects.
pub fn defects(item: &Item) -> Vec<u32> {
    item.modifications
        .iter()
        .filter(|&&(_, state)| state)
        .map(|&(id, _)| id)
        .collect()
}

/// The rank the lobby shows: how many modifications of its own it has.
pub fn rank(item: &Item) -> usize {
    item.modifications
        .iter()
        .filter(|&&(_, state)| !state)
        .count()
}

/// The seconds from `now` until the Unix second `until`; 0 once it is
/// over, or for none.
pub fn seconds_left(until: u32, now: u64) -> u32 {
    u64::from(until)
        .saturating_sub(now)
        .min(u64::from(u32::MAX)) as u32
}

fn until(now: u64, seconds: u32) -> u32 {
    now.saturating_add(u64::from(seconds))
        .min(u64::from(u32::MAX)) as u32
}

/// Takes off every upgrade whose cosmetic the inventory lacks
/// (`Account::sync_kits` calls it after each change of the inventory).
/// Returns the ids of the items that lost one.
pub fn drop_gone_upgrades(inventory: &mut [Item]) -> Vec<u32> {
    let mut stripped = Vec::new();
    if inventory.iter().all(|item| item.upgrades.is_empty()) {
        return stripped;
    }
    let owned: Vec<u32> = inventory.iter().map(|item| item.item_id).collect();
    for item in inventory.iter_mut() {
        let before = item.upgrades.len();
        let host = item.item_id;
        item.upgrades
            .retain(|upgrade| upgrade.id != host && owned.contains(&upgrade.id));
        if item.upgrades.len() != before {
            stripped.push(host);
        }
    }
    stripped
}

/// Makes a loaded item one the client can be sent: no modification or
/// defect its tables lack (it reads the row it did not find), no more of
/// its own than three, no offer of what it lacks, and upgrades that are in
/// a slot of their kind, one a slot.
pub fn make_safe(item: &mut Item) {
    let table = Modifications::shared();
    item.modifications
        .retain(|&(id, state)| table.is_known(id, state));
    let mut seen: Vec<(u32, bool)> = Vec::new();
    item.modifications.retain(|entry| {
        let fresh = !seen.contains(entry);
        seen.push(*entry);
        fresh
    });
    let mut own = 0;
    item.modifications.retain(|&(_, state)| {
        own += usize::from(!state);
        state || own <= MAX_RANK
    });
    let held = own_modifications(item);
    if let Some((replaced, choices)) = item.reroll.offer {
        let good =
            held.contains(&replaced) && choices.iter().all(|&choice| table.is_known(choice, false));
        if !good {
            item.reroll.offer = None;
        }
    }
    if item.reroll.modification != 0 && !held.contains(&item.reroll.modification) {
        item.reroll.modification = 0;
    }
    let host = item.dict_id;
    let mut slots: Vec<u8> = Vec::new();
    item.upgrades.retain(|upgrade| {
        let dict_id = u16::try_from(upgrade.a).unwrap_or(0);
        let fits = modifications::upgrade_slots(host, dict_id).contains(&upgrade.param)
            && !slots.contains(&upgrade.param);
        slots.push(upgrade.param);
        fits
    });
}

/// The kits wearing `item_id` show another item: new revisions.
fn bump_kits_wearing(account: &mut Account, item_id: u32) {
    for kit in &mut account.kits {
        if kit.slot_of(item_id).is_some() {
            kit.revision = kit.revision.wrapping_add(1);
        }
    }
}

fn is_gear(item: &Item) -> bool {
    !Dictionary::shared().is_stack(item.dict_id)
        && modifications::item_mods(item.dict_id).is_some_and(|mods| mods.modifications != 0)
}

fn item_level(dict_id: u16) -> u8 {
    item_params(dict_id).map_or(0, |params| params.item_level)
}

// ----------------------------------------------------------- cosmetics

/// What `83 46 01` carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attached {
    pub host_item_id: u32,
    pub upgrade_item_id: u32,
    pub slot: u8,
    pub value: u64,
    /// The cosmetic was on an item and came off it first.
    pub detach_first: bool,
}

/// `46 01 <kit> <host> <upgrade> <slot> <u64 0>`: put a camouflage or a
/// decal on an item. What was in the slot comes off.
pub fn attach_upgrade(
    account: &mut Account,
    host_item_id: u32,
    upgrade_item_id: u32,
    slot: u8,
) -> Result<Attached, ItemError> {
    if host_item_id == upgrade_item_id {
        return Err(ItemError::Upgrade("an item on itself"));
    }
    let host = account
        .item(host_item_id)
        .ok_or(ItemError::Upgrade("no such item"))?;
    let upgrade = account
        .item(upgrade_item_id)
        .ok_or(ItemError::Upgrade("no such cosmetic"))?;
    let upgrade_dict = upgrade.dict_id;
    if !modifications::upgrade_slots(host.dict_id, upgrade_dict).contains(&slot) {
        return Err(ItemError::Upgrade("it does not go into that slot"));
    }
    // What the host keeps: all but what is in the slot and the cosmetic
    // itself (moved from another slot).
    let kept = host
        .upgrades
        .iter()
        .filter(|held| held.param != slot && held.id != upgrade_item_id)
        .count();
    if kept >= limits::UPGRADES {
        return Err(ItemError::Upgrade("the item holds no more"));
    }
    let mut detach_first = false;
    let mut changed_hosts = vec![host_item_id];
    for item in &mut account.inventory {
        let before = item.upgrades.len();
        item.upgrades.retain(|held| held.id != upgrade_item_id);
        if item.upgrades.len() != before {
            detach_first = true;
            changed_hosts.push(item.item_id);
        }
    }
    let host = account
        .item_mut(host_item_id)
        .expect("the host was found above");
    host.upgrades.retain(|held| held.param != slot);
    host.upgrades.push(Upgrade {
        id: upgrade_item_id,
        a: upgrade_dict.into(),
        value: 0,
        param: slot,
    });
    // Removing the cosmetic changes the old host's appearance too. Refresh
    // every affected kit once, including alternate kits sharing either host.
    for kit in &mut account.kits {
        if changed_hosts.iter().any(|&id| kit.slot_of(id).is_some()) {
            kit.revision = kit.revision.wrapping_add(1);
        }
    }
    Ok(Attached {
        host_item_id,
        upgrade_item_id,
        slot,
        value: 0,
        detach_first,
    })
}

/// `46 02 <kit> <upgrade>`: take a cosmetic off the item it is on. Returns
/// that item's id.
pub fn detach_upgrade(account: &mut Account, upgrade_item_id: u32) -> Result<u32, ItemError> {
    let host = account
        .inventory
        .iter_mut()
        .find(|item| item.upgrades.iter().any(|held| held.id == upgrade_item_id))
        .ok_or(ItemError::Upgrade("the cosmetic is on no item"))?;
    host.upgrades.retain(|held| held.id != upgrade_item_id);
    let host_item_id = host.item_id;
    bump_kits_wearing(account, host_item_id);
    Ok(host_item_id)
}

/// `46 05 <kit> <host> <upgrade> <u64>`: the decal editor's value for a
/// cosmetic on an item (the client has written it into its own copy, and
/// expects no answer).
pub fn place_upgrade(
    account: &mut Account,
    host_item_id: u32,
    upgrade_item_id: u32,
    value: u64,
) -> Result<(), ItemError> {
    let held = account
        .item_mut(host_item_id)
        .and_then(|host| {
            host.upgrades
                .iter_mut()
                .find(|held| held.id == upgrade_item_id)
        })
        .ok_or(ItemError::Upgrade("the cosmetic is not on that item"))?;
    held.value = value;
    bump_kits_wearing(account, host_item_id);
    Ok(())
}

// -------------------------------------------------------------- reroll

/// What the reroll window of an item shows (`83 46 06`, and the tail of
/// `83 46 08`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RerollWindow {
    pub item_id: u32,
    /// What the next change costs, in silver and in gold.
    pub regular_cost: u32,
    pub premium_cost: u32,
    /// Once a modification was changed: its id (the one that may be
    /// changed again) and the seconds until silver pays again.
    pub changed: Option<(u32, u32)>,
    /// A paid change that waits for its choice (`83 46 07` again).
    pub offer: Option<[u32; 3]>,
}

fn window_of(item: &Item, now: u64) -> RerollWindow {
    let reroll = &Modifications::shared().reroll;
    let level = item_level(item.dict_id);
    let wait = seconds_left(item.reroll_time, now);
    RerollWindow {
        item_id: item.item_id,
        regular_cost: reroll.price(level, false, item.reroll.count),
        premium_cost: reroll.price(level, true, item.reroll.count),
        changed: (item.reroll.modification != 0 || wait != 0)
            .then_some((item.reroll.modification, wait)),
        offer: item.reroll.offer.map(|(_, choices)| choices),
    }
}

/// `46 06 <item>`: the reroll window was opened.
pub fn reroll_window(account: &Account, item_id: u32, now: u64) -> Result<RerollWindow, ItemError> {
    let item = account
        .item(item_id)
        .ok_or(ItemError::Reroll("no such item"))?;
    Ok(window_of(item, now))
}

/// What a reroll did (`83 46 07`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rerolled {
    pub choices: [u32; 3],
    /// The item's `reroll_time`: the seconds until silver pays again.
    pub reroll_seconds: u32,
    /// What was taken: silver, gold. Both 0 for an offer made before and
    /// shown again.
    pub paid: (u32, u32),
}

/// `46 07 <item> <modification> <gold>`: change `modification` of the
/// item; three are offered in its place.
pub fn reroll(
    account: &mut Account,
    item_id: u32,
    modification: u32,
    gold: bool,
    now: u64,
    rng: &mut Rng,
) -> Result<Rerolled, ItemError> {
    let item = account
        .item(item_id)
        .ok_or(ItemError::Reroll("no such item"))?;
    if let Some((_, choices)) = item.reroll.offer {
        // Paid for and not chosen yet: the same three.
        return Ok(Rerolled {
            choices,
            reroll_seconds: seconds_left(item.reroll_time, now),
            paid: (0, 0),
        });
    }
    if !is_gear(item) {
        return Err(ItemError::Reroll("not an item with modifications"));
    }
    if !defects(item).is_empty() {
        return Err(ItemError::RerollBroken);
    }
    if seconds_left(item.repair_time, now) != 0 {
        return Err(ItemError::Reroll("the item is in repair"));
    }
    let held = own_modifications(item);
    if !held.contains(&modification) {
        return Err(ItemError::Reroll("the item has no such modification"));
    }
    if item.reroll.modification != 0 && item.reroll.modification != modification {
        return Err(ItemError::Reroll(
            "another modification is the one to change",
        ));
    }
    if !gold && seconds_left(item.reroll_time, now) != 0 {
        return Err(ItemError::Reroll("silver pays once in six hours"));
    }
    let table = Modifications::shared();
    let price = table
        .reroll
        .price(item_level(item.dict_id), gold, item.reroll.count);
    let balance = if gold {
        account.premium_money
    } else {
        account.generic_money
    };
    if price > balance {
        return Err(if gold {
            ItemError::NotEnoughGold
        } else {
            ItemError::NotEnoughMoney
        });
    }
    let choices = table
        .reroll_choices(item.dict_id, &held, modification, rng)
        .ok_or(ItemError::Reroll("nothing to offer for the item"))?;
    let paid = if gold {
        account.premium_money -= price;
        (0, price)
    } else {
        account.generic_money -= price;
        (price, 0)
    };
    let item = account.item_mut(item_id).expect("the item was found above");
    item.reroll.modification = modification;
    item.reroll.count = item.reroll.count.saturating_add(1);
    item.reroll.offer = Some((modification, choices));
    if !gold {
        item.reroll_time = until(now, REROLL_WAIT_SECONDS);
    }
    Ok(Rerolled {
        choices,
        reroll_seconds: seconds_left(item.reroll_time, now),
        paid,
    })
}

/// What a choice did (`83 46 08`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chosen {
    pub old_modification: u32,
    pub new_modification: u32,
    /// The window as it is now.
    pub window: RerollWindow,
}

/// `46 08 <item> <old modification> <choice>`: take one of the three
/// offered; `choice` is its place among them (the client sends the index
/// of the row clicked, `MessageRerollItem.onMessageAccept`).
pub fn choose_modifier(
    account: &mut Account,
    item_id: u32,
    old_modification: u32,
    choice: u32,
    now: u64,
) -> Result<Chosen, ItemError> {
    let item = account
        .item_mut(item_id)
        .ok_or(ItemError::Reroll("no such item"))?;
    let (replaced, choices) = item
        .reroll
        .offer
        .ok_or(ItemError::Reroll("nothing was offered"))?;
    if replaced != old_modification {
        return Err(ItemError::Reroll("another modification is being changed"));
    }
    let new_modification = *choices
        .get(choice as usize)
        .ok_or(ItemError::Reroll("no such choice"))?;
    let held = item
        .modifications
        .iter_mut()
        .find(|entry| **entry == (replaced, false))
        .ok_or(ItemError::Reroll("the item lost the modification"))?;
    held.0 = new_modification;
    item.reroll.modification = new_modification;
    item.reroll.offer = None;
    let window = window_of(item, now);
    bump_kits_wearing(account, item_id);
    Ok(Chosen {
        old_modification,
        new_modification,
        window,
    })
}

// --------------------------------------------------------------- craft

/// `46 09 <item> <donor> <gold>`: upgrade an item with a donor. `Err` is
/// why it is refused, for the log: the client is told with
/// [`Crafted::Failed`] (its window waits for an `83 46 09`). `queued`:
/// the account waits for a match (`st_craft_message_equipped_item`).
pub fn craft(
    account: &mut Account,
    item_id: u32,
    donor_item_id: u32,
    gold: bool,
    queued: bool,
    now: u64,
    rng: &mut Rng,
) -> Result<Crafted, &'static str> {
    if item_id == donor_item_id {
        return Err("an item is its own donor");
    }
    let item = account.item(item_id).ok_or("no such item")?;
    let donor = account.item(donor_item_id).ok_or("no such donor")?;
    if !is_gear(item) || !is_gear(donor) {
        return Err("not an item with modifications");
    }
    if donor.dict_id != item.dict_id {
        return Err("the donor is another item");
    }
    if !defects(item).is_empty() || !defects(donor).is_empty() {
        return Err("a damaged item");
    }
    if seconds_left(item.repair_time, now) != 0 || seconds_left(donor.repair_time, now) != 0 {
        return Err("an item in repair");
    }
    if item.reroll.offer.is_some() {
        return Err("a modification of the item is being changed");
    }
    let (held, donor_rank) = (own_modifications(item), rank(donor));
    if held.len() >= MAX_RANK || donor_rank >= MAX_RANK {
        return Err("the most modifications already");
    }
    if donor_rank > held.len() {
        return Err("the donor has more modifications");
    }
    let equipped = |id: u32| account.kits.iter().any(|kit| kit.slot_of(id).is_some());
    if queued && (equipped(item_id) || equipped(donor_item_id)) {
        return Err("an equipped item while waiting for a match");
    }
    let table = Modifications::shared();
    let dict_id = item.dict_id;
    let price = table.craft.price(item_level(dict_id), gold);
    let balance = if gold {
        account.premium_money
    } else {
        account.generic_money
    };
    if price > balance {
        return Err("not enough money");
    }
    let at = rng.below(100_000) as f32 / 1000.0;
    let outcome = table
        .craft
        .outcome_at(held.len() - donor_rank, gold, at)
        .ok_or("no chances for that difference")?;
    // The modification gained or lost, before anything changes.
    let change = match outcome {
        CraftOutcome::Success => table.roll(dict_id, &held, rng),
        CraftOutcome::Failure if !held.is_empty() => {
            Some(held[rng.below(held.len() as u32) as usize])
        }
        _ => None,
    };

    if gold {
        account.premium_money -= price;
    } else {
        account.generic_money -= price;
    }
    account
        .inventory
        .retain(|item| item.item_id != donor_item_id);
    let item = account.item_mut(item_id).expect("the item was found above");
    let crafted = match (outcome, change) {
        (CraftOutcome::Success, Some(modification)) => {
            item.modifications.push((modification, false));
            Crafted::Added {
                consumed_item_id: donor_item_id,
                modification,
            }
        }
        (CraftOutcome::Failure, Some(modification)) => {
            if let Some(index) = item
                .modifications
                .iter()
                .position(|&entry| entry == (modification, false))
            {
                item.modifications.remove(index);
            }
            if item.reroll.modification == modification {
                item.reroll.modification = 0;
            }
            Crafted::Removed {
                mode: 2,
                consumed_item_id: donor_item_id,
                modification,
            }
        }
        _ => Crafted::Nothing {
            consumed_item_id: donor_item_id,
        },
    };
    bump_kits_wearing(account, item_id);
    account.sync_kits();
    Ok(crafted)
}

// -------------------------------------------------------------- repair

/// What the repair window shows for the item: `(silver, gold, seconds,
/// defects)`.
pub fn repair_quote(item: &Item) -> (u32, u32, u32, usize) {
    let table = Modifications::shared();
    let wear = prices::repair_cost(
        Dictionary::shared(),
        item.dict_id,
        shown_condition(item.condition_or_stack),
    );
    let rows: Vec<_> = defects(item)
        .into_iter()
        .filter_map(|id| table.defect(id))
        .collect();
    let sum = |value: fn(&modifications::Defect) -> u32| {
        rows.iter()
            .fold(0_u32, |sum, row| sum.saturating_add(value(row)))
    };
    (
        wear.saturating_add(sum(|row| row.fix_cost_general)),
        sum(|row| row.fix_cost_gold),
        sum(|row| row.fix_time_general),
        rows.len(),
    )
}

/// `47 03 <item> <kind> [<kit item>]`: repair an item whole. Returns what
/// `83 47 03` carries.
pub fn repair(
    account: &mut Account,
    item_id: u32,
    payment: RepairPayment,
    now: u64,
) -> Result<Repaired, ItemError> {
    let dictionary = Dictionary::shared();
    let item = account
        .item(item_id)
        .ok_or(ItemError::Repair("no such item"))?;
    if dictionary.is_stack(item.dict_id) {
        return Err(ItemError::Repair("a stack"));
    }
    if seconds_left(item.repair_time, now) != 0 {
        return Err(ItemError::Repair("already in repair"));
    }
    let (silver, gold, seconds, broken) = repair_quote(item);
    if broken == 0 && item.condition_or_stack >= FULL_CONDITION {
        return Err(ItemError::Repair("nothing to repair"));
    }
    let repaired = match payment {
        RepairPayment::Silver => {
            if silver > account.generic_money {
                return Err(ItemError::NotEnoughMoneyForRepair);
            }
            account.generic_money -= silver;
            if seconds == 0 {
                Repaired::AtOnce
            } else {
                Repaired::Timed {
                    repair_time: seconds,
                }
            }
        }
        // Gold and kits mend defects: the window offers neither without.
        RepairPayment::Gold => {
            if broken == 0 {
                return Err(ItemError::Repair("no defects to pay gold for"));
            }
            if gold > account.premium_money {
                return Err(ItemError::NotEnoughGold);
            }
            account.premium_money -= gold;
            Repaired::AtOnce
        }
        RepairPayment::Kit { kit_item_id } => {
            if broken == 0 {
                return Err(ItemError::Repair("no defects to use a kit on"));
            }
            let used = broken as u32;
            let index = account
                .inventory
                .iter()
                .position(|kit| {
                    kit.item_id == kit_item_id
                        && item_params(kit.dict_id).is_some_and(|params| params.kind == "vch_fixr")
                })
                .ok_or(ItemError::NoRepairKit)?;
            let kit = &mut account.inventory[index];
            if kit.condition_or_stack < used {
                return Err(ItemError::NoRepairKit);
            }
            kit.condition_or_stack -= used;
            if kit.condition_or_stack == 0 {
                account.inventory.remove(index);
            }
            Repaired::WithKit {
                kit_item_id,
                used: used as u8,
            }
        }
        RepairPayment::Other(_) => return Err(ItemError::Repair("no such payment")),
    };
    let item = account.item_mut(item_id).expect("the item was found above");
    item.condition_or_stack = FULL_CONDITION;
    item.modifications.retain(|&(_, state)| !state);
    item.repair_time = match repaired {
        Repaired::Timed { repair_time } => until(now, repair_time),
        _ => 0,
    };
    bump_kits_wearing(account, item_id);
    account.sync_kits();
    Ok(repaired)
}

/// The repairs that are over at `now`: their timers are cleared, and the
/// ids returned are what the client is told ([`repaired_notice`]).
pub fn finish_repairs(account: &mut Account, now: u64) -> Vec<u32> {
    let mut done = Vec::new();
    for item in &mut account.inventory {
        if item.repair_time != 0 && u64::from(item.repair_time) <= now {
            item.repair_time = 0;
            done.push(item.item_id);
        }
        if item.reroll_time != 0 && u64::from(item.reroll_time) <= now {
            item.reroll_time = 0;
        }
    }
    done
}

/// The important message for a repair that is over: `(type, text)`.
pub fn repaired_notice(item_id: u32) -> (u8, String) {
    (REPAIRED_MESSAGE, format!("#ir[{item_id}]"))
}

/// Makes every weapon and armour piece of the account whole: full
/// condition, no defect, no repair running. Returns how many needed it.
/// For the day wear was switched off (`pay::item_wear`): what matches had
/// worn until then would stay worn for ever.
pub fn mend_all(account: &mut Account) -> usize {
    let dictionary = Dictionary::shared();
    let mut mended = Vec::new();
    for item in &mut account.inventory {
        let gear = !dictionary.is_stack(item.dict_id)
            && item_params(item.dict_id).is_some_and(|params| {
                params.kind.starts_with("wpn_") || params.kind.starts_with("arm_")
            });
        let broken = item.modifications.iter().any(|&(_, state)| state);
        if !gear || (item.condition_or_stack >= FULL_CONDITION && !broken && item.repair_time == 0)
        {
            continue;
        }
        item.condition_or_stack = FULL_CONDITION;
        item.modifications.retain(|&(_, state)| !state);
        item.repair_time = 0;
        mended.push(item.item_id);
    }
    for &item_id in &mended {
        bump_kits_wearing(account, item_id);
    }
    if !mended.is_empty() {
        account.sync_kits();
    }
    mended.len()
}

// ------------------------------------------------- matches and drops

/// Gives a new item its modifications by the rarity a drop rolled; returns
/// the rank the lobby shows for it (the item's `base_rarity_level` and
/// its modifications, 3 at most).
pub fn roll_modifications(account: &mut Account, item_id: u32, rarity: u8, rng: &mut Rng) -> u8 {
    let Some(item) = account.item_mut(item_id) else {
        return 0;
    };
    let rolled = Modifications::shared().roll_for_rarity(item.dict_id, rarity, rng);
    item.modifications
        .extend(rolled.into_iter().map(|id| (id, false)));
    let base = item_params(item.dict_id).map_or(0, |params| params.base_rarity_level);
    (usize::from(base) + rank(item)).min(MAX_RANK) as u8
}

/// What the item's wear is multiplied by: its "reliability"
/// modifications (5 % to 25 % less; a premium weapon has one as a
/// default).
pub fn wear_factor(account: &Account, item_id: u32) -> f32 {
    account.item(item_id).map_or(1.0, |item| {
        Modifications::shared().wear_factor(item.dict_id, &item.modifications, "degrade_per_match")
    })
}

/// The defect rolls for the items a match wore: `worn` is `(item id,
/// condition before, after)`. Returns `(item id, defect)` for each that
/// got one. Nothing with [`DEFECTS_VAR`] off.
pub fn after_wear(
    account: &mut Account,
    worn: &[(u32, u32, u32)],
    rng: &mut Rng,
) -> Vec<(u32, u32)> {
    if !defects_enabled() {
        return Vec::new();
    }
    roll_defects(account, worn, rng)
}

/// [`after_wear`] whatever the setting.
pub fn roll_defects(
    account: &mut Account,
    worn: &[(u32, u32, u32)],
    rng: &mut Rng,
) -> Vec<(u32, u32)> {
    let table = Modifications::shared();
    let mut broke = Vec::new();
    for &(item_id, before, after) in worn {
        if after >= before {
            continue;
        }
        let Some(item) = account.item_mut(item_id) else {
            continue;
        };
        if item.modifications.len() >= limits::MODIFICATIONS {
            continue;
        }
        let condition = after as f32 / FULL_CONDITION as f32;
        let rolled = table.roll_defect(
            item.dict_id,
            &own_modifications(item),
            &defects(item),
            condition,
            rng,
        );
        if let Some(defect) = rolled {
            item.modifications.push((defect, true));
            broke.push((item_id, defect));
        }
    }
    for &(item_id, _) in &broke {
        bump_kits_wearing(account, item_id);
    }
    broke
}

/// What the items of the kit at `kit_index` add to a match's pay (the
/// premium copies of weapons and armour: 10 % silver, 5 % experience, 5 %
/// reputation with one faction).
pub fn kit_bonuses(account: &Account, kit_index: usize) -> Bonuses {
    let table = Modifications::shared();
    let mut total = Bonuses::default();
    let Some(kit) = account.kits.get(kit_index) else {
        return total;
    };
    // An item in two slots is worn once.
    let mut seen: Vec<u32> = Vec::new();
    for slot in kit.slots.iter().flatten() {
        if seen.contains(&slot.item_id) {
            continue;
        }
        seen.push(slot.item_id);
        let Some(item) = account.item(slot.item_id) else {
            continue;
        };
        let bonuses = table.bonuses(item.dict_id, &item.modifications);
        total.money += bonuses.money;
        total.experience += bonuses.experience;
        for (sum, value) in total.reputation.iter_mut().zip(bonuses.reputation) {
            *sum += value;
        }
    }
    total
}

#[cfg(test)]
mod tests;
