# Survarium 0.26g: HUD doll tint, doll art, modification icons and lobby style

Sources: the pristine `survarium.exe` (image base 0x10000; VAs below), the decoded `swf2/flash_movies__hud.swf`
and `swf2/flash_movies__lobby_menu.swf`, the FFDec-exported ActionScript and the decoded configs (`json/`).
The game and Wine were never started. Labels: **PROVEN** = read from the instructions, swf tags or config.
**LIKELY** = strong static evidence, but part of it is inferred.

Scripts are in `tools/`: `swftree.py` dumps a sprite tree from raw SWF tags, `build_doll2.py`, `cut_mods.py`, `mock_mods.py`.
`tools/py.sh` is a nix python with Pillow and fontTools. FFDec intermediates are in `work/`.

---

## 1. HUD doll tint inputs (exe → `set_body_part_hp`)

### The only caller
- `root.set_body_part_hp` (string 0xa035e4) has exactly one code reference, at **0x237286**, inside function **0x2370e0**. **PROVEN**
- 0x2370e0 has one caller, **0x23b78d**, in the HUD update function 0x23b740. That function also refreshes the other local-player HUD state. The per-frame call is **LIKELY**.
- The function loops over a static table at **0xacfc38**: 6 entries × 12 bytes, `{const char* pool_name; float last_value; float last_break_point}`. The table is read at 0x2370fd and the cache is updated at 0x237191/0x237199. **PROVEN**

| part id (AS `_bodyParts`) | table entry | damage pool | pool Hmax | **break_point** |
|---|---|---|---|---|
| 1 `lHand` | 0xacfc38 → "left_hand" | left_hand | 0.5 | **0.2** |
| 2 `rHand` | 0xacfc44 → "right_hand" | right_hand | 0.5 | **0.2** |
| 3 `lLeg` | 0xacfc50 → "left_leg" | left_leg | 0.5 | **0.2** |
| 4 `rLeg` | 0xacfc5c → "right_leg" | right_leg | 0.5 | **0.2** |
| 5 `head` | 0xacfc68 → "head" | head | 0.4 | **0.5** |
| 6 `body` | 0xacfc74 → "body" | body | 1.0 | **0.5** |

- **part id** = table index + 1 (`inc eax` at 0x2371f4). It is passed as GFx value type 4 (uint). **PROVEN**
- **pool lookup:** 0x2e58b0 walks the damage model's body-part list (`model+0x108`) and compares the name string at `node+0x80` (the body-part ctor 0x329e42 builds that string). **PROVEN**
- **value = H / Hmax** = `pool+0xa0 / pool+0x9c` (0x237114 `movss [eax+0xa0]` / 0x23711c `divss [eax+0x9c]`). It is passed as a double. It is the fraction of that pool, 0..1. **PROVEN**
- **break_point = `pool+0xa8` = `regeneration_threshold`**. It is read at 0x23712c. The body-part ctor stores this field at 0x329e97. The values come from `human_hit_params`: body 0.5, head 0.5, legs and hands 0.2. **PROVEN**
- **Send condition:** the exe calls the movie only when `|value − last| ≥ 0.01` or `|bp − last_bp| ≥ 0.01` (constant 0.01 at 0xa2d774; compares at 0x237154/0x237186). The cache starts at 0/0, so the first update sends all six parts. **PROVEN**
- **Pain is not on the doll.** It is not in the table (PROVEN). Infection is not on the doll either. The pain stages 75/50/25 % are full-screen HUD effects (`effect_threshold_pain`; LIKELY, see `findings/pain-limbs.md`).
- **Movie functions the exe never calls (PROVEN: their names do not exist anywhere in the exe):**
  `set_body_part_state`, `crit_player_body_part`, `heal_player_body_part`, `set_radiation`, `set_toxic`, `set_bleeding`, `set_player_hp`, `crit_part` and `part_hp`.
  So in 0.26g the **rad/tox/bleed icons never appear**, `crit()` (0 → 1, 1 → 0.4, 2 → 0, with bp 0.5) is dead code, and so is `heal()`.
  The only HUD functions on this path are `root.set_body_part_hp`, `root.hit_player` (0x237574), `root.set_stamina` and `root.reset_damage_indicator`.

### What the movie does with it (`CriticalIndicator.partHP`)
```
g     = uint(min(value/bp*180, 180))      // getColor(255, g, 0): the uint argument truncates
alpha = value < bp ? 0.5 : 0.5*(1-value)/(1-bp)
colorTransform.color = 0xFF<g>00          // multipliers 0, offsets = colour, so the red bitmap becomes flat (255,g,0)
colorTransform.alphaMultiplier = alpha    // result alpha = bitmap alpha × alpha
```
- When value ≥ bp the colour is a constant **#FFB400** and fades out as the value rises: alpha is 0.5 at bp and 0 at 100 %.
- When value < bp the alpha stays at 0.5 and the colour goes from orange to **#FF0000** at 0.
- A healthy part (value 1) is **invisible**.
- The whole indicator, outline included, is `visible=false` while all six values are exactly 1. It becomes visible as soon as any value ≠ 1.
- Per `pain-limbs.md`, regeneration stops at threshold × Hmax: body and head at 50 %, limbs at 20 %. So after a hit the doll stays shown until the part is healed.

Examples (body/head bp 0.5; limbs bp 0.2):

| value | body/head | limbs |
|---|---|---|
| 1.0 | invisible | invisible |
| 0.8 | #FFB400 α0.20 | #FFB400 α0.125 |
| 0.5 | #FFB400 α0.50 | #FFB400 α0.3125 |
| 0.4 | #FF9000 α0.50 | #FFB400 α0.375 |
| 0.1 | #FF2400 α0.50 | #FF5A00 α0.50 |
| 0 | #FF0000 α0.50 | #FF0000 α0.50 |

## 2. Doll art (`doll/`)

**There are no `mc_red` / `mc_yellow` children in 0.26g.** This is PROVEN from the raw tags of hud.swf.
- Each part sprite (106 leg, 109 body, 112 head, 115 arm) has 1 frame and holds one shape with a bitmap fill of a **pure red (#FF0000) bitmap**.
- The constructor's `if(redMC && yellowMC)` guard therefore skips them, and `heal()`'s "off" label does not exist.
- The tint is applied to the whole part sprite by the colour transform above.
- `rHand` and `rLeg` reuse the left sprites with a −1 x scale.
- radInd sits in a PlaceObject3 tag that FFDec shows as `TagStub` and leaves out of its own render. It was decoded by hand.

Files:
- `art/`: native PNGs.
  - Part bitmaps: `leg.png` (bitmap 104, 16×49), `body.png` (107, 22×36), `head.png` (110, 12×16), `arm.png` (113, 18×35).
  - `outline.png` (117, 63×107, white lines plus ground ring, drawn on top).
  - Icons: `icon_bleed.png/.svg` (shape 97, white drop, 10×15) and `icon_rad_tox.png/.svg` (shape 100, white trefoil, 20×19). **Rad and tox use the same trefoil** (sprites 103 and 102 both contain sprite 101). Each icon also has a `@4x` copy.
- `placement.json`: the full matrices. The canvas is sprite-119 space x 0..140, y −24.65..98.7, so canvas = (x, y + 24.65), size 140×124.
- `doll_recompose.svg`: an exact SVG recomposition. Each part is a `<rect id=lLeg…>` with `fill`/`fill-opacity`, masked by its bitmap's alpha (`mask-type="alpha"`), with the outline on top. It shows the reference state.
- References, each as `_1x`, `_4x_nearest` and `_4x_smooth`, transparent and `_on_dark`:
  - `ref_hit_body40_lleg0_*`: body 0.4 and left leg 0, everything else 1.
  - `ref_mixed_*`
  - `ref_authoring_*`: the swf's initial red α0.5 look with all icons. Compare it with FFDec's render in `ffdec_sprite119_authoring_*.png`. The two match, except that FFDec leaves out radInd.

Placement in canvas px, top-left of the bitmap. Subtract 63 from x to get coordinates in the 63×107 outline frame.

| element | depth | bitmap | x | y | mirrored |
|---|---|---|---|---|---|
| bleedInd | 2 | icon_bleed 10×15 | 11 | 86.15 | |
| toxInd | 4 | icon_rad_tox 20×19 | 30 | 84.15 | |
| radInd | 6 | icon_rad_tox 20×19 | 6 | 62.6 | |
| lLeg (id 3) | 8 | leg 16×49 | 93 | 41.05 | no |
| rLeg (id 4) | 10 | leg | 79.25 | 41.05 | yes |
| body (id 6) | 12 | body 22×36 | 82.95 | 12.6 | no |
| head (id 5) | 14 | head 12×16 | 88 | 0.3 | no |
| rHand (id 2) | 16 | arm 18×35 | 67.35 | 15.65 | yes |
| lHand (id 1) | 18 | arm | 103.1 | 15.65 | no |
| outline | 21 | outline 63×107 | 63 | 0 | no |

Notes on the figure:
- It faces the viewer, so the character's left (lHand/lLeg) is on the viewer's right.
- Depth 1 holds an invisible hit rectangle (shape 96, #000 α1/255) and `tempTx` is an empty debug text field. Both are omitted.
- The bitmap fills are the smoothed kind (fill type 0x41).
- For HTML, use the part PNG as a CSS `mask-image` on a div with `background:rgb(255,g,0); opacity:alpha`, or use the SVG.

## 3. Modification icons (`mod_icons/`) and lobby style (`style/`)

### Icon rule (PROVEN, `LobbyMenu.as:5224` + `IconContainer.as`)
- The atlas is `new IconContainer("modifiers", 38, 38, LobbyMenu_ModifierIcons, vertical=true, count=0, stepH=0, stepV=0, atStart=false)`. It is character 3279, 266×494, which gives **7 columns × 13 rows**.
- Icons are pushed **column-major**: `index = col*13 + row`, so the rect is `(38·col, 38·row, 38, 38)`. Equivalently, `col = floor(i/13)` and `row = i % 13`.
- `getIcon(ui_desc.icon)` indexes this array directly, so `ui_desc.icon` is the plain index.
- The icon pictures match their modifiers: 0 shield = armour, 9 cross = health regen, 18 gas mask = filters, 19 O₂, 32 bullet = barrel damage, 34 crosshair = dispersion, and so on.
- 40 cells are non-empty and saved as `mod_icon_<i>.png` (38×38 RGBA, grey #a0a0a0-ish art).
- All 35 indices used by the config exist: 0, 2–7, 9–21, 32–40 and 65–70. Indices 1 and 8 have art but no config user.
- `mod_icons.json` holds the index → rect → config users. `_contact_sheet_2x.png` shows the icons in index order, 13 per row, which is one atlas column per row.

### How the lobby draws an item's modifications (item hint `InventoryItemHint`, sprite 1547, class `ItemInfoRenderer`)

**Lists (PROVEN).** `sortModifiers` splits `item.modifiers` into three vertical ButtonBars with spacing 0:
- `defaultModifiersList` (renderer `SrvModifierLabeledRendererDefault` 2546) takes `isDefault` modifiers. Its back is #000 α0.85.
- `modifiersList` (renderer `SrvModifierLabeledRenderer` 1352) takes normal modifiers.
- `criticalModifiersList` (renderer 1352) takes `state==1` modifiers, the defects.
- Order: default list at `propsBottomLine.y+10`, then the modifiers (+5 if a default list exists), then a "Critical damages" separator (`modifiers.y+h+15`), then the defects (`separator.y+separator.h−10`).

**Row (sprite 1352, `ModificatorListRenderer`; PROVEN).** The row is 230×25 and grows with multi-line text.
- **Icon:** the 38 px icon goes into `iconContainer` at (0,−1) with scale 0.8065, so it shows at **≈31×31**, with smoothing.
- **Label:** the text field is at (**40**,4), w≈191, wrap+multiline.
  - Font **Calibri Bold 14 px** (font class `CalibriBold`, swf font 133), leading −2.
  - Default colour **#BCCBCB**.
  - DropShadow #000, blur 4, distance 1, angle 90°, strength 1, which is about `text-shadow: 0 1px 4px #000`.
- The default-row variant puts the icon at (5,−1), the text at (45,4), and a 10×27 grey bar at (−6,0) (`default_marker_bitmap2544.png`).
- **Defect rows:** the icon is made a flat **#960F0E** silhouette (`colorTransform.color = Colors.RED_DIFF`).
- Text is one line per property: `<prop_name>: <styled>±value</styled>`.
  - The sign is `+` if ≥ 0.
  - Additive properties (`mod_type` 1) show the raw value plus a postfix. All others show `value·100` plus "%".
  - The exe turns `mod_type==1` into `modifier_type 0` at about 0x221670–0x22170b. **LIKELY**
  - The value is **green #87A816** when `(direction==0 && v<0) || (direction==1 && v>0)`, otherwise **red #960F0E**. `direction` comes from `static_game_parameters.ui_properties`.
  - These colours belong to the row's own stylesheet. The global `MainScene.textStyleSheet` and the tooltips use green #5C7D47 and red #CF4242 instead.
- **Other text styles:** gray #5B6261, yellow #D79342, "current" #B3EA97.

**Hint panel (PROVEN, sprite 1547).**
- Width ≈250, contents 240 wide.
- `topBlackContainer`: #000 α216/255 (0.85).
- `bottomBackContainer` (behind the modifiers): #000 α89/255 (0.35).
- Separator lines use bitmap 41 stretched. Each is 2 px: #ECECEC α0.106 over #000 α0.40.
- Critical separator caption: Calibri Bold 14 #BCCBCB, centred, same shadow. Its left and right lines are 54 px each.
- Item name: Calibri Bold 14 (#46494A in the authoring file, recoloured by rank).
  - Rank colours: `Colors.RANK_1` #5D8FA3, RANK_2 #7EAA63, RANK_3 #EFB503, none #CCCCCC.
- Item level: 15 px #ACB4B6, right-aligned.
- Compare rows (`SrvItemCompareHintG` 1538):
  - Label #ACB4B6, value #CAAF84, diff #9BC3C7, all 14 px.
  - Bar 6 px high: base #AEA191, plus #5C7D47 / minus #CF4242.
- `orangeOverlay`: #814F2E α0.30.

`style/` holds:
- FFDec renders at 1× and 2× of the hint (`InventoryItemHint_1547_*`), both rows, the separator, a line, the compare block and its bar. These renders show the authoring placeholders "textField" / "урон".
- `mock_modifiers_2x.png`: a PIL approximation using the values above, the extracted font and real icons. It is not a Flash render.

**Fonts embedded in lobby_menu.swf:**
- Calibri Bold: ids 24, 133 (class `CalibriBold`), 154, 847 and 865.
- Calibri: 2803 (class `Fonts_CB_Bold`) and 420.
- Calibri Italic: 1819.
- Exo 2 Bold, Extra Bold and Black: 168, 2731 and 2733.
- Open Sans: Bold 330, Regular 2524, Extrabold 1857, Condensed Bold 857, Condensed Light 2535.
- Arial: Regular 2526/3126, Bold 1289/2530. Arial Black: 3058.
- Tahoma: Regular 2528, Bold 2572.
- Impact: 616.

The modification text uses **Calibri Bold**.
- `style/fonts/CalibriBold_swf133.{ttf,woff,woff2}` (834 glyphs) and `CalibriBold_swf24.*` (1508 glyphs) are exported with FFDec. Both cover Latin and Cyrillic.
- ⚠ **Calibri is a Microsoft font, and these are subsets of it.** Use them for local comparison only. For a public page, use **Carlito**, which is metric-compatible with Calibri and licensed under OFL, in bold.
