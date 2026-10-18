# Damage calculator (`/damage`)

The 0.26g site's tab "Damage calculator" / "Калькулятор шкоди": shooter against target, every
number from the game data, the rules as the client's hit code applies them. Built by
`srp-026g/scripts/damage-calc/` (its README), served by `servers/web-server/src/damage.rs`.

## Owner's rules

- The data is the game as players have it: the stock `resources.db` with every **released**
  patch over it (now revision 21). Rebuild with each content patch.
- English and Ukrainian, as the rest of the site (`?lang=`). Item, map and bonus names are the
  game's own localization.
- Less text: one line per rule, players know the game. No intro, no sources paragraph; the
  rules section is the formula only.
- Shooter: weapon, ammo, distance 0–150 m (all fights), weapon bonuses always shown, one
  button per grade (Damage +1% … +5%), no defects (negative bonuses).
- Target: the HUD doll and the equipment cards (both aim), armour bonuses, Onyx (four slot
  tiles, passive or active), the Anesthetic, the pain already taken.
- A closed helmet (or an exo suit) takes the slots its model covers: their cards are locked.
- Map: top-down renders, two clicks for the distance; the distance slider moves the target
  along the same line.
- Compare table: every weapon, sortable by any column, column separators.
- The calculator keeps its own look; only the site's header sits above it.
- Not modelled: cover a bullet passes through, ricochets, explosions, anomalies.

## Serving

- `GET /damage`: the page in the request's language, under the site's header (`no-store`, as
  every page). Its Content-Security-Policy adds `script-src 'self'`, `img-src 'self' data:` and
  `font-src 'self'` to the site's; nothing inline runs and nothing comes from elsewhere.
- `GET`/`HEAD /damage/calc.js`, `/damage/fonts/*.woff2`, `/damage/maps/*.webp`: the files the
  page names with `?v=<hash>`, kept a year (`immutable`). Part of the binary.
- The 0.21d site has no calculator.
