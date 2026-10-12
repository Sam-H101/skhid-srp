# 0.21d game data vs 0.100b

## Download
- **Source:** archive.org item `vostok_engine_v0.21d_build_2010_internal_id_839_apr_24_2014`, file `survarium_full_021d.7z` (3,100,086,596 B).
- **Integrity:** MD5 2d93de29b26ecfd4faa1fa0315140c9c matches archive.org.
- **Location:** `E:\sur-res-pro\downloads\v021d\`. Only `game/` was extracted, to `extracted\game\`. Skipped: `downloads/torrent/021d.sup` (1.5 GB updater payload) and `temp/`.
- **Contents:** a full install.
  - `binaries/x86/`: survarium.exe, the same SHA-256 (def82eea…) as the binary-evidence copy already disassembled, plus PDB, launcher, service, updater (each with PDB), bugtrap.dll, apatch.exe.
  - `launcher/` (HTML/JS), `resources.db` (3.6 GB), `resources/ssl/survarium_login_server.crt`.
- **Nothing was run.**

## Findings
| Item | 0.100b | 0.21d | srp impact |
|---|---|---|---|
| resources.db format | header `little-endian`, 12,559 entries | same header, 52,702 entries | srp's reader (`vostok-resources-db`, via scratch `dbgrab`) opens it unchanged |
| binary configs | srp `binary-config-cli` parses | parses unchanged (player, server_project, dictionaries, options) | reusable |
| login cert | `survarium_login_server.crt` (expired 2014) | byte-identical file | same key, so srp's re-issued cert (until 2037) can replace it the same way |
| maps | level_03 (3 lighting variants) | 6 maps (level_01..06), each day/rain/evening | more content |
| map data | `server_project` + plain `server_project.lua` | binary `server_project` only (no .lua) | srp reads the .lua, so a new reader is needed (the binary one parses fine) |
| map objects | no anomalies, no `team_base_points` | damage_zones 1..21, generic_anomaly_cores 1..7, artefact_containers 4..27, respawn points 20..24, 2 battery containers, 30..51 battery spawners, `team_base_points[0]` everywhere | anomalies and artefacts exist, base capture still has no data |
| item dictionary | `db_static_dictionaries`: boosters, perks, skills, items (54), factions | adds items_compatibility (41), categories_dict (24), slots_restrictions (77); items 83, boosters 14 | slot rules srp hard-codes for 0.100b (`restrictions.md`) come from data in 0.21d |
| new gameplay files | — | `static_game_parameters.options` (46 UI item properties), `test_player_profiles.options` (10 test loadouts), `db_skills_tree`, `gameplay/drops`, `quests`, `devices`, `modifications`, `effects/client` | the lobby must serve more item kinds |
| breath | `character_breath_holding_params` | `character_breath_vibration_params` (e.g. max hold 5 s, penalty 0.2) | matches the disassembly comparison |

Extracted samples are in `scope021d/data/`, with text dumps in `scope021d/txt/`. File lists: `db021d_list.txt`, `db0100b_list.txt`.

## Not checked
- Whether the client starts without the launcher. See `login.md`.
- Lua or other scripts in 0.21d.
- The UI swf contents.
