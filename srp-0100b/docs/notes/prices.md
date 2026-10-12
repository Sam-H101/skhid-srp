# Shop prices (2013)

The lobby's price list (`PRICES` in `servers/lobby-server/src/lobby_server/mod.rs`)
uses what players recorded in 2013. Neither the client nor its data has
prices: the server sent them.

## Sources

The Survarium fandom wiki (`https://survarium.fandom.com`). Its current
pages show the 2015 economy, so the revisions below come from the page
histories. To fetch one, use the MediaWiki API:
`api.php?action=query&prop=revisions&titles=<Page>&rvlimit=50&rvprop=timestamp|content`.

| What | Page, revision | Notes |
|---|---|---|
| Weapons | each weapon's page, 2013-05-17 (`cost =` in its infobox); `Weapons`, 2013-12-30, for the sellers | 0.100b is the May 2013 build |
| Armour | `Equipment`, 2013-05-21 (prices); 2013-12-30 (later names, weights, sellers) | three tiers: 50 / 100 / 150 |
| Trap | `Trap`, 2013-05-18 | Black Market |
| Medkit, bandages, anesthetic | `Items`, 2013-12-23 | medkit "available from multiple factions" |
| Ammo | `Ammunition`, 2013-12-22 and 2013-12-30 | "per box basis"; each box cost as many as it held rounds |

The official "Alpha Test Diary #1" (forum.survarium.com, 13 May 2013; Wayback
capture 20130810183434) says that in this alpha, reputation and faction tasks
were off and every Scavengers and Black Market item could be bought from the
start. So every item has `reputation_level` 0.

## How the wiki's names were matched to items

- **Weapons:** by name; Vityaz is the PP-19-01 page.
- **Armour:** the wiki's tiers (50, 100, 150) match the config numbers `_1`,
  `_2` and `_3` of each faction and slot. The wiki gives 11 weights, and each
  equals its config's `weight` (`gameplay/items/armour/...`):
  - Bruiser 7, Corsair 8, leather sleeveless jacket 2;
  - choppers 1;
  - chemical protection pants 3, field pants 2.5, reinforced jeans 2.5;
  - chemical protection gloves 0.8, spiked gloves 0.3;
  - biker's helmet 2, "Amber" respirator 0.5.
- **Ammo:** the box sizes the wiki lists (.357 20, slug 20, 7.62x25 50,
  9x18 50) equal each config's `clip_size`.

## What the client does with a price

A price is for one item, and for ammo one round (`inventory.swf`):

- The ammo window sets `item_price` to the price times `clip_size`, and
  `get_required_price` multiplies that by the clips to buy. It sends the
  rounds (clips times `clip_size`).
- The buy dialog (`MessageAsk.update_slider`) steps a stack by `clip_size`
  and shows the count times the price.

So each 2013 box (its price equal to its rounds) is 1 a round.

A purchase (`lobby_client::buy_item`) sends the item, the count as a u32 and
the seller. It has no money type: the byte after the seller is always 0, so a
purchase takes generic money. After `process_shop_action` adds the item, the
client asks for the money again (`q_account_money`) and shows that.
`on_operation_denied_received` shows a refusal's text in a message box.

## Not from a source

- **Ammo sellers:** the wiki doesn't say who sold ammo. Each type goes where
  a weapon that fires it is sold.
- **Oxygen tank, 100:** a guess, priced as the backpack in the same slot.
- **12x70 3/0 buckshot, 20:** a guess. The wiki says 3/0 existed until 0.18a
  and assumes it worked like 2/0, but gives no price.

The scope (69) and the lifebone (54, 57) were never sold.

A price must not be 0: the client caps what it can buy at the money divided
by the price (`try_buy_item`).
