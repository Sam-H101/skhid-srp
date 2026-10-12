# 0.21d faction loyalty

Tags: **[V]** checked in the exe (VAs, image base 0x10000) or `lobby_menu.swf`
(disassembled ABC); **[W]** from the web; **[C]** srp's choice.

## What the client does [V]

- `36 0F` (`read_account_services` 0x19E358): types 1..4 are the four
  factions' loyalty; the low byte of `b` (unsigned, `movzx`) is the bonus. At
  most 8 entries in all.
- `set_factions_loyalties` 0x17879A hands `[{faction_id, bonus}]` to Flash
  `set_faction_bonus` → `GangShop.setFactionBonus` → `GangShop.draw`, which
  puts `faction_bonus` = "+N" (empty for 0) on each faction's reputation row;
  `ReputationRenderer.updateText` shows it with a "%" field, hidden when empty.
- Nothing else reads it: the bonus is the server's to apply.
- Changing a quest always warns `st_quest_change_warning` ("Changing tasks
  will lower your faction loyalty!", `MessageChangeQuest.configUI`).

## The rules [W]

From the 0.20a announcement (survarium.com news, spring 2014; 0.21d is April
2014):
- loyalty raises all the reputation earned with that faction, by 5 % for each
  of its daily quests done, up to 25 %;
- changing a quest lowers it by 5 %; failing one resets it.
- 0.22a (May 2014) and 0.23g later softened failing (−10 %) and tied quests to
  one faction; 0.27a replaced the quests. No source ties loyalty to the
  reputation level.

## srp [C]

`crates/survarium/src/v021d/loyalty.rs`, `account.loyalty` per faction:
- a done quest +5 (cap 25), a change −5, a quest that runs out → 0;
- the bonus multiplies the match's reputation (after premium) and each quest's
  reputation award, with the loyalty before that quest raises it;
- `36 0F` lists each faction above 0, after premium; sent again with the
  results, after a change (`34 2B`) and with the quest list (`36 0D`), since
  the client asks for `0F` only at sign-in and after a premium purchase.
