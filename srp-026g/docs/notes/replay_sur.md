# The 0.26g client's own replay file (`.sur`), and a converter from match recordings

**Version scope:** client **0.26g** (build 2777, patch revisions 7 to 10 on the players' server). Every address is a VA in the original 0.26g exe, read with `tools/re-kit/common/scripts/dis.py 026g` and `xref.py`. Written 2026-10-15.

**Status: the format was read from the exe; no file made here has been played by a client yet.** The code (`servers/match-server/src/v026g/sur.rs`, `sur_convert.rs`, the binary `srp-recording-to-sur`) makes files that pass their own checks and parse back with a reader written from the client's. Whether the game plays them is the first thing to find out (section 6).

Tags: **PROVEN** = read in the 0.26g code at the address given. **LIKELY** = strong inference. **GUESS** = weak.

## 1. What the client has

The 0.26g exe has a replay player the 0.21d exe only began (0.21d: `replay_match_reader::open_replay` with three fixed header reads, no key frames). PROVEN (strings and code):

- the command-line key `-match_replay=<name>` (description "replay match"; key object 0xA9C9B0, read with `is_set_as_string` in `game::on_base_resources_created` at 0x1EB099, after `-spectator`);
- a file reader, `replay_match_reader` (0x694 bytes, constructor 0x1F03D0), with **key frames** (`key_frame_type`, at most 128), seeking (`game::enqueue_replay_seek_to` 0x1EE3C0 -> 0x1EE2E0) and the check "Replay of other engine version is used" (0x1F067A);
- a replay UI, `resources/flash_movies/replay_player.swf`: play/pause, speed 0.01 to 10, a seek bar over the match's length, one-frame steps, a player list (`select_player(id, 0|1)`, first or third person), a free view, `smooth_replay_camera`, `toggle_replay_ui`;
- no lobby list of replays, and **no writer**: the strings "replays" and ".sur" each have one reader, nothing in the exe opens a `.sur` for writing, and `key_frame_type::push_back` is called only by the reader (LIKELY: no writer path found). The game's own recorder is the journal (`-record_journal`), which is off since patch revision 4 and is another thing (it holds the packets one client got and its device input, and plays only with `-replay_journal`).

## 2. Where the file is

PROVEN (0x1F02C0, 0x1F0FA0, 0x1F1020):

- `<user data dir>\replays\<name>.sur`; the user data dir is `Documents\Survarium` (LIKELY: it is the engine's `get_user_data_directory` as in 0.21d, where the journal and the logs are);
- if `<name>` has a `:` before any `\` it is a path and is used **as given**, without `.sur` added.

The file is read whole into a resource of its size + 0x1800000 (0x1EB165..0x1EB3AD).

## 3. The file

```text
file  = chunk*                        any order; ids 0..=4 must each be there
chunk = u32 id, u32 word, size bytes  size = word & 0x3FFFFFFF; the top 2 bits are read and ignored
```

PROVEN (0xF1490 / 0xF1540). A missing chunk is not checked for: the reader would use garbage.

| id | content | tag |
|---|---|---|
| 0 | raw: a NUL-terminated string (skipped), the string `0.26g0` (compared with the exe's own, 0x9DAE98), `u32 8` (the format version, kept at reader+0x63C and handed to the world's `deserialize` for every key frame) | PROVEN (0x1F060D..0x1F068D) |
| 1 | LZMA2: the match options, **a 0xE4 payload without its first byte** (the local player index): `static_info.md` | PROVEN (0x1F08E0 -> `match_options::deserialize` 0x1F0E70, the function `process_static_match_info` uses) |
| 2 | LZMA2: the body, records, section 4 | PROVEN (0x1F069C..0x1F0735) |
| 3 | `u16 count`, then LZMA2 of `count` entries `u16 len, u32 body_offset, len bytes`: the key frames | PROVEN (0x1F0AC0) |
| 4 | LZMA2: `u32 len`, then `len` bytes of a Vostok binary config; the game relocates it in place (0xF6D00) and asks it for `mode` -> `match_making_params` -> `map_name` | the query PROVEN (0x1F0980, 0x218F08..0x218F3C); that the blob is the mode's `gameplay/game_modes/<config>.options` file as it is in `resources.db`: LIKELY (those files hold `mode{... match_making_params{map_id, map_weather_id, map_name} ...}`, `data.md`) |

**LZMA2.** Every chunk is a raw LZMA2 stream, decoded by the same wrapper and function as the 0xE4 options and the 0xE8 world (0x357EB0, dictionary 0x100000); the result is not checked. Stored chunks do (`lzma2::stored`), and so do compressed ones (`lzma2::packed`, as the worlds of 0xE8 go to the clients).

**Memory.** All decoded chunks (in the order 2, 3, 1, 4) go into one 8 MiB area; **the reader does not check the total**, nor the 128 key frames. The writer refuses both ([`sur::write`]).

**Key frame.** `data` is `u32 time_ms` and then the body of a 0xE8, which is the body of an upload (`client_patch.md` 3.4): `u32 active_clients_mask`, the player blocks, the six sections. It goes to `game_world_core::deserialize` 0x2D0B70 with the version of chunk 0 (8), as 0xE8 goes with the constant 8 (0x27C0D9) - PROVEN. `body_offset` is where in the decoded body the reading goes on after it (0x1EE31E..0x1EE347). The player finds the last key frame with a time at or before the time asked (0x1E8EC0).

## 4. The body

```text
u8 type, u32 time_ms, then:
  0, 4         nothing more
  5..=20       f = type - 5
               f & 8: u32 mask, a u8 for each set bit (0..20)          0xF3 kit choices
               f & 4: u32 mask                                         entered
               f & 2: u32 mask                                         left
               f & 1: u32 digits, then for each non-zero digit a u32 (1) or a u64 (2)
```

PROVEN, the reader 0x1F0BE0 (types 1 to 3 are never right). The digits are 20 base-3 numerals, player 0 the most significant (checked against the client's two loops, a test in `sur.rs`). A u64 is `{u32 rotation = two half floats, u32 actions mask}`: the slot array that the record is read into is initialised from each player's current actions mask in its high half (0x1EDFD4: `[player+0x330]` at slot+4) and from a zero rotation in its low half, and a u32 only writes the low half - so the rotation is first (PROVEN from the slots; that the two halves are what 0xEB carries, in this order, is LIKELY). The writer always writes u64.

**Playing** (0x1EDEF0, called with the game time t): `target = (t / 50 + 1) * 50`. While the reader's cursor (the time of the last record read) is behind `target`: the world is moved to the cursor, the next record is read, the cursor becomes its time, and then the record is **applied at once, whatever its time**: `enter_match(player, time)` 0x2D1470, `leave_match` 0x2D18A0, the kit choices (`change_input<profile_input>` 0x1FB730), the inputs (`change_input<player_input>` 0x1FAB50; the "precise" flag is `time % 50 != 0`), in this order; a type 0 record ends the loop, type 4 does nothing. PROVEN (0x1EDEF0..0x1EE2DE).

So a record is read as soon as the one before it is behind the tick being played, and a record far ahead would be applied early. The writer therefore ends every tick's records with a **type 0 record timed at the start of the next tick that has records** (and the file with one): the cursor then stays ahead of the ticks until that one. (`sur.rs`, the tests `a_tick_is_closed_before_the_next_ticks_records`.) The loop is PROVEN; that these records make it stop where wanted is read from it, not run in a game (no file of the original server's was seen, and type 0 is the only way the loop stops).

**Start and end.** The start is the time of the body's first record, and `on_match_replay_ready` (0x218FC0) uses nothing else of it: it must be a type 4 record at the start time. The end is the time of the last record (0x1F0D20), so the last record is a type 0 at the end. The world is built fresh from chunks 1 and 4 (`game_world::load` 0x1F8CA0, `serialize` 0x2D04F0) and its clock set to the start.

**Key frames while playing.** The 50 ms grid reaching the time of the next key frame calls `enqueue_replay_seek_to(time - start, true)`: the key frame is deserialized, the body reading goes on from its `body_offset`, the clock is set (0x218BF0, 0x1EE2E0). So the sim is re-synchronised at every key frame: a replay that differs from the match in between is put right there. Key frame 0 is used only when seeking back to the start (index 1 is the first one watched, 0x218DF3).

## 5. The converter

`srp-recording-to-sur <file.srprec> --game-modes <dir> [--out <dir>] [--stored] [--every <ms>]` (`cargo run -p match-server --bin srp-recording-to-sur`). `<dir>` holds the game's `gameplay/game_modes/*.options` files under their own names (`tools/re-kit/common/dbgrab get gameplay/game_modes/ <dir>` writes them as `gameplay__game_modes__<name>`; strip the prefix); the one the recording's `map_id` names in `maps_dict` is used. It prints a summary and writes `<name>.sur`.

| `.sur` part | made from |
|---|---|
| chunk 1 | the stored LZMA2 stream of the 0xE4 of the place that got the **largest roster** (with late joins, the roster grows: 6 to 20 seen), without its first byte |
| chunk 4 | the mode config file |
| body | the relays the server sent to the places: 0xEB, 0xEC, 0xED, 0xEE (inputs), 0xF3 (kit choice), 0xE9, 0xEA (enter, leave), every message of every place put together and a copy of one dropped (one input at one time of one player is one input: 1.5 to 4.3 million copies in a match, **0 conflicts**) |
| key frames | the uploaded worlds (0xE0 chunks of patched clients, `Reassembler`), one body for each item time (the most common one when clients disagree: 1 to 2 times in a match), the first world, one at least every 10 s, and **one soon after each late joiner enters**, at most 128 |

Choices, each one in the summary:

- **The match clock** is the recording's time less a constant, the median over the notes that carry both (`entered`, `clock`: 1300 to 2300 ms); its use is only to read the relays' one-byte ticks (`tick_time`, as a client does). An event the clock puts more than 1 s ahead of the recording is a late copy (a replay to a resyncing game) of an event 12.8 s older, and is dropped (0 in all 20 samples).
- **Inputs, not the players' own packets.** The relays hold what the server accepted (late inputs moved, refused ones dropped, `game.rs::input`); each input is relayed to every place but its owner's, so the union over the places is all of them. A match with one place only would lose its inputs.
- **A 0xEA to the leaver itself** is its farewell (tick 0 for a game that never loaded), not a leave: ignored.
- **The start** is the first uploaded world's time (50 ms in every sample); the first record is the marker.

**Samples.** All 20 recordings made 2026-10-14/15 convert and verify (`sur::verify`: the five chunks once, the version chunk, a body that parses with a marker first and a type 0 last, key frames in order that point at record boundaries, nothing after a key frame's time skipped, under 8 MiB). Sizes (`--stored` is about 2.9 times larger than packed: 2.2 MB for the first match below):

| match | roster | recording | `.sur` packed | key frames | inputs |
|---|---|---|---|---|---|
| level_07_evn_4, mode 4 | 10 to 13 | 8.1 MB | 0.77 MB | 95 | 97 709 |
| level_02_4, mode 4 | 11 to 20 | 13.5 MB | 1.17 MB | 96 | 148 801 |
| level_05_evn_2, mode 2 | 6 to 10 | 10.0 MB | 0.80 MB | 94 | 101 610 |
| level_07_4, mode 4 | 2 to 7 | 2.0 MB | 0.24 MB | 94 | 28 399 |

Overall a `.sur` is about **a tenth** of the recording (0.24 to 1.2 MB against 2 to 20 MB), because a recording holds every datagram once for each place and a `.sur` holds each event once. The body is 1.0 to 1.5 MB decoded (about 10 bytes for each of 100 to 150 thousand inputs), the decoded total 2.2 to 3.4 MB (the client's limit is 8 MiB).

## 6. Not proven, and the first run

Not proven (each would show on a first run):

1. **That the client plays a file of this layout at all**: chunk 4 as the raw `.options` file, the first string of chunk 0 being free, `-match_replay` working without the launcher or a sign-in, the replay needing nothing else in the folder.
2. **The patch revision of the viewer must be the revision of the recording's clients** (LIKELY): the world is a key frame's `deserialize`, and the patch revisions change what the world holds (revision 9 raises the statistics' player count to 20, `client_patch.md` 3.16; revision 10 the weapons' random state). The 10 recordings of 2026-10-14 09:04 to 13:36 UTC were made by games of revision 7 or 8 (the minimum was 7, revision 8 released 03:48 UTC), the 10 of 21:29 UTC on and after by games of revision 10 (the minimum was 10 from 21:26 UTC). Use a recording of the revision the exe is.
3. **Late joiners**: the client's late join (the exe's `lj_native_install`) does not run in a replay, and `enter_match` for a place beyond the first roster has not been seen to work. Every sample has them (the recorder keeps a match once 6 places have entered, and the server fills a match). The key frames soon after each entry carry the newcomer's player in the world, so it should appear at the latest then.
4. **The start**: the fresh world is built from the options and the config; the first seconds differ from the match if it does not agree with the first world (50 ms) until the first key frame (about 10 s).
5. **The record rules** of section 4: the type 0 at every tick's end, and that a seek lands where the writer's `body_offset` says (first record **after** the key frame's time, as the server's `send_replay` takes inputs `> snapshot time`).
6. **Statistics of the replay** (the UI's score screen): the replay restarts the match's score; a replay does not run the server's rules.
7. A recording made with `SRP_RECORD_MIN_PLAYERS` below 2 places has no relays to take inputs from.

**The first run** (not done: the owner's decision to run a game):

1. Make the file: `srp-recording-to-sur <recording of the exe's revision> --game-modes <dir> --out <dir>` (the stored variant too, `--stored`, to exclude the encoder).
2. Put it in `Documents\Survarium\replays\<name>.sur` (make the folder).
3. Start the exe of the same revision from its own folder (`game\binaries\x86`), as `play_skhid.bat` does, with `-match_replay=<name>` added: `survarium_skhid.exe -localization=english -mt_allocator=768 -minimum_resources_memory_size=768 -match_replay=<name>`. If the name is not found, the full path `-match_replay=<drive>:\...\<name>.sur` is the other form.
4. Look at: a crash (the dump is beside the exe), "Replay of other engine version is used" (chunk 0), the map loading, the replay UI, whether the players move and whether the clock and the key frames keep the world together (a world that drifts is put right every 10 s).
