# 0.21d premium account

What the client fixes, and what srp chooses. Tags: **[V]** exe or data,
**[I]** inferred, **[C]** srp's choice.

## The client [V]

- The crown button opens the store (`MessagePremium.as`): one row per offer
  of `36 11` (`read_shop_items` 0x19E40E, layout in `lobby_spec.md` §3.16),
  shelf 0 only. A row is enabled only if the gold (`premium_money`, `36 05`)
  covers its price. "Buy" calls `accept_premium(id)` → `24 02 <u32 id>`.
- `on_shop_items_arrived` 0x17861E passes `label` and `value2` through
  `text_translator::translate_text` 0x15DAC0: a whole localization key
  (`st_premium_week`), or the raw text when no key matches.
- The reply `34 24 02` makes the client ask `21 05` (money) and `21 0F`
  (services).
- `36 0F` type 0 entry {a = hours left} → `set_premium_state`: 1 when at
  least 24 h ("N д."), 2 under ("N год." or "less than an hour").
- The "+" (add gold) button is disabled: gold came from the web shop.
- The localization names six durations, `st_premium_day`, `_week`, `_month`,
  `_3_month`, `_6_month`, `_year`, each with a `_descr` line. 72 hours has
  only `st_premium_3_day_descr`, no name.
- The results screen shows non-premium players "(+X)", X = value × 0.5, for
  experience, reputation and money (`results_spec.md` §2.3): premium was
  +50 % on all three [I]. The drop bar's top is `drop_amount` × 1.5 [V]
  (`progression.md`), which fits premium also filling the bar half again [I].
- No currency is an item (`items_dict` has none), so gold can never be a
  "Found" drop; the results screen has a silver box only.

## srp [C]

- Offers (`crates/survarium/src/v021d/premium.rs`; the original prices are
  lost): day 250, week 1,200, month 3,000, 3 months 7,500, 6 months 13,500,
  year 24,000 gold. No 72-hour offer, for want of its name.
- A purchase adds its hours to what is left (or from now). The account keeps
  `premium_until` (Unix seconds); `36 0F` sends the whole hours left,
  rounded up.
- With premium, a match's experience, money and reputation are × 1.5 (after
  the experience rate), and the drop progress is too, up to 15,000.
- New accounts start with 1,000 gold (the user's choice). Gold still also
  comes as level rewards (`results.rs` `level_reward`).
