# content-edit: game data changes as resources.db patches

`content_edit.py` makes a patch folder by editing the live game configs in place:
only the bytes of the edited values change (a renamed key's table is re-sorted by
name CRC, as the game's lookup needs; new names and strings go after the file's
last byte), and every edited file is read back and compared with the live one
value by value. It refuses a value whose live number or card text is not the one
the edit list expects, so an edit list made for one release cannot silently land
on other data.

## Making the patch

```bash
# the live files: the server game data the live server runs on holds them
dbgrab server-game-data.db get gameplay/items/ live/      # tools/re-kit/common/dbgrab
python3 -I content_edit.py weapon-balance-1.json live/ folder/
release/build-patch folder/ <revision>                    # resources.db.patch_<revision>
```

Then build the server data with it (`srp-game-data build`, or `release/release.sh`
with the patch) and run `weapon_balance_1_changes_only_its_values`
(`crates/survarium/src/v026g/maps.rs`, ignored: it needs `SRP_TEST_RESOURCES_DB`,
`SRP_TEST_SKHID_PATCHES` and `SRP_TEST_PATCH_WEAPON_BALANCE_1`).

## weapon-balance-1.json (2026-10-18)

Scored against the recorded live matches since revision 19 (about 10,000 gun kills):
each victim's real armour, the killer's ammo and the distance, through the damage
chain read in the exe (damage x (1 - clamp(armour - piercing, 0, 1)), both falling
to 60 % between the weapon's two damage distances). "Kill index" = kills with the
weapon / kills the same players get with their other weapons.

| change | why |
|---|---|
| Armour piercing of the 0.29a3 weapons on the 0.26g scale, by calibre: 5.56 down (HK416 0.45 -> 0.34, M4A1 and MSBS 0.42 -> 0.32, FNC 0.40 -> 0.30), MP7 and P90 0.55 -> 0.30, KRISS 0.45 -> 0.25, PKP 0.60 -> 0.45; 7.62x39 up to the AKM's 0.40 (vz. 58, Valmet, SOK-94 0.30); 9x18 up to the Kiparis's 0.20 (Kedr, AEK-919K 0.10) | the port brought damage back to 0.26g's scale but kept 0.29a3's piercing, a different scale per calibre (0.29a3 moved wall penetration into a key 0.26g does not read); piercing also decides wall penetration |
| MP7 damage 0.35 -> 0.40, FNC 0.45 -> 0.50 | keeps two guns that played average or weak (kill index 0.9, 0.75) where they were after the piercing fix |
| KRISS damage 0.45 -> 0.42 | kill index 1.22 (1.09-1.36) |
| SR-2M 900 -> 800 rounds a minute | kill index 1.33, the best accuracy of the automatics |
| SR-3M (and premium) 900 -> 800 rounds a minute, damage 0.55 -> 0.45 | the fastest automatic in the recorded kills, kill index 1.22 |
| AS Val, 9A-91, VSS, VSK-94 (and premiums) damage 0.55 -> 0.50 | the same 9x39 numbers as the SR-3M |
| PP-2000 750 -> 680 rounds a minute | kill index 1.22 |
| Mosin damage 1.0 -> 1.1, piercing 0.35 -> 0.40; TOZ-122 damage 1.1 -> 1.2 | the two beginner rifles played weakest (kill index 0.54, 0.66) |
| `stamina_regenation_speed_modifier` -> `stamina_regeneration_speed_modifier` in every item config and in the modifications' paths (5 modifications, 1 defect) | the exe reads only the correct spelling (name table 0x928448 entry 3); the items' stamina-regeneration modifications and defect did nothing |

The lobby card shows text kept in each config (`ui_desc/props_list`), not the
values: the list changes that text with each value (`set_prop`).
