//! What a player has left of their supplies, from a world snapshot
//! (`../srp-021d/docs/notes/supplies.md`).
//!
//! A player's block starts with `player_serialized_state` (`base_player::
//! serialize` [V] 0x1D5E96), u16-prefixed: the pools of the slots with one,
//! 23 bytes of movement, the inventory's own state (`inventory::serialize`
//! [V] 0x1FB09E, variable), then each carried item by ascending slot. The
//! block ends in a bitstream with no byte length, so only the first block can
//! be stepped to: instead every offset is tried. Several can parse (an
//! enclosing block's length also reaches the items, with some other bytes as
//! its "pools"); the real state is the one whose first item comes right
//! after its fixed part, and it must be the only one that does.

use std::collections::BTreeMap;

use survarium::dictionary::Dictionary;
use survarium::v021d::supplies::{Carried, Kind, Loadout};

/// A magazine larger than this is not a magazine.
const MAGAZINE_MAX: u16 = 200;
/// The most bytes the inventory's own state (`inventory::serialize`, a few
/// bytes, more with a carried battery) takes between the 23 fixed bytes and
/// the first item. The real state in the snapshots has 10.
const INVENTORY_MAX: usize = 64;

/// What the player at `roster` has left of each stack item (pool and in
/// hand, a loaded magazine counted to its ammo), by dict id; None when the
/// snapshot doesn't show it unambiguously.
pub fn left(
    body: &[u8],
    roster: u8,
    loadout: &Loadout,
    dictionary: &Dictionary,
) -> Option<BTreeMap<u16, u32>> {
    let carried = &loadout.carried;
    if !carried
        .iter()
        .any(|c| matches!(c.kind, Kind::Stack | Kind::Drug))
    {
        return Some(BTreeMap::new());
    }
    // `(gap, pools, items)` of each offset that parses.
    let mut parsed = Vec::new();
    for p in 4..body.len().saturating_sub(1) {
        let length = u16::from_le_bytes([body[p], body[p + 1]]) as usize;
        let Some(state) = body.get(p + 2..p + 2 + length) else {
            continue;
        };
        if let Some(found) = parse(state, roster, carried) {
            parsed.push(found);
        }
    }
    parsed.sort_by_key(|found| found.0);
    let (gap, pools, items) = parsed.first()?;
    if *gap > INVENTORY_MAX || parsed.get(1).is_some_and(|next| next.0 == *gap) {
        return None;
    }

    let mut left = BTreeMap::new();
    for (index, c) in carried.iter().enumerate() {
        if matches!(c.kind, Kind::Stack | Kind::Drug) {
            *left.entry(c.dict_id).or_insert(0) += pools[index] + u32::from(items[index].amount);
        }
    }
    for (c, item) in carried.iter().zip(items) {
        let Some(loaded) = item.loaded else { continue };
        let ammo = carried.iter().find(|a| {
            a.kind == Kind::Stack
                && (dictionary.compatibility.contains(&(a.dict_id, c.dict_id))
                    || dictionary.compatibility.contains(&(c.dict_id, a.dict_id)))
        });
        if let Some(ammo) = ammo {
            *left.entry(ammo.dict_id).or_insert(0) += loaded;
        }
    }
    Some(left)
}

/// Slots whose pool `base_player::serialize` writes
/// (`account::slot_carries_amount`).
fn has_pool(slot: u8) -> bool {
    slot > 7 && slot != 10
}

#[derive(Clone, Copy, Debug)]
struct Item {
    /// Where its header is in the state.
    at: usize,
    amount: u16,
    /// A weapon's magazine, plus a chambered round.
    loaded: Option<u32>,
}

/// A state's pools (0 for slots without one) and items, by carried index,
/// and the bytes between its fixed part and its first item.
fn parse(state: &[u8], roster: u8, carried: &[Carried]) -> Option<(usize, Vec<u32>, Vec<Item>)> {
    let mut pools = vec![0; carried.len()];
    let mut offset = 0;
    for (index, c) in carried.iter().enumerate() {
        if has_pool(c.slot) {
            let pool = u32::from_le_bytes(state.get(offset..offset + 4)?.try_into().ok()?);
            if pool > c.pool {
                return None;
            }
            pools[index] = pool;
            offset += 4;
        }
    }
    let mut items = Vec::with_capacity(carried.len());
    if !search(state, roster, carried, offset + 23, &mut items) {
        return None;
    }
    let gap = items.first()?.at - (offset + 23);
    Some((gap, pools, items))
}

/// The item header `inventory_item::serialize` [V] 0x1E6F9F writes: u16 in
/// hand, u8 owner, u8 slot.
fn header(state: &[u8], at: usize, roster: u8, c: &Carried) -> Option<u16> {
    let bytes = state.get(at..at + 4)?;
    let amount = u16::from_le_bytes([bytes[0], bytes[1]]);
    let max = match c.kind {
        Kind::Weapon { .. } | Kind::Other => 1,
        Kind::Stack | Kind::Drug => c.count,
    };
    (bytes[2] == roster && bytes[3] == c.slot && amount <= max).then_some(amount)
}

/// The first offset from `from` where the rest of the items chain.
fn search(
    state: &[u8],
    roster: u8,
    carried: &[Carried],
    from: usize,
    items: &mut Vec<Item>,
) -> bool {
    if items.len() == carried.len() {
        return true;
    }
    (from..state.len()).any(|at| chain(state, roster, carried, at, items))
}

/// Whether the next item is at `at` and the rest chain after it.
fn chain(state: &[u8], roster: u8, carried: &[Carried], at: usize, items: &mut Vec<Item>) -> bool {
    let index = items.len();
    if index == carried.len() {
        return true;
    }
    let c = &carried[index];
    let Some(amount) = header(state, at, roster, c) else {
        return false;
    };
    let (next, fixed, loaded) = match c.kind {
        Kind::Stack | Kind::Other => (at + 4, true, None),
        // `drug::serialize` [V] 0x20AE24: an active bool, then more if set.
        Kind::Drug => match state.get(at + 4) {
            Some(0) => (at + 5, true, None),
            Some(_) => (at + 5, false, None),
            None => return false,
        },
        // `weapon_core::serialize` [V] 0x1E42A8: the magazine at +13, a
        // chambered bool at +19, then variable data.
        Kind::Weapon { chambered } => {
            let Some(magazine) = state.get(at + 13..at + 15) else {
                return false;
            };
            let magazine = u16::from_le_bytes([magazine[0], magazine[1]]);
            if magazine > MAGAZINE_MAX {
                return false;
            }
            let round = if chambered {
                match state.get(at + 19) {
                    Some(&byte) => u32::from(byte != 0),
                    None => return false,
                }
            } else {
                0
            };
            let next = at + if chambered { 20 } else { 19 };
            if next > state.len() {
                return false;
            }
            (next, false, Some(u32::from(magazine) + round))
        }
    };
    items.push(Item { at, amount, loaded });
    let rest = if fixed {
        chain(state, roster, carried, next, items)
    } else {
        search(state, roster, carried, next, items)
    };
    if !rest {
        items.truncate(index);
    }
    rest
}

#[cfg(test)]
mod tests {
    use super::*;
    use survarium::v021d::supplies::CHAMBERED;

    const START: &[u8] = include_bytes!("../tests/data/level_05_mode3_solo_4550.bin");
    const LATE: &[u8] = include_bytes!("../tests/data/level_05_mode3_solo_611600.bin");

    fn solo() -> Loadout {
        let carried = vec![
            Carried {
                slot: 7,
                dict_id: 18,
                count: 100,
                pool: 0,
                kind: Kind::Weapon { chambered: false },
            },
            Carried {
                slot: 8,
                dict_id: 50,
                count: 50,
                pool: 50,
                kind: Kind::Stack,
            },
            Carried {
                slot: 9,
                dict_id: 104,
                count: 4,
                pool: 4,
                kind: Kind::Stack,
            },
            Carried {
                slot: 10,
                dict_id: 107,
                count: 100,
                pool: 0,
                kind: Kind::Weapon { chambered: true },
            },
        ];
        Loadout {
            carried,
            owned: BTreeMap::from([(50, 50), (104, 4)]),
        }
    }

    #[test]
    fn compatibility() {
        let dictionary = Dictionary::shared();
        assert!(dictionary.compatibility.contains(&(50, 18)));
        assert!(dictionary.compatibility.contains(&(104, 107)));
        assert!(CHAMBERED.contains(&107));
        assert!(!CHAMBERED.contains(&18));
    }

    #[test]
    fn solo_match() {
        let dictionary = Dictionary::shared();
        let loadout = solo();
        assert_eq!(
            left(START, 0, &loadout, dictionary),
            Some(BTreeMap::from([(50, 50), (104, 4)]))
        );
        assert_eq!(
            left(LATE, 0, &loadout, dictionary),
            Some(BTreeMap::from([(50, 46), (104, 4)]))
        );
    }

    /// Test's loadout in the two-player match: the TT-33 and the PPSh-41
    /// both on the 7.62x25 in slot 8, 250 of it, 200 a life.
    fn two_weapons() -> Loadout {
        let weapon = |slot, dict_id| Carried {
            slot,
            dict_id,
            count: 100,
            pool: 0,
            kind: Kind::Weapon { chambered: false },
        };
        Loadout {
            carried: vec![
                weapon(7, 18),
                Carried {
                    slot: 8,
                    dict_id: 50,
                    count: 200,
                    pool: 250,
                    kind: Kind::Stack,
                },
                weapon(10, 106),
            ],
            owned: BTreeMap::from([(50, 250)]),
        }
    }

    #[test]
    fn an_enclosing_block_does_not_hide_the_state() {
        // At 611550 ms two offsets parse: the player's own block (pool 50,
        // items 10 bytes on) and an enclosing one (a "pool" of 1, items 670
        // bytes on). The first is the state: 50 in the pool, 144 in hand, 8
        // and 35 in the two magazines.
        const TWO: &[u8] = include_bytes!("../tests/data/level_03_mode2_two_611550.bin");
        let dictionary = Dictionary::shared();
        assert_eq!(
            left(TWO, 1, &two_weapons(), dictionary),
            Some(BTreeMap::from([(50, 50 + 144 + 8 + 35)]))
        );
    }

    #[test]
    fn unreadable() {
        let dictionary = Dictionary::shared();
        let loadout = solo();
        assert_eq!(left(LATE, 1, &loadout, dictionary), None);
        let garbage: Vec<u8> = (0..4096u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        assert_eq!(left(&garbage, 0, &loadout, dictionary), None);
        assert_eq!(left(&[], 0, &loadout, dictionary), None);

        // The state twice: two offsets parse.
        let length = u16::from_le_bytes([LATE[4], LATE[5]]) as usize;
        let mut twice = LATE.to_vec();
        twice.extend_from_slice(&LATE[4..6 + length]);
        assert_eq!(left(&twice, 0, &loadout, dictionary), None);
    }
}
