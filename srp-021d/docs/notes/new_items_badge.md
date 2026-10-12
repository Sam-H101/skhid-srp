# The new items badge (0.21d lobby)

The red number by the top menu counts shop items the last reputation
update unlocked (`GangShop.setPlayerProgress` → `NEW_ITEMS_COUNT_CHANGED` →
`StatusPanel.setNewItems`). srp only sends the ranks (`36 09`); the client
does the counting. Reaching rank 1 with the Scavengers and the Black
Market unlocks 13 items (`prices::RANKS`).

`StatusPanel.setNewItems` places it at
`topMenu.x + (tabPage.buttonWidth + tabPage.spacing) * 2 - 35`, the top
right of the second button (Items), with `buttonWidth` 100
(`LobbyMenu.init`). The buttons are drawn wider, so it showed on the third
button (Tasks). [V: seen in the client at 1.95× the 2000 px screenshot]

Fix (`client-patches/lobby_menu.py`, with the shop slider fix):
`pushbyte 2` → `pushbyte 1` in that sum (body offset 17989259), one button
less. Same-size repack and backup as `shop_slider.md`;
`--restore` puts the original back. Not yet seen in the client after the fix.
