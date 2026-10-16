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
//!   the inventory. [srp] Any camouflage fits any such item:
//!   `items_compatibility` pairs none.
//!
//!   A decal is on one item at a time, and putting it on another takes it
//!   off the first (the answer's `detach_first`). [srp, the owner's rule of
//!   2026-10-16] **One camouflage goes on any number of items at once**:
//!   each item wears a *bound copy* of it, an item of the inventory with an
//!   id of its own and [`Item::copy_of`] naming the camouflage, while the
//!   camouflage itself stays free in the stash. Why copies and not the one
//!   id on many items: `46 02` names only the cosmetic, never the item
//!   (`drop_upgrade_item` sends the second of its two, 0x29AAD5), and the
//!   client's `83 46 02` takes it off the first item of its inventory that
//!   holds it (0x277CF0); with an id of its own on each item, taking one
//!   off is never ambiguous. The client needs each copy as an item of its
//!   inventory: the answer's handler does nothing for an upgrade it lacks
//!   (0x27565B), and the upgrade window finds what a slot holds there
//!   (`UpgradeWindow.create_item`); so the stash lists each copy as worn
//!   (`is_attached_to`, the host) beside the free camouflage. A match reads
//!   only the record's dict id (`item_config_cook`, 0x2F18D8), so every
//!   item shows the camouflage there too. Rules: a copy dragged onto
//!   another item moves, as any cosmetic did; a copy that comes off, whose
//!   item goes (sold, used up as a donor) or whose slot gets another
//!   cosmetic is gone; a copy is never sold (`rules::sell`) or anything
//!   but its item's; the camouflage sold takes every copy with it and is
//!   paid once. A camouflage worn itself under the rule before copies stays
//!   on its item; put on another, it leaves a copy in its place first.
//!   The upgrade window keeps the ids it showed until it is opened again,
//!   and its cancel sends `46 02` for what a slot holds and `46 01` for
//!   what it held: the lobby remembers, while it runs, the item each
//!   camouflage was last put on by its own id (a `46 02` naming the
//!   camouflage means that item's copy) and the copies gone (a `46 01`
//!   naming one puts its camouflage on again), [`CopyMemory`].
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

/// Takes off every upgrade whose cosmetic the inventory lacks, after
/// dropping every bound copy of a camouflage that is worn by no item or
/// whose camouflage is gone (sold): a copy is never an item of its own
/// (`Account::sync_kits` calls it after each change of the inventory).
/// Returns the ids of the items that lost one.
pub fn drop_gone_upgrades(inventory: &mut Vec<Item>) -> Vec<u32> {
    let mut stripped = Vec::new();
    if inventory
        .iter()
        .all(|item| item.upgrades.is_empty() && item.copy_of == 0)
    {
        return stripped;
    }
    if inventory.iter().any(|item| item.copy_of != 0) {
        let originals: Vec<(u32, u16)> = inventory
            .iter()
            .filter(|item| item.copy_of == 0)
            .map(|item| (item.item_id, item.dict_id))
            .collect();
        let worn: Vec<u32> = inventory
            .iter()
            .flat_map(|host| {
                host.upgrades
                    .iter()
                    .filter(move |held| held.id != host.item_id)
                    .map(|held| held.id)
            })
            .collect();
        inventory.retain(|item| {
            item.copy_of == 0
                || (modifications::is_camouflage(item.dict_id)
                    && originals.contains(&(item.copy_of, item.dict_id))
                    && worn.contains(&item.item_id))
        });
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
    /// What went on: for a camouflage, the bound copy of it the host wears.
    pub upgrade_item_id: u32,
    pub slot: u8,
    pub value: u64,
    /// The cosmetic was on an item and came off it first.
    pub detach_first: bool,
    /// The upgrade is a bound copy made for this: the client does not hold
    /// it yet, so it must be sent the inventory before the answer (the
    /// answer's handler does nothing for an item it lacks, 0x27565B).
    pub new_copy: bool,
}

/// What the host's `slot` holds, if anything.
fn held_in(account: &Account, host_item_id: u32, slot: u8) -> Option<Upgrade> {
    account
        .item(host_item_id)?
        .upgrades
        .iter()
        .find(|held| held.param == slot)
        .copied()
}

/// The item whose upgrades hold `upgrade_item_id`, if any.
fn host_of(account: &Account, upgrade_item_id: u32) -> Option<u32> {
    account
        .inventory
        .iter()
        .find(|item| item.upgrades.iter().any(|held| held.id == upgrade_item_id))
        .map(|item| item.item_id)
}

/// A bound copy of the camouflage `original` (an item of the inventory):
/// a new item of the same kind. Returns its id.
fn make_copy(account: &mut Account, original: u32) -> u32 {
    let (dict_id, condition) = account
        .item(original)
        .map(|item| (item.dict_id, item.condition_or_stack))
        .expect("the camouflage is an item of the inventory");
    let copy = account.add_item(dict_id, condition);
    account
        .item_mut(copy)
        .expect("the copy was just made")
        .copy_of = original;
    copy
}

/// How many camouflages and gone copies [`CopyMemory`] keeps.
const REMEMBERED: usize = 32;

/// What the upgrade window may still name, kept while the lobby runs. The
/// window keeps the ids it was opened with and the ones dropped on it until
/// it is opened again (`UpgradeWindow.updateData` refills only then), and
/// its cancel sends `46 02` for what a slot holds now and `46 01` for what
/// it held when the window opened (`cancelChanges`). So it may name the
/// camouflage itself after it went on as a copy, and a copy that is gone
/// (replaced in this window). It is no part of what the account is: it is
/// not saved, and accounts are equal whatever each remembers.
#[derive(Clone, Debug, Default)]
pub struct CopyMemory {
    /// `(camouflage, host)`: the item each camouflage was last put on by
    /// its own id, the oldest first.
    put_on: Vec<(u32, u32)>,
    /// `(copy, camouflage)`: bound copies gone, the oldest first.
    gone: Vec<(u32, u32)>,
}

impl PartialEq for CopyMemory {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for CopyMemory {}

impl CopyMemory {
    fn remember(list: &mut Vec<(u32, u32)>, key: u32, value: u32) {
        list.retain(|&(held, _)| held != key);
        list.push((key, value));
        if list.len() > REMEMBERED {
            list.remove(0);
        }
    }

    fn put_on(&mut self, camouflage: u32, host: u32) {
        Self::remember(&mut self.put_on, camouflage, host);
    }

    /// The item `camouflage` was last put on by its own id.
    fn last_put_on(&self, camouflage: u32) -> Option<u32> {
        self.put_on
            .iter()
            .find(|&&(held, _)| held == camouflage)
            .map(|&(_, host)| host)
    }

    fn gone(&mut self, copy: u32, camouflage: u32) {
        Self::remember(&mut self.gone, copy, camouflage);
    }

    /// The camouflage of a bound copy that is gone.
    fn camouflage_of_gone(&self, copy: u32) -> Option<u32> {
        self.gone
            .iter()
            .find(|&&(held, _)| held == copy)
            .map(|&(_, camouflage)| camouflage)
    }
}

/// Notes `item_id`, about to go, if it is a bound copy
/// ([`CopyMemory::gone`]).
fn note_gone(account: &mut Account, item_id: u32) {
    if let Some(camouflage) = account
        .item(item_id)
        .map(|item| item.copy_of)
        .filter(|&camouflage| camouflage != 0)
    {
        account.copy_memory.gone(item_id, camouflage);
    }
}

/// The kits wearing any of `item_ids` show another item: new revisions,
/// one a kit.
fn bump_kits_wearing_any(account: &mut Account, item_ids: &[u32]) {
    for kit in &mut account.kits {
        if item_ids.iter().any(|&id| kit.slot_of(id).is_some()) {
            kit.revision = kit.revision.wrapping_add(1);
        }
    }
}

/// `46 01 <kit> <host> <upgrade> <slot> <u64 0>`: put a camouflage or a
/// decal on an item. What was in the slot comes off.
///
/// A camouflage of its own goes on as a new bound copy of it and stays
/// where it is ([`attach_camouflage`]). A decal, or a bound copy, moves:
/// off the item it was on, onto this one. A bound copy that is gone
/// (the upgrade window's cancel names what its slot held when it opened,
/// [`CopyMemory`]) stands for its camouflage, if that is still owned.
pub fn attach_upgrade(
    account: &mut Account,
    host_item_id: u32,
    upgrade_item_id: u32,
    slot: u8,
) -> Result<Attached, ItemError> {
    let upgrade_item_id = if account.item(upgrade_item_id).is_some() {
        upgrade_item_id
    } else {
        account
            .copy_memory
            .camouflage_of_gone(upgrade_item_id)
            .filter(|&camouflage| {
                account
                    .item(camouflage)
                    .is_some_and(|item| item.copy_of == 0)
            })
            .ok_or(ItemError::Upgrade("no such cosmetic"))?
    };
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
    if modifications::is_camouflage(upgrade_dict) && upgrade.copy_of == 0 {
        return Ok(attach_camouflage(
            account,
            host_item_id,
            upgrade_item_id,
            slot,
        ));
    }
    if let Some(replaced) = held_in(account, host_item_id, slot)
        && replaced.id != upgrade_item_id
    {
        note_gone(account, replaced.id);
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
    // A bound copy that was in the slot is on nothing now: it goes.
    drop_gone_upgrades(&mut account.inventory);
    // Removing the cosmetic changes the old host's appearance too. Refresh
    // every affected kit once, including alternate kits sharing either host.
    bump_kits_wearing_any(account, &changed_hosts);
    Ok(Attached {
        host_item_id,
        upgrade_item_id,
        slot,
        value: 0,
        detach_first,
        new_copy: false,
    })
}

/// A camouflage of its own (`original`) onto the host: a new bound copy
/// of it goes into the slot, and the camouflage stays in the stash, free
/// to go on any number of items more. One the host already wears (itself
/// or a copy of it) is left as it is. A camouflage worn itself by another
/// item, as the rule before copies had it, gives that item a copy in its
/// place first: so the camouflage is again on nothing itself, and a later
/// `46 02` naming it means the copy just made ([`detach_upgrade`]).
fn attach_camouflage(
    account: &mut Account,
    host_item_id: u32,
    original: u32,
    slot: u8,
) -> Attached {
    let dict_id = account
        .item(original)
        .expect("the camouflage was found above")
        .dict_id;
    // The window this was dropped on still names the camouflage: a `46 02`
    // naming it means this host's copy ([`detach_upgrade`]).
    account.copy_memory.put_on(original, host_item_id);
    if let Some(held) = held_in(account, host_item_id, slot) {
        let same = held.id == original
            || account
                .item(held.id)
                .is_some_and(|item| item.copy_of == original);
        if same {
            return Attached {
                host_item_id,
                upgrade_item_id: held.id,
                slot,
                value: held.value,
                detach_first: false,
                new_copy: false,
            };
        }
        note_gone(account, held.id);
    }
    let mut changed_hosts = vec![host_item_id];
    if let Some(wearer) = host_of(account, original) {
        let copy = make_copy(account, original);
        let wearer_item = account.item_mut(wearer).expect("the wearer was found");
        for held in &mut wearer_item.upgrades {
            if held.id == original {
                held.id = copy;
            }
        }
        changed_hosts.push(wearer);
    }
    let copy = make_copy(account, original);
    let host = account
        .item_mut(host_item_id)
        .expect("the host was found above");
    host.upgrades.retain(|held| held.param != slot);
    host.upgrades.push(Upgrade {
        id: copy,
        a: dict_id.into(),
        value: 0,
        param: slot,
    });
    // A bound copy that was in the slot is on nothing now: it goes.
    drop_gone_upgrades(&mut account.inventory);
    bump_kits_wearing_any(account, &changed_hosts);
    Attached {
        host_item_id,
        upgrade_item_id: copy,
        slot,
        value: 0,
        detach_first: false,
        new_copy: true,
    }
}

/// What `46 02` took off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Detached {
    /// The item it was on.
    pub host_item_id: u32,
    /// The cosmetic that came off: what `83 46 02` names.
    pub upgrade_item_id: u32,
    /// It was a bound copy, and is gone from the inventory with it.
    pub copy_dropped: bool,
}

/// `46 02 <kit> <upgrade>`: take a cosmetic off the item it is on. The
/// request names no host (`drop_upgrade_item(item, upgrade)` sends only
/// the second, 0x29AAD5), which is why each item wears a camouflage as a
/// copy with an id of its own; a bound copy that comes off is gone. The
/// camouflage itself, worn by nothing, is named only by the upgrade window
/// it was just put on with (the slot still holds the id that was dragged
/// there, [`CopyMemory`]): the copy on the item it was last put on by its
/// own id comes off, else (nothing remembered) its newest copy.
pub fn detach_upgrade(account: &mut Account, upgrade_item_id: u32) -> Result<Detached, ItemError> {
    let target = if host_of(account, upgrade_item_id).is_some() {
        upgrade_item_id
    } else {
        let worn: Vec<(u32, u32)> = account
            .inventory
            .iter()
            .filter(|item| upgrade_item_id != 0 && item.copy_of == upgrade_item_id)
            .filter_map(|item| Some((item.item_id, host_of(account, item.item_id)?)))
            .collect();
        let window = account.copy_memory.last_put_on(upgrade_item_id);
        worn.iter()
            .find(|&&(_, host)| Some(host) == window)
            .or_else(|| worn.iter().max())
            .map(|&(copy, _)| copy)
            .ok_or(ItemError::Upgrade("the cosmetic is on no item"))?
    };
    note_gone(account, target);
    let host = account
        .inventory
        .iter_mut()
        .find(|item| item.upgrades.iter().any(|held| held.id == target))
        .expect("the host was found above");
    host.upgrades.retain(|held| held.id != target);
    let host_item_id = host.item_id;
    let copy_dropped = account.item(target).is_some_and(|item| item.copy_of != 0);
    if copy_dropped {
        account.inventory.retain(|item| item.item_id != target);
    }
    bump_kits_wearing(account, host_item_id);
    Ok(Detached {
        host_item_id,
        upgrade_item_id: target,
        copy_dropped,
    })
}

/// `46 05 <kit> <host> <upgrade> <u64>`: the decal editor's value for a
/// cosmetic on an item (the client has written it into its own copy, and
/// expects no answer). A camouflage named by its own id stands for the
/// copy of it the host wears.
pub fn place_upgrade(
    account: &mut Account,
    host_item_id: u32,
    upgrade_item_id: u32,
    value: u64,
) -> Result<(), ItemError> {
    let copies: Vec<u32> = account
        .inventory
        .iter()
        .filter(|item| upgrade_item_id != 0 && item.copy_of == upgrade_item_id)
        .map(|item| item.item_id)
        .collect();
    let held = account
        .item_mut(host_item_id)
        .and_then(|host| {
            let index = host
                .upgrades
                .iter()
                .position(|held| held.id == upgrade_item_id)
                .or_else(|| {
                    host.upgrades
                        .iter()
                        .position(|held| copies.contains(&held.id))
                })?;
            host.upgrades.get_mut(index)
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
