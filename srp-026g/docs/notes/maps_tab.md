# Maps tab (`/maps`)

The 0.26g site's tab "Maps" / "Мапи". Built by `srp-026g/scripts/maps-tab/` (its README),
served by `servers/web-server/src/maps.rs`.

## Owner's rules

- Every playable map, the ported ones too. The points shown:
  - player spawns (by team);
  - battery spawns;
  - chargers (they spawn batteries);
  - battery containers;
  - artefact spawns.
- Each kind is toggled on its own; one toggle per artefact.
- The map underneath is either the top-down render ("From above") or the game's minimap. Nothing else.
- Data-driven, per release, never stale:
  - everything is read from the released patches;
  - a live release refuses a Maps tab built from other patches than it offers.
- The calculator's look; English and Ukrainian; names from the game's localization. Little text.
- A mode shows only what its rules use. Chargers appear in the timed battery mode only, with "N active"
  from the mode's `spawn_points_count`.
- Zoom and drag (wheel, two fingers, buttons). The chosen map and mode are in the address (`#level_01/3`).
- The view and toggles are remembered in the browser.
