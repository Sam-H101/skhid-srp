# Maps tab

The website's `/maps` (`srp-026g/servers/web-server/src/maps.rs`, spec in
`docs/notes/maps_tab.md`). It shows every playable map, by mode, with these
points, each kind toggled on its own:

- respawn points, by team;
- battery spawns and chargers (charging stations), plus a line from each
  charger to the battery spawns it feeds;
- the teams' battery containers;
- artefact spawns, one toggle per artefact.

The map underneath is either the top-down render or the game's own minimap.

## Build: every release that changes content

`build.py` reads it all from the game as the players have it: the stock
`resources.db` with exactly the patches of `release/patches.tsv` over it,
each checked by its sha256. Nothing about a map is typed in by hand: a patch
that adds a map, moves a spawn or redraws a minimap shows up at the next build.

```
nix build --impure -o py --expr 'with import <nixpkgs>{}; python3.withPackages(p:[p.pillow])'
nix shell nixpkgs#jdk21 -c py/bin/python3 -I srp-026g/scripts/maps-tab/build.py \
  --stock "$SKHID_STOCK_DB" --patches <folder with the list's patches> \
  --dbgrab tools/re-kit/common/dbgrab/target/release/dbgrab --ffdec <ffdec-cli.jar> \
  --icons <the lobby icons, as the damage calculator's build exports them>
# a content release: add --with <its resources.db.patch_<n>>, before the patch is in the list
```

It writes `srp-026g/servers/web-server/maps/`:

- `maps.html`, `maps.js` (the data and `ui.js`);
- `minimap/` and `icons/`;
- `files.rs` (what the server includes);
- `built-from.tsv`.

Commit them. The patches in `built-from.tsv` must be exactly the offered
ones:

- `release/release.sh` refuses a live release otherwise (its preflight).
- The web-server test `the_maps_are_built_from_the_offered_patches` fails
  otherwise.

So a content release first merges a rebuild made with `--with <its patch>`.

What comes from where:

| what | where in the game data |
|---|---|
| maps, modes, names | `maps_dict` ids whose `gameplay/game_modes/` config is mode 2, 3 or 4 and whose project exists (the server's rule, `crates/survarium/src/v026g/maps.rs`); names from the localization |
| respawn points, battery spawns, chargers, containers | the project's `server_objects`, shown only where the mode's `rules` read them (88 gather, 89 timed with chargers) |
| artefact spawns | `client_objects`: each `artefact_container`'s collision zone, its artefacts from the `generic_anomaly_core` that lists it; names from `st_<artefact>_pickup_hint` |
| minimap and its frame | `flash_movies/hud.swf`, `survarium.controls.Minimap` (`_minimapProps`, `_levels`) by the project's `project_name`, as the HUD picks it (exported with FFDec) |
| top-down render | `srp-026g/scripts/damage-calc/assets/maps/` (its README) and the site copy in `servers/web-server/damage/maps/`: the one input not read from the game |

A map without a render stops the build. Render it first, add it to
`damage/maps/` and `damage.rs`, then build again.

The build also stops in these cases, rather than guess:

- the weathers of one map and mode hold different objects;
- the modes of one map hold different artefact spawns.

`--work <dir>` keeps the extracted files between runs while you work on the
page. `--allow-missing-renders` is for the same purpose: a map without a
render then shows its minimap only.
