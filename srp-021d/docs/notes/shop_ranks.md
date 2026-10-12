# 0.21d faction shop: items by reputation rank

Tags: **[V]** checked in the exe or `lobby_menu.swf` (disassembled ABC);
**[W]** from the web; **[C]** srp's choice.

## What the client does [V]

- `36 04` price entries carry `u8 level`: the rank with the seller needed
  (`lobby_spec.md` §3.4). `GangShop.setupShopItems` puts every item on its
  shelf as buyable (`canBuy = true`).
- `36 09` → `on_player_reputations_arrived` 0x17673A computes each faction's
  rank from the points and `factions_dict`, and calls
  `setup_player_progress(faction, rank, points, lock text)` →
  `GangShop.setPlayerProgress`: an item above the rank gets `canBuy = false`
  and the lock text `item_sell_precondition_faction_<n>` + the rank's name; an
  item that was locked and now is not gets `isNew`, and the shop fires
  `NEW_ITEMS_COUNT_CHANGED` (the lobby's new items count, `updateNewItems`).
- So the prices must come before the reputations, and every rank-up must be
  followed by `36 09`: srp sends it with the results, and the client asks for
  it after each `#ru` message.
- The client data (`factions_dict`) names the ranks and thresholds only; no
  item lists.

## The ranks [W]

- 0.19a (Feb 2014) added "items in the store require a certain reputation
  with the faction"; 0.21a (Apr 2014) put the 12x70 slug at Scavenger
  Veteran. No per-item list from 0.20-0.22 survives.
- The Survarium wiki's faction pages of 25 Jan 2015 (0.25-0.26) list each
  rank's items. srp uses them for the items 0.21d has [C, the user's choice],
  per seller (`prices::RANKS`):
  - Scavengers: 1 TOZ-34, set 1, backpack, bandages; 2 Kiparis, PM, Amber
    respirator, chemical protection set; 3 TOZ-122, oxygen tank, medkit,
    antitoxin, RGD-5; 4 MP-153, light armoured vest, field pants, military
    boots; 5 Remington 700.
  - Black Market: 1 TOZ-66, set 1, trap; 2 Uzi; 3 Winchester, Colt Python,
    biker's helmet, thug's jacket, reinforced jeans, anesthetic, RGD-5;
    4 Remington 870, ropers; 5 AKS-74U, Corsair jacket, spiked gloves,
    choppers.
  - Renaissance: 1 AKMN (the list's AKM), army set and helmet; 2 SPAS-12,
    Fort-17.
  - Edge: 1 SKS, hunter set, Dar 2M container, Malachite respirator;
    2 Benelli M4, APS.
- Not on the list [C]: the IZh-27 (2) and the Vityaz (5) at their item
  level, the list's own rule for Scavengers and Black Market; the slug at
  Scavenger 5 (0.21a). Everything else needs no rank: the TT-33, Mosin and
  PPSh (starting weapons in 0.20a), ammo but the slug, the scope, artefacts,
  and the same items from other sellers.

## srp

- `prices::rank(dict_id, seller)`; `36 04` sends it; `24 00` is refused
  (`st_cannot_buy_item`) unless a seller, or the one named, sells at the
  account's rank. Items already owned or dropped stay usable, as in the game.
- A new account (no faction gear) gets reputation from daily quests: level 0
  quests need no gear and give 100, which is Scavenger and Black Market rank 1.
  Renaissance and Edge rank 1 needs 1,500, so all their goods wait for about
  12-15 of their quests (or their gear from a drop).

## Prices: srp's rank ladder [S]

The 2013 wiki prices (TT-33 10 … AKMN 1050, armour 50 … 450) were far too
low against 0.21d's income (a daily quest pays 1000..2400). Weapons and
armour now cost a base price times the tier of the rank that unlocks them
(`prices.rs`):

- base: a rifle, shotgun or SMG 1000 (the PPSh-41, sold from rank 0); a
  pistol 400; an armour piece 300;
- tier: Scavenger and Black Market rank 0..5 → ×1, 1.5, 2, 3, 4, 6;
  Renaissance and Edge rank 1, 2 → ×6, 8 (their rank 1 takes 1500 points,
  as much as the others' rank 4..5).

So: PPSh-41, Mosin 1000; TOZ-34, TOZ-66 1500; Uzi, Kiparis, IZh-27 2000;
TOZ-122, Winchester 3000; Remington 870, MP-153 4000; AKS-74U, Remington
700, Vityaz, AKMN, SKS 6000; SPAS-12, Benelli M4 8000. Armour 450 (rank 1)
to 1800 (Black Market 5, Renaissance and Edge). Ammo and consumables keep
the wiki's prices; selling pays half.
