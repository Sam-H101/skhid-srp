# "Get Gold" opens the exchange

The rules the code and the tests follow. Code: `client-patches/buy_gold_026g.py`,
`buy_gold/PatchBuyGold.java`, `buy_gold/SrpBuyGold.as`; README "Get Gold opens the exchange";
the test: `test_buy_gold.py`. The owner's rule: 2026-10-20 ("it leads to a dead official
website, instead it should just open gold trading menu").

- **What:** every way the lobby had to the online store (`go_to_store`, a browser window) opens
  srp's exchange window instead, on the buy side: Pay silver, Receive gold.
- **Where:** the "Get Gold" button of the "Insufficient Gold Rubles to complete the purchase."
  dialog, whichever window showed it: buying (the buy dialog, an item's card), repairing, rerolling,
  crafting, buying or extending premium, the premium discount; and the premium window's gold
  button, which asked "Online store will open in a browser window. Continue?" and now opens the
  exchange at once. The top bar's gold and silver boxes already opened it (`exchange_026g.py`).
- **Over the dialog it came from:** the exchange shows at once, over any window still open (the
  premium window, for one); that window stays visible under it, as under its own dialogs, and is
  active again when the exchange closes.
  The player presses Buy (or Repair, …) again: the dialog does not retry by itself.
- **The amount:** the exchange opens as from the top bar; it is not filled with the gold missing.
- **Texts:** unchanged. "Get Gold" fits buying gold with silver.
- **Not enough silver for the gold either:** typing more than the silver buys gives the most it
  does (`exchange.md`).
- **Server:** nothing changes; the exchange's `47 04` is the one already live.
- **Release:** a new `lobby_menu.swf` in a new `resources.db.patch_<n>`; built on revision 24's
  movie. A later movie patch must be built on this one (or rebuild both).
