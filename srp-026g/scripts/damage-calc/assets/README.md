# Survarium 0.26g UI art for the damage calculator

All art comes from the pristine `Survarium-0.26g-original/Survarium/game/resources.db`.
It was extracted with the repo's `dbgrab` and JPEXS FFDec 26.0.0 (CLI). The game was never run.

## doll/ — body-part doll ("kukla")

**Source:** `flash_movies/hud.swf`, sprite **119**, class `SrvCriticalIndicator`
(`survarium.controls.CriticalIndicator`). This is the in-match HUD figure that tints a body part by its HP.
The lobby (`lobby_menu.swf`) has no body silhouette: its paperdoll is equipment slots around a 3D character.

The game draws the doll from small **bitmaps**, not vectors:

| game instance | sprite | bitmap | size | part |
|---|---|---|---|---|
| `head`  | 112 | 110 | 12×16 | head |
| `body`  | 109 | 107 | 22×36 | torso |
| `lHand` | 115 | 113 | 18×35 | whole left arm (`rHand` is the same bitmap mirrored) |
| `lLeg`  | 106 | 104 | 16×49 | whole left leg (`rLeg` is the same bitmap mirrored) |
| outline | shape 118 | 117 | 63×107 | white line drawing plus a ground ring, drawn on top |

The game has **only 6 parts**: head, torso, 2 arms, 2 legs. The calculator's zones were made by splitting those
parts with horizontal cuts, in the 63×107 frame. The zones follow the hit_param ↔ bone mapping in
`models/character/human/base/base_character_full.skinned_model/hit_targets`:
- brain|face at y=6
- the whole arm|hand at the **wrist** (y=44)
- the whole leg|foot at the **ankle** (y=84)

The wrist and ankle cuts sit at the narrowest row of the arm/leg bitmaps. The ids keep the game's hit_param names,
so `arm_*` is the hand and `forearm_*` is the whole arm.

| SVG id | data-label | bones (hit_targets) | game part |
|---|---|---|---|
| `brain` | Head (brain) | Head (sphere) | head |
| `face` | Face | Head (front box) | head |
| `front` (`back` in doll_back.svg) | Torso (front/back) | Spine+Chest+Neck / Chest+Spine | body |
| `forearm_l` / `forearm_r` | Left/Right arm | Arm+ForeArm (shoulder→wrist) | lHand / rHand |
| `arm_l` / `arm_r` | Left/Right hand | Hand | lHand / rHand |
| `hip_l` / `hip_r` | Left/Right leg | UpLeg+Leg (hip→ankle) | lLeg / rLeg |
| `foot_l` / `foot_r` | Left/Right foot | Foot | lLeg / rLeg |

Files:
- `doll_front.svg`: the main asset. Each part bitmap was upscaled 12×, cut into zones and traced with potrace. Every zone is one `<path class="zone" id=… data-label=… data-game-part=…>`, with the traced outline on top (`pointer-events:none`). Built-in CSS gives hover and `.selected` styles. viewBox `-1 -1 65 109`. The figure faces the viewer, so the character's left (`_l`) is on the viewer's right, as in the game.
- `doll_back.svg`: the same figure mirrored. The torso is `back`, and `_l`/`_r` still mean the character's side.
- `doll_zonemap.svg` / `.png`: flat unique colours per zone, for colour-pick hit testing. The colours are in `doll.json`.
- `doll_front.png`, `doll_back.png`: 260 px wide PNG fallbacks of the SVGs.
- `doll_hud_original.svg` / `.png`: a faithful copy of the HUD, with the game's own bitmaps embedded and placed by the swf matrices (ids `head body arm_left arm_right leg_left leg_right outline`). It is low-res (63×107 source).
- `doll.json`: zone ids, labels, game-part and bone mapping, cut lines and zonemap colours.

The ground ellipse in the traced SVGs is redrawn by hand (a fading ellipse); the game's ring is a soft bitmap.

## icons/ + icons.json — item icons

**Source:** the atlases embedded in `flash_movies/lobby_menu.swf`, cut exactly as the game's ActionScript does:

- `LobbyMenu.createIcons` → `new IconContainer(name, 171, 83, atlas, vertical=true, 0, stepH=0, stepV=8, atStart=false)`
- `IconContainer`: `columns = W/171`, `rows = H/91`. Cell (c, r) is the rect `(171·c, 91·r, 171, 83)`, with an 8 px gap below each cell.
- `ItemFactory.createBaseItem`: `iconId = (icon_column−1)·rows + (icon_row−1)`.
- **`ui_desc.icon` in the item config is `[row, column]`, 1-based.** This was checked visually: `[7,2]` in equipment is `edge_boots_1`, and `wpn_ak_74u [7,2]` is the AKS-74U.

| atlas | swf character | size | grid | used for |
|---|---|---|---|---|
| equipment   | 3273 `LobbyMenu_EquipmentIcons` | 855×2048 | 5 col × 22 row | armour (all slots) |
| weapons     | 3277 `LobbyMenu_WeaponIcons`    | 513×2048 | 3 × 22 | `wpn_*` |
| consumables | 3276 `LobbyMenu_AmmoIcons`      | 513×2048 | 3 × 22 | ammo, grenades, drugs, artefacts, boosters, devices |
| vouchers    | 3282 `LobbyMenu_VoucherIcons`   | 171×546  | 1 × 6  | vouchers, money |

The set has 211 icons: 110 armour, 50 weapons, 29 ammo, 4 drugs, 3 artefacts, 6 boosters, 2 grenades,
1 device, 4 vouchers and 2 money. They are native 171×83 RGBA PNGs, palette-quantised to 255 colours (about 1 MB total),
named `<config name>.png`, e.g. `wpn_ak_74u.png`, `ammo_5.45x39_7n10.png`, `edge_torso_3.png`,
`artefact_onyx.png`, `painkiller.png`.

`icons.json` → `items[<config name>] = {file, group, subfolder, atlas, icon:[row,col], rect:[x,y,w,h], name_key, name_en}`,
plus `atlases` (the geometry above) and `icon_rule`. `name_en` comes from `localization/english`. It is null for the
three artefacts, because their `st_artefact_*_name` keys are missing from the 0.26g English table.

Not included:
- Camo/upgrades (`camo` atlas 3270, 72×72) and weapon modifications/defects (`modifiers` atlas 3279, 38×38, by plain index).
- Scopes, which have `icon:[0,0]` (no icon).

## Regenerate

```sh
nix shell --impure --expr 'with import <nixpkgs>{}; [ jdk potrace librsvg (python3.withPackages(p:[p.pillow])) ]' \
  -c bash tools/regenerate.sh \
     /home/sheep/games/Survarium-0.26g-original/Survarium/game/resources.db \
     <config_json_dir> <ffdec-cli.jar> <work_dir> <assets_dir>
```

- `<config_json_dir>` holds `gameplay/items/**` and `localization/english` decoded to JSON, named with `/`→`__` (e.g. `gameplay__items__weapons__wpn_ak_74u.options.json`).
- The script runs `dbgrab get` on `lobby_menu.swf` and `hud.swf`, then `ffdec -export image` on both, then `tools/build_icons.py` and `tools/build_doll.py`, then renders the PNGs with `rsvg-convert`. A re-run produced byte-identical output.
- `tools/symbols.py <swf2xml output>` lists `characterId → class` from the SymbolClass tag. It was used to find sprite 119.

Other useful one-offs:
- `ffdec -format sprite:svg -selectid 119 -export sprite out hud.swf`: the raw HUD sprite as SVG.
- `ffdec -export script out lobby_menu.swf`: the ActionScript (`survarium/data/IconContainer.as`, `survarium/item/ItemFactory.as`, `LobbyMenu.as` ~l.5190).
