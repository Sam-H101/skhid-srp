# Survarium 0.21d (build 2010): scoping summary (2026-10-10)

Paths written `S/...` in these notes are the scratch folder of the scoping session (disassemblies, dumps); they are not in the repo. The 0.21d install itself is at `E:\sur-res-pro\downloads\v021d\extracted\game\` (from archive.org, `survarium_full_021d.7z`).

Detailed reports in this folder:
- login.md
- lobby.md
- option_c.md (can the client be the match server? no; option B is cheap)
- option_b.md (the snapshot-upload patch: hooks, code cave, wire format, live checks)
- lobby_spec.md (byte layouts of every lobby message; examples in lobby_spec_examples.txt)
- match.md
- snapshot_format.md (byte layout of an uploaded snapshot: statistics and game-rules sections, match-over and winner rules, mapping to lobby results)
- clock_sync.md (floating_timer and 0x55, input send timing, finality, the 0x55 policy for srp)
- data.md
- persistence.md (accounts in SQLite, sign-in and passwords, what is kept and how it is saved; no real passwords in tests or logs)

All findings come from the disassembly and PDB plus the game data. Nothing was run against a 0.21d client.

## Verified by hand (key claim)
- **Frozen until the first snapshot.** `game_world_core::move` 0x1CCCA3 returns at once while byte +0xC80E is set (check at 0x1CCCB4). The constructor sets it at 0x1CA5F8..0x1CA609: `C80E = !(server_mode || replay)`. It is cleared only when a 0x53 snapshot is deserialized (0x1CAD89).
- **Server mode.** `server_mode` comes from `pvp_match_core_cook`+0x21 (on_resources_loaded 0x1C77CA). That byte is set to 0 in `game::game` (0x151603) and written nowhere else. The stock client can never run as the authority.
- **Replay mode.** `replay` is user-data +0x14:
  - `network_client::query_players` 0x1A2852 sets it to 0;
  - `replay_network_client::load_replay` 0x16C1FD sets it to 1. The client has a replay player that needs no snapshot.

## Verdict per layer
| Layer | Reuse from srp | Work |
|---|---|---|
| Login, browser, sessions | high | small: version field, 12 extra reply bytes, UDP ping to login port (5 B) |
| Lobby and chat | framing, matchmaking and 0x20/0x33 unchanged | large: the server pushes all data unasked; new profile and inventory format (23 slots, 8 profiles, modifications, 83 items); 0x38 counter mandatory (Play button, quit on 0); stats and rewards rewritten; chat ids moved |
| Match transport | ack/resend/order ideas | substantial rework: 12-byte header, 64 ack bits, 508 B datagrams, per-message channels, u16 lengths, 0xFF multipart |
| Match logic | almost none (combat, spawns, weapon FSM, traps, batteries not needed) | relay is small (clock, 250 ms finality, input log, 0x58 discard, time sync, hash voting); **snapshot source is the blocker** |
| Data | resources.db reader and config parser work unchanged | binary server_project reader; 6 maps; item compatibility and slot rules now in data |

## Snapshot options (a client patch is unavoidable)
- **A. Skip-snapshot patch.** C80E starts at 0, and 0x52 must not clear the initial state.
  - Server: small (transport + relay + static info).
  - Cost: no late join, no resync. Any desync is permanent, and everyone must join at the start.
- **B. Patched client uploads its snapshot.** The server checks the CRC against the majority's 0x46 hashes, then forwards it as 0x53.
  - Gains: late join and resync work.
  - Cost: the patch needs a send path.
- **C. Headless 0.21d in server mode.** Flip 0x56E825D and drive it from srp.
  - Gains: bit-exact snapshots and results.
  - Cost: hardest; needs a client without rendering plus networking glue.

## Other blockers and risks
- **One client per Windows session** (mutex `survarium_or_its_installer_is_already_running`). Two-client tests need two machines, a VM or a patch.
- **No binary-matching decompilation of 0.21d.** `tools/vostok` covers 0.100b only, so everything comes from raw disassembly with symbols.
- **Results.** Match results and rewards must be computed server-side; clients report nothing. With option B or C they can come from the statistics part of the snapshot.
- **`-debug_hash_mismatches`** makes a client build field-named serialization descriptors. Running one would document the snapshot format.

## Planned after the match server: match results
Results are a separate step that starts only once a match runs end to end. The screen and wire format are in `lobby_spec.md` §3.17.
1. Map each field of `match_stats` / `player_result_full` to its meaning (kills, experience, money, drops).
2. Reverse `shared_statistics::serialize`, the statistics part of the snapshot.
3. At match end, srp requests a final upload (0x59), checks its CRC against the majority's 0x46 hash, and reads the statistics from it. The fallback is a patch extension that sends the finished statistics directly.
4. Compute rewards from the 0.21d data (levelling, drops) and apply them to the profiles.
5. Send the `#mstats [<match_id>]` stats line, and answer `21 13` with `36 13`. Also fill 0x26 `just_left_match` / `pending_match_stats_id`.
