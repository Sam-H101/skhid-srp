# A player's equipment from the Rankings (and the rooms' lists)

The rules the code follows. Code: `client-patches/player_gear_026g.py`,
`player_gear/SrpPlayerGear.as`, `player_gear/PatchPlayerGear.java`; the lobby's
`servers/lobby-server/src/v026g/gear.rs`; the wire `lobby::GEAR_BY_PLACE`. The owner's rules,
2026-10-20: "after match in match overview you can rmb to see the players equipments", "you
should be able to go to rankings and rmb to do the same", "for every single player".

- **What the results screen does (the game's own, unchanged):** a **double-click** on a row
  ("Double click to show information", `show_player_info_hint`) asks for that player's kits
  (`victory_screen_request_player_data` → `3D 00 <row> <match>`); the answer (`93`) opens the
  game's "Information about the player:" window (`player_info`, its kits and items, a tab a
  kit). Its right-click menu has Add to friends, Add to ignore, Report and Compliment, no
  equipment item (`VictoryScreen.showContextMenu`). srp answers the double-click for players of
  a match it keeps (`rewards.rs` `player_details`, the kits as the match left them).
- **Rankings:** a right-click on any row (Top 100 and My place, any page, the player's own row,
  the own place under the list) opens the game's right-click menu (as the results screen's) with
  one item, the game's own "Show player's information" (`show_player_info_button_hint`, every
  language). It opens the same window as the results screen, with that player's name.
- **Every player:** online or not, never signed in since a restart, with no match ever, the
  asker itself: the server answers from the account as it is now (srp.db, kept in memory).
  Kits as they are now, not as some match left them.
- **Which player:** the request names the row's place (`3D 00 <GEAR_BY_PLACE> <place>`, as the
  History's "Watch replay" names its match). The server keeps, for each player's session, which
  account each place it was sent was (since its page last opened): a row is the player the page
  showed there, however the ranking moved since. A place it was not sent (rows of the
  sign-in's pushes): the ranking's now.
- **Nothing to show** (no such place or account): no answer, no window (the game has no "nothing
  to show" message for `93`).
- **The rooms' lists** use the same menu and window (`SrpPlayerGear.menu`; `rooms.md`).
- **Older movies:** never ask. An older server does not answer (no window opens).
- **Release:** a lobby movie (resources.db patch) and the server; the server first.
