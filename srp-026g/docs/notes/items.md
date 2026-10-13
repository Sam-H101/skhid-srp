# 0.26g items: modifications, defects, cosmetics, reroll, craft, repair

**Version scope:** client 0.26g (build 2777). Read from its exe, its
`lobby_menu.swf` (decompiled ActionScript) and its `resources.db`
(2026-10-12). Nothing here was tested against a running client. Tags:
PROVEN (read at the address or in the file named), LIKELY, GUESS, and
[srp] for srp's own choice where neither the client nor the data says.

The code: `crates/survarium/src/v026g/modifications.rs` (data),
`crates/session/src/v026g/items_rules.rs` (rules),
`servers/lobby-server/src/v026g/items.rs` (requests and answers).

## 1. The data

| file in `resources.db` | what | who reads it |
|---|---|---|
| `gameplay/items/modifications/modifications.options` | 158 `modifications`, 20 `defects`, 3 `defect_slots` | client: `id`, `modifiers`, `ui_desc`, `lobby_info`, a defect's `fix_cost_general` / `fix_cost_gold` / `fix_time_general` (loader 0x2C9E31.., strings at 0xA02264..). **Server only:** `drop_weight`, `type_mask`, `type_requirements_mask`, `required_modifications`, `defect_slots` (none of these strings is in the exe) |
| `gameplay/items/craft/craft.options` | `craft_price_general[16]`, `craft_price_premium[16]`, `craft_probabilities_general[3][2]`, `craft_probabilities_premium[3][2]` | server only; the lobby gets them in 0x9B |
| `gameplay/items/reroll/reroll.options` | `reroll_price_general[16]`, `reroll_price_premium[16]`, `reroll_modificator_multiplicator` 1.1 | server only; the lobby gets an item's two prices in `83 46 06` / `08` |
| every item config | `available_modifications_mask` (40 characters), `available_defects_mask` (20), `default_modifications[]`, `camo_available`, `decal_available` | client: the last three (0x2C97B7, 0x287703, 0x28777D). The two masks are server only |

PROVEN by the data:

- A modification's `type_mask` (41 characters) has one character set: its
  **type**, 1..=40. Five strengths share a type (ids `xxx0`..`xxx4`) with
  `drop_weight` 10, 8, 6.4, 5.12, 4.096. `type_requirements_mask` equals
  `type_mask` in every row.
- **An item's mask is one character shorter: its character `j` is type
  `j + 1`.** Only so do bolt-action rifles (`1110111…`) get "chamber a
  round" (type 6) and no rate of fire (type 4), automatic ones the other
  way round, and the defects' masks (21 characters against the items' 20)
  agree in the same way.
- Types with `drop_weight` 0 never roll: 11 (weight, unused), 15 pockets,
  16 artefact slots, 17 ammo bags, 29 pocket size, 33..40 (the boosters'
  effects and the premium items' silver, experience and reputation
  bonuses). They are only ever `default_modifications`.
- Three defects have `required_modifications: [low, high]` (4030 pockets
  `[1030, 1039]`, 4040 artefact slots, 4050 ammo bags): the id range of the
  modifications that give what the defect takes.
- Every defect: `fix_cost_general` 100, `fix_cost_gold` 50,
  `fix_time_general` 360, `drop_weight` 10.
- `defect_slots`: `{condition [0.9, 0], probability [0.1, 0.5]}`,
  `{[0.5, 0], [0.3, 0.7]}`, `{[0.1, 0], [0.5, 1]}`.
- A modifier is `{path[], value, type}`: type 1 with absolute steps
  (armour +0.02, a slot +1), type 2 with small fractions (damage +0.01,
  weight −0.05, wear −0.05). LIKELY 1 adds and 2 multiplies by `1 + value`.
- 47 items (every vest and every pair of trousers) have `camo_available`
  and `decal_available`; `items_compatibility` pairs no cosmetic with
  anything.

## 2. An item on the wire (corrects lobby.md §2.6, static_info.md §3.2)

`n1 × {u32 id, bool state}` — **state 0 is a modification, anything else a
defect.** PROVEN: the UI fill 0x229A40 looks the id up in the dictionary's
modifications table for state 0 (`+0x140`, count `+0x144`, 0x229F0D) and in
its defects table otherwise (`+0x148`, count `+0x14C`, 0x229AA0), 0x2C
bytes a row, and **reads the row it did not find in both cases** (`xor
esi,esi; mov eax,[esi+4]` at 0x229AC2). So every id must exist in the
table of its kind. (The earlier note had "state 0: no lookup"; wrong.)

- The lobby shows a state-1 entry as damage (`MessageRepair`, "critical
  damages", with its `fix_*` values), counts the state-0 entries as the
  item's **rank** (`InventoryList.getItemRank`; the exe's `rarity_level =
  min(3, base_rarity_level + count)`), and lets only those be changed.
- `default_modifications` are **not** sent: the client adds them from the
  item's config (`modifiersDefault`, `isDefault`).
- The match client applies an item's modifications to the item's config by
  their `path` (strings `modifiers`, `path`, `modification` used at
  0x2F1978..0x2F1A9A). LIKELY: nothing but the ids is needed from the
  server.

`n2 × {u32 upgrade_id, u32, u64, u8 param}` — PROVEN from the attach
handler (0x275671..0x2756A1), which builds the record: **`upgrade_id` is
the item id of the cosmetic** (it stays an item of the inventory: the
upgrade window finds it there, `UpgradeWindow.create_item`), **the second
u32 its dict id** (`movzx eax, word [item+4]`), **the u64 the value the
decal editor sets** (`46 05`), **`param` the slot**: 0 and 1 for decals
(the lobby shows two for `decal_available`, `and esi, 2` at 0x2877A6), 3
for the camouflage (`DragRegion.as`: `relocate_upgrade_item(item, camo,
3)`). Three at most (the inline capacity): one camouflage and two decals.

`repair_time` and `reroll_time` are **seconds left** (PROVEN in the UI:
`InventoryItem.updateRepairTime` counts `repairTime - delta` down with its
own timer; `ItemControlPanel` shows `rerollTime / 60` minutes).

## 3. Requests and answers (corrects lobby_requests.md)

| request | body (PROVEN) | Flash call |
|---|---|---|
| `46 01` | u32 active kit, **u32 host item, u32 cosmetic item, u8 slot**, u64 0 | `relocate_upgrade_item(item, upgrade, slot)` (0x29A984: args 0, 1, 2 in order) |
| `46 02` | u32 active kit, u32 cosmetic item | `drop_upgrade_item(item, upgrade)`: only the second is sent |
| `46 05` | u32 active kit, **u32 host item, u32 cosmetic item**, u64 value | the decal editor 0x263950; the client then finds the host and the cosmetic's record and writes the value itself (0x270A1A..0x270AC6) |
| `46 06` | u32 item | `item_reroll_opened` |
| `46 07` | u32 item, u32 modification id (`item.mods[index]`), **bool gold** | `reroll_item_modifier(item, index, money type)` |
| `46 08` | u32 item, u32 old modification id, **u32 the place of the choice (0..2)** | `modifier_choosed(item, index, modifiersList2.selectedIndex)`: the third is an index into the three offered, not an id |
| `46 09` | **u32 item, u32 donor item, bool gold** | `craft_item(slot1.itemId, slot2.itemId, premium)` |
| `47 03` | u32 item, u8 kind, u32 kit item if kind 2 | as before |

`83 46 <ev>` (0x275520):

| ev | body | notes |
|---|---|---|
| 01 | u32 host, u32 cosmetic, u8 slot, u64 value, bool detach_first | `detach_first` takes **the cosmetic** off the item it is on (0x275589 → 0x275220 with the second u32), not the host's upgrades. Both items must be in the inventory or nothing is done |
| 02 | u32 cosmetic | |
| 06 | u32 item, **u32 regular_cost, u32 premium_cost**, bool more; if more: **u32 the modification changed before, u32 reroll_time** | 0x2757CC → 0x29F430 → `root.fill_item_reroll_data {reroll_time, regular_cost, premium_cost, modifiers[]}`; each modifier gets `was_rerolled_before = (id == that u32)` and `reroll_available = state 0 and (none changed before or this one)` (0x29FC8D..0x29FD1A). The item must be in the inventory (no check, 0x29F58B) |
| 07 | u32 item, 3 × u32 modification, u32 reroll_time | each is looked up as a modification (0x29F680 → 0x229FD0): an unknown id crashes. `item.reroll_time` is set |
| 08 | u32 item, u32 old, u32 new, **u32 regular_cost, u32 premium_cost, u32 reroll_time** | the client replaces old by new in the inventory and the kit copies, then fills the window again with `new` as the one changed before (0x2759F0..0x275A16) |
| 09 | u8 mode, u32 item; **mode 0xFE ends here**; else u32 consumed item; if mode ≠ 0: u32 modification | 0x275A31: for 0xFE nothing more is read (the earlier note had a modification id; wrong). Mode 1 adds, another non-zero mode removes. The window shows **the consumed item's dictionary entry** as the upgraded item (0x275B84 → 0x29F770 `dict_id`) and the modification from its table |

`83 47 03`: as lobby_requests.md §4. The client sets the condition to
full and `repair_time`; it does **not** take the defects off its copy:
send the inventory after it.

Important message type 6 (`social.md` §1.4): the text is
**`#ir[<item id>]`** (PROVEN: prefix check 0x226851, then the bracket
parser 0x227120 that the quest texts use, `[` 0x9DB57C and `]` 0x9ECA34;
the client finds the item and sets its `repair_time` to 0, 0x22693A).

0x9B (PROVEN in the UI, `MessageCraftItem.updateCraftChance`): the twelve
floats are four for each difference in rank between the item and its donor
(0, 1, 2): **success in silver, failure in silver, success in gold, failure
in gold**, in percent; the two u16 tables are the prices by **item level**.
The window has no values of its own: without 0x9B it shows nothing.

## 4. The rules

What the lobby checks or says itself (PROVEN, `lobby_menu.swf` and the
0.26g localization):

- Reroll: "You can change only one modifier on an item but an unlimited
  number of times. When you change the modifier you will be prompted to
  choose one of three random modifiers." "Only one attempt in 6 hours is
  available when paying with silver." Silver is disabled while the item's
  `reroll_time` runs; gold never. Not for a damaged item
  (`st_reroll_message_broken_item`). The window can not be closed between
  the offer and the choice.
- Craft: both items not damaged and not in repair; both with fewer than
  three modifications; the item with no fewer than the donor
  (`st_craft_message_modifiers_difference`); not an equipped item while in
  the queue. Price = table[item level of the item]. Outcomes: "+1 mod.",
  "-1 mod.", or "you failed to upgrade the item".
- Repair: silver = `(1 - condition) * repair_cost_ratio * cost` + each
  defect's `fix_cost_general`, and takes the defects' `fix_time_general`
  together; gold ("quick repair") = the defects' `fix_cost_gold`; kits = one
  `repair_voucher` a defect. Gold and kits are offered only with defects.

srp's reading of what only the server knew:

| rule | tag |
|---|---|
| an item gets modifications of the types its mask names, one of a type, by `drop_weight` | LIKELY |
| a dropped weapon or armour of rarity `n` gets `n` modifications | LIKELY (the rank shown is their count) |
| a reroll offers three of different types that fit in the replaced one's place (its own type too, another strength) | [srp] |
| the payment is taken when the three are offered; an offer not chosen is kept and shown again when the window opens | [srp] |
| each change of an item makes the next cost `reroll_modificator_multiplicator` (1.1) times more | GUESS: the number's use is not in the client; the per-item prices in `83 46 06`/`08` suggest the price was the item's own |
| the donor must be another of the same item | LIKELY (the result window shows the donor's dictionary entry as the upgraded item) |
| the craft's chance row is `rank(item) - rank(donor)`; success, then failure, then nothing | PROVEN for the row (the UI indexes it so); the roll is the server's |
| a gained modification is rolled as a drop's; a lost one is any of the item's own | [srp] |
| a repair takes the defects off at once; the wait only locks the item in the lobby | [srp] |
| a defect slot opens under its first `condition`; its chance goes from the first `probability` there to the second at condition 0 | LIKELY |
| one defect roll for each match that wore the item, for its first free slot | [srp] (`SRP_ITEM_DEFECTS=off` switches defects off). Since 2026-10-13 a match wears nothing unless `SRP_ITEM_WEAR=on` (the owner's choice, `results.md` §12), so only a found item has a defect |
| a defect needs its type in `available_defects_mask` and, with `required_modifications`, such a modification on the item (a default counts) | LIKELY |
| wear is multiplied by `1 + value` of the item's "reliability" modifications | LIKELY |
| the kit's items' `money_bonus` / `experience_bonus` / `reputation_bonus` (modifications 1230..1280, defaults of the premium items) add that share to a match's pay, shown in the screen's "premium items" column | LIKELY for the meaning; that the three bytes of the column are percents is GUESS |
| any camouflage goes on any vest or trousers | [srp]: the data pairs none |

## 5. Not done

- Whether an item in repair may be played with: srp lets it [srp].
- `fill_craft_data {craft_time, …}` (`MessageCraftItem.fillCraftData`): a
  craft timer the window has a clip for; no message that sets it was
  found, and srp crafts at once.
- The camouflage's u64 (the decal editor serves both): kept as sent.
