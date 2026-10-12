# 0.21d shop: buy slider cap for ammo (client patch)

Tags as in `shop_ranks.md`: **[V]** checked in `lobby_menu.swf` (disassembled
ABC); **[C]** srp's choice.

## The bug [V]

The buy dialog is `MessageAsk`. For a stack item `_step = clip_size` (rounds
per clip), the slider runs in rounds with `snapInterval = _step`, and
`_price` is the price of one step: `onSliderUpdate` and `update_slider` show
and charge `value / _step * _price`, `ohYes` sends `count = value / _step`
(clips). But `update_slider` caps the slider (paid branch, `_price >= 1`) at

    int(int(money / _price) / _step) * _step        // rounds

where `int(money / _price)` is already a number of clips. The cap is
`_step` times too low: with 1000 money, a 10-per-clip price and 30-round
clips it allows 90 rounds (3 clips) instead of 3000 (100 clips; the slider
max is also min'd with `100 * _step`, the pocket and the weight).

## The fix [C]

    int(int(money / _price) * _step)

At `MessageAsk.update_slider` +168..+184 the bytes
`d2 d0 66 _price a3 46 int 01 d0 66 _step a3 46 int 01 d0 66 _step a2 75 75`
(once in the SWF body) become `... a2 46 int 01 d0 66 _step 29 75 75`: the
second `divide` (+176) is a `multiply`, the trailing `multiply` (+184) a `pop`
(the stack `[x, _step]` leaves `x`). Same length, same stack depth.

## Repack, same size

`flash_movies/lobby_menu.swf` is stored uncompressed in `resources.db`
(pos_in_db 14268063, 16529440 bytes, FAT hash unused at load). The SWF is
CWS; the original body is zlib level 9 and recompresses byte for byte. The
patched body is written back to exactly the original size: deflate level 8
(memLevel 8) of all but the last 574 bytes, `Z_SYNC_FLUSH`, the 574 bytes as
one final stored block (`01 LEN NLEN data`), big-endian adler32. The header
(`CWS`, version, uncompressed length) is unchanged, and the result
decompresses to the original body with only those two bytes different. So the
entry is overwritten in place; no FAT change.

## Apply / restore

    python -I client-patches/lobby_menu.py <game>/resources.db
    python -I client-patches/lobby_menu.py <game>/resources.db --restore

It checks the entry's sha256 (original `1370380f…`, patched `2049e72f…`),
refuses anything else, saves the original as `lobby_menu_original.swf` next
to the db (once), writes, and re-reads to verify. Running it twice says
`already patched`.

## Other formulas [V]

- `MessageAskSell`, `AmmoShareElement` (`get_required_price`,
  `updateStorage`: `clip_size * int((money - others) / _item_price)`),
  `ItemCountControl.setData` (price only tested for 0): per-clip, fine.
- `ItemCard` (shop card): `option_stepper` (`stepSize = _step`, max
  `int(money / cost)`, button `value * _cost`, label `_cost * _step`) treats the
  price as per round; but the stepper is never made visible and `acceptAction`
  buys count 1, so it is dead.
- `LobbyMenu.try_buy_item` sets `count = int(money / price)` (clips); the buy
  dialog does not use it.
- Unknown: the pocket (`getPocketSize / quick_slot_item_size`) and weight
  (`free weight / weight`) caps in `update_slider` are compared with rounds;
  right only if those sizes are per round.

## Status

Applied to `downloads/v021d/extracted/game/resources.db` (Oct 2026), verified
by re-extracting and disassembling. Not yet tried in the game.
