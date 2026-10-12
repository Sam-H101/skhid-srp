# 0.26g: upstream's live-tested match server against our notes and code

**Scope.** Upstream = commit `8204c88b7` (`origin/feature/server-026g`, repo `E:\sur-res-pro\srp`), read with `git show` only: `scripts/protocol-research/026g-findings.md` (cited "F:line"), `servers/match-server-026/` ("S:" = `src/server.rs`), `client/026g/`, `crates/survarium/src/protocol_026*`, `crates/vostok/src/network_packet/udp_026.rs`, the fixtures. Ours = `srp-026g` notes and code as of 2026-10-12. Written 2026-10-12. Nothing was run against a client for this note.

**Evidence words.** *live* = upstream saw it with real 0.26g clients (all of them under Wine 11.8 + DXVK on Linux, none on Windows). *oracle* = upstream ran the original exe's own function on test vectors. *static* = read in the disassembly (theirs or ours).

## Summary

Conflicts that can break our first live session, worst first:

1. **Launch flag.** Map loading ran out of the 376 MiB multithreaded arena; `-mt_allocator=768` fixed it (live, F:201-204). Our shortcut has only `-minimum_resources_memory_size=768` and our client has never loaded a map.
2. **The client does join messages by default.** Upstream saw a snapshot chunk bundled with 0xDA in one multi datagram (live, F:224-225). Our "one message per client datagram unless `-join_net_messages`" (transport.md §2.5, LIKELY) is wrong. Our upload survives only if the patch's "never joined" bit works as read (client_patch.md §3.2, not run); if it does not, no first sync completes.
3. **Resync.** Replaying a client's own inputs to it after a resync made both worlds diverge (live, F:380-388): 0.26g *moves* a late relayed input instead of rejecting it. Our `finish_sync` replays every input, the client's own too. Upstream's proven resync is 0xE8 alone with the next generation, then only other players' inputs.
4. **Silent clients.** Unfocused clients stalled for up to 51-58 s (live, Wine; F:267, F:313); upstream's server waits 120 s. Ours drops a match client after 10 s and a lobby link after 30 s. Two clients on one PC means one is always unfocused. (2026-10-12, later: by its code the client ticks its lobby link every 500 ms with the window out of focus too, transport.md §6.1; the lobby now gives a player in a match 120 s and keeps a silent link's session a minute for the client's own reconnect.)
5. **Wine only:** the client dies in `qwave.dll.QOSSetFlow` when it connects to the lobby (live, F:1340-1341). Not a Windows problem; it blocks Linux players of our full stack.
6. Minor: the retail header parser takes 18 tail ack bytes, ours refuses more than 17 (oracle; a real client sends at most 17).

What their results confirm of ours: the whole transport framing (637 of 653 oracle rows pass through `udp_transport`, the 16 others are item 6); stored LZMA2 chunks (oracle); the static-info layout, kit state 0, slots 7/8, map ids; the upload chunk format and the 50 ms empty initial world; the order 0xE5 → 0xDB → 0xE8 → 0xE9; six blobs, timelimit rule first, the statistics layout (365 live blocks); 0xE6/0xDB/0xE7 fields, sign and 5 % slew; `-hash_check_interval_in_ms=50`; own-index 0xEA ends the match; the login reply's IP byte order; squads (0xA1, 0x4A). **Upstream has no matchmaking for 0.26** (topic 11).

## Per topic

### 1. UDP envelope, acks, parts, lobby delivery

| point | theirs | ours | verdict |
|---|---|---|---|
| header `seq rseq ack0`, tail bytes read backwards, flag bit 0 = multi | `udp_026.rs:40-80`, oracle 416 rows (`026g-header.tsv`), live | transport.md §2.1-2.2, `crates/udp-transport/src/ack.rs:55-110` | **agree**. Checked by a throwaway program (scratch dir) that fed every row to `ack::decode` / `ack::encode`: 400 rows identical |
| 18 tail bytes | retail parser accepts them (16 oracle rows, `ack_bits` 127); upstream's server sends 18 on **every** datagram (`S:93-99`, `ack_bits: 127`) and clients took it live | `ack::MAX_TAIL = 17`: `decode` returns `None` (`ack.rs:23`, `:95`) | **conflict, theirs right** (oracle + live). Harmless today: the client's depth is capped at 120 → 17 bytes (transport.md §2.2) |
| per-id lengths, x byte, parts, `FF u16` prefix | `protocol_026.rs:57-115`, oracle 77 match rows + 160 lobby rows | transport.md §3-§5, `wire.rs`, `table.rs` | **agree**: all 237 rows parse through `wire::parse` with the same id, payload length, class, x, part index and count |
| 0xFE | parser takes it, dispatcher has no entry: never send (F:126-129) | same (transport.md §3.4) | agree |
| max datagram | 512 (`udp_026.rs:10`); the largest they ever sent is about 505 | 508 = receive buffer 0x1FC at 0x3F25D9 | **conflict, ours right** (static; theirs untested above 508). Keep 508 |
| one message per datagram from the server | always single format (`S:98`) | we also send multi datagrams | new: multi S→C is proven by oracle only, not live |
| client joining | E2 was bundled with 0xDA live (F:224-225) | "off by default" (transport.md §1 row 17, §2.5, open question 4) | **conflict, theirs right** (live). Our parser handles multi datagrams with fixed lengths; the risk is the 0xE0 chunk, topic 4 |
| lobby: all ids variable, 0x86/0xA7 null, 0x9F plain, flags 0x20 vs 0x60 | F:1292-1303, `protocol_026.rs:57-64` | transport.md §4.1 | agree. They state the need for ordered delivery; only we implement it |
| empty variable body | the retail parser **asserted** on empty lobby bodies in their oracle (F:1306-1307) | not noted for the lobby (only E4/F6 "needs ≥ 1 B") | **new**: never send a lobby message with no payload bytes. Check our lobby writers once |
| resend policy | E5 every 200 ms until 0xDB; E8, E3, E4 every 500 ms for ever; E9..EC 50 ms / 250 ms; replay messages kept until acked (`S:129-137`, `S:993-995`) | transport.md §4 values; E8 1.75 s then a new generation (match.md §13) | agree in kind |

### 2. Login redirect, QoS, TLS

| point | theirs | ours | verdict |
|---|---|---|---|
| reply `08 u32 session, IPv4, u16 port` (0x37FB10) | F:1309-1311 | login_chat.md row 26, §3 | agree |
| IP byte order | their probe's order sent the client to **1.0.0.127** (live, F:1338-1340) | `u32::from(ip).to_le_bytes()`, 127.0.0.1 = `01 00 00 7F` (login_chat.md line 168, static) | **agree; their failure proves our order**: they sent `7F 00 00 01` |
| TLS and version | client sent `0.26g0`, completed TLS against a local certificate, accepted the reply (live, F:1335-1338) | login_chat.md | **confirmed live** |
| QoS | client aborted in Wine's unimplemented `QOSSetFlow` on lobby connect, with no `-qos` flag (live) | "created when the `qos` option is given", polarity open (transport.md §8, open question 3) | **new**: QoS is on for the lobby link by default. Windows has qwave; Wine does not |

### 3. Static info, match options, LZMA

| point | theirs | ours | verdict |
|---|---|---|---|
| E4 = `u8 local index` + raw LZMA2 of the options | F:139-150, live | static_info.md §6 | agree |
| options layout | `match_options.rs:117-227`; oracle fixture `026g-options.hex` (436 bytes) | static_info.md §3.1, §6 | **agree**: the fixture reads field by field with our layout (team, name, clan, u32, K, M; kit: u8, u32, name, u32 slot mask, `u16 index, u16 count` pairs, mask high qword first; items; 19 pads; `3D 01`, id, level, 28 f32, 20 bytes) |
| kit first byte | `tag`, 0 in every live run (`S:271-275`) | `state`, "send 0" (static_info.md §3.1) | **confirmed live** |
| slots | 7 = weapon (count 1), 8 = its ammo (120 → HUD 30/90) (live, `S:276-283`) | `loadouts.rs` uses 7 and 8 | confirmed live |
| ammo pairing | dict 3 (AK-74M) needs 217 (5.45 7N10); dict 7 gave **zero usable ammo** (live, F:180-182) | from the dictionary (`fits`) | new detail: a wrong ammo id does not crash, the weapon is just empty |
| unused roster place | `FF 00 00 00000000 00 00` (team byte 255) | 9 zero bytes (team 0) | differs; theirs is the live one. LIKELY harmless (places ≥ N) |
| the profile's u32 after the clan tag | "key" = slot + 1 (live) | "protection", we send 10 | no conflict; note their players had protection 1 and 2, the user's "early shots did not kill" (F:261) may be that |
| score weights | 0.21d's 24, then 24..27 = 0 (`026g-development-scores.txt`); recompute 0x26DC30, 172 oracle rows: per event the three kits' sum (wraps to u16) × weight × (0.01 for 6, 17, 18), summed in f32, `floor(x + 0.5)` to i16 | `SCORE_WEIGHTS` identical (`static_info.rs:64`); formula per event `round(count × w × k)` (static_info.md row 14, `rewards.rs:19`) | weights **agree**. Rounding **differs**: theirs rounds once at the end (oracle). Same result for whole weights |
| event 22 | shots fired (live counters), 19 one byte, 22 two bytes | LIKELY bullet fire | confirmed live |
| squad bytes | 0 = none; 255 for all put everyone in one squad (F:1209-1215) | 0 = none (static_info.md §6) | agree |
| map ids | 61 `level_01_day_2`, 65 rain, 71 `level_02_rain_2` (School), all loaded live; test map 115 crashed loading | static_info.md §4 table; 115 excluded | agree, confirmed live |
| LZMA framing | raw LZMA2, dict 32768, lc 4, liblzma preset 9; oracle `026g-lzma.tsv` | stored chunks only (`lzma2.rs:56`), "not tested" (match.md §7) | **confirmed by oracle**: one fixture row is a stored chunk (`01 00 5F` + 96 bytes + `00`) and the retail decoder 0x3EE810 took it. Only single-chunk stored streams are covered |
| 0xE8 compression | always `compressed = 1` (`S:806`) with ≤ 32764 body bytes (F:217-218) | `compressed = 0`, raw (client_patch.md §3.4) | new: the raw path is static only. Not a conflict |

### 4. Where the worlds come from

| point | theirs | ours | verdict |
|---|---|---|---|
| carrier | new id **0xE2** C→S; range bytes patched (0x27A15B `E1`→`E2`), length and delivery getters wrapped, flags 0x1F (`direct_match.S:89-131`, `patch_exe.py:56-57`) | **0xE0** with a payload, single datagram only (client_patch.md §3.2, `wire.rs:253-262`) | different, both workable. Theirs live |
| chunk body | `u32 time, u16 offset, u16 total, ≤ 480 bytes`, offsets multiples of 480 (F:215-218, `snapshot.rs:47-62`) | the same (client_patch.md §3.4) | **agree** |
| joining | with flags 0x9F the client put the chunk and 0xDA in one multi datagram **without a length** (live) | our chunk is fixed-length 0xE0, so it has no length either; the patch clears bit 6 of pkt+0x98 so it is never joined (`snapshot_upload_026g.asm:198`) | **risk**. If the bit does what we read, fine. If not, the server sees `E0 <chunk> DA` in a multi datagram and drops it. A chunk describes its own length (`8 + min(480, total − offset)`), so the parser can be made safe either way (change 2) |
| initial world | hook at 0x27AE9A, newest history item, mask from the mask history, serializer 0x324BE0 (`direct_match.S:203-262`) | hook at 0x27AEA1, same item, same serializer | agree |
| what the initial world is | time **50 ms**, **empty** active mask, 2377-2530 bytes (live, several runs) | "taken from the first upload; GUESS on the grid" (match.md §13) | **confirmed**: 50 ms, on the grid |
| does the server edit it before sending | **no**. Slot 0's upload goes out unchanged as `gen, 1, LZMA2(u32 time + body)`; players appear through 0xE9 (`S:789-809`, live: both spawned) | same idea (0.21d's) | agree. Spawn needs no server-side change |
| start order | all 0xDA and uploads in → 0xE5 → 0xDB → 0xE8 → **all parts acked** → 0xE9 for everyone 300 ms ahead on the grid (`S:1260-1298`) | 0xE5 until 0xDB, then 0xE5 + 0xE8, then 0xE9 at `grid_up(clock)` with no lead (`game.rs:1586`) | agree in order. The lead is their policy; ours can make a client stamp the entry later than the others ("CORRECTED", match.md §5.1) if 0xE9 arrives late |
| 0xE9 time | time 0 was **rejected** live: must be > last resync time (F:205-207) | `clock.max(time + 1)` rounded up | agree |
| later worlds | **0xF7** S→C `u32 exact time`: the client searches its history for that item (`direct_match.S:157-196`). They ask for the tick of the 0xDF just received (`S:711-733`) | **0xF6** + 1 byte: "upload your event-horizon item now" | different. Theirs fails when the item has left the 11-item history before the request arrives (two expired requests even on loopback, F:312-313); ours does not depend on the round trip. **Keep ours** |
| size | populated worlds 3.2-4.7 KB with 2 players (live) | limit 0x8000, 69 chunks | new: 7-10 chunks in practice |
| blob boundaries | player blocks read forward, animation bit stream decoded (oracle, 253 envelopes); then 6 u16 blobs: bullets, statistics, objects, physics, rules, effects (`state.rs`); 157 live worlds end exactly | chained from the end, 6 prefixes (snapshot_rules.md §1, §6) | **agree** on the six blobs and order. New: a forward reader exists if ours ever mis-splits |
| nothing found that makes our upload fail | — | — | except the joining risk above |

### 5. Clock

| point | theirs | ours | verdict |
|---|---|---|---|
| 0xE5 | `u8 gen, u16 echo, u32 time`; clears the world, so only for start and late join (F:553-554). First generation 1 | same bytes (match.md §2.2); first generation 0; **also sent on every resync** (`game.rs:1353-1376`) | bytes agree. Use on resync differs: topic 7 |
| 0xE6 / 0xDB | E6 `u16 server ms, u16 advertised RTT`; DB `u16 RTT echo, u16 server ms + hold, u16 client game ms` (F:547-550, `clock.rs`) | the same reading, LIKELY (match.md §4) | **confirmed live**: 30 samples within ±1 ms of target |
| 0xE7 | signed i16; a positive value makes the client clock gain; about 226 ms gained in 4.6 s; a new 0xE7 replaces the running one (F:580-585) | 0.21d's handler, 5 % assumed (`CORRECTION_MS_PER_MS = 20`) | **confirmed live** (sign, rate, replace) |
| policy | one probe per 2 s, never resent; offset = half RTT after subtracting hold; median of 3 same-sign samples; 10 ms deadband; cap 100 ms | probe each second; least of last 4 samples | both policy. Theirs is the tested one |
| measurement trap | taking the receive time before blocking in `recv_from` gave negative RTTs (F:565-570) | — | check where our receive `Instant` is taken |
| generations | 0xE8 with the next generation and no 0xE5 is taken (live "FULL RESYNC") | "every resync must send 0xE5(g+1) then 0xE8(g+1)" (match.md §2.2 step 6) | ours is stricter than needed: 0xE8(g+1) alone works |

### 6. Hash reports

| point | theirs | ours | verdict |
|---|---|---|---|
| `-hash_check_interval_in_ms=50` | on every command line; about 20 reports a second per client (live) | "LIKELY syntax, not tried" (match.md §14, client_patch.md §3.6) | **syntax confirmed live** |
| default 100 ms | clients can report disjoint odd/even ticks (F:466-467) | same finding (match.md §14) | agree |
| comparing | same tick only; ticks resolved against the match clock, not from message to message (F:264-266); 1 s settling; ≥ 3 mismatches over 200 ms; the reference is the **longest-active** client | same-tick majority; with 2 clients `judge` | new: with 2 clients "longest in the match is right" is the tested rule; "lowest slot" restored the healthy client once (F:1256-1261) |
| after a late join | 15 mismatching ticks right after re-entry, cured by one recovery (F:1264-1269) | — | new: expect a short disagreement after a join |

### 7. Late join, recovery, replay, late inputs

| point | theirs | ours | verdict |
|---|---|---|---|
| resync of a running client | **0xE8 only**, generation + 1, compressed; wait for every part's ack; then the inputs after the world's time **of other players only** (`S:1590-1650`); live: 1071 and 882 matching ticks after | 0xE5 (repeated) then 0xE5 + 0xE8; discards of own ticks; replay of **all** inputs after the world, own included (`game.rs:1495-1500`) | **conflict**. Their first version echoed own inputs and diverged: 399 matching ticks, then 586 mismatches, both logs "input is moved" (F:380-388). The cause fits our own note: 0.26g moves a relayed input older than the horizon (match.md §5.1) where 0.21d rejected it. Our 0xE5 rewinds the clock, which narrows the window, but an own input within `delay − 150 ms` of the world's time is still moved |
| own input lists | survive the mode-1 load (live) | LIKELY (match.md §5.2) | **confirmed live** |
| late join | world ≤ 1 s old without the joiner; 0xE5 with the shared clock; 0xDB; 0xE8; acks; replay of inputs **and 0xE9/0xEA** in acked batches; then 0xE9 300 ms ahead (`S:897-1034`); live: 1443 matching ticks | world ≤ 5 s old (match.md §13); replay then `roster_change` | agree in kind. Two details to copy: replayed messages must not expire after 250 ms; one join at a time |
| who gives the world | longest-active client; a client being restored never | sources by `sources()` | see topic 6 |
| leave | 0xEA to the others 300 ms ahead; changed the real world (mask 2 in later worlds) (live, F:528-530) | 0xEA | confirmed live |
| reconnect | same session keeps its place after 5 s of silence; needs recovery on; last player's crash not resumable | by session (`attach`) | no conflict |
| late inputs | **dropped silently**: outside `(clock − 6 s, clock + 500 ms]` or before the player's entry; no 0xF1, no 0xEF (`history.rs:33-36`, `S:636-642`) | `discard` (default) or `move` | **no live evidence either way** for 0xF1/0xEF/0xF0. Open question 6 of match.md stays open |
| profile input 0xDE | ignored (`S:772`) | relayed as 0xF3 | no evidence; one kit per player in their runs |
| relay | never to the sender; half floats and the two time bytes copied as they are (F:317-321) | same | agree |

### 8. End of match and results

| point | theirs | ours | verdict |
|---|---|---|---|
| timelimit rule | 13 bytes `start, current, mask, state`; states 1/2/3; `wait_time × 60000`; first in the rules blob; 84 oracle rows; live: start 20500 after entry at 500 | snapshot_rules.md §2.2, modes_maps.md §10; "first: LIKELY" | **agree; rule order confirmed live** (185 worlds pass `current == time` at offset 0) |
| time end | from a CRC-checked world in state 3 with `current − start ≥ 900 000`; finished 1.1 s after the deadline (live, F:781-786) | own copy of the rule, set right from trusted worlds (match.md §13) | agree in kind |
| battery end | handler 0x313A50, completion = event time + 3000 ms, 0x314230; the deadline is **not** in the world; they wait 3 s of game time between two winning worlds; 144 oracle rows; live only with injected events | `END_DELAY_MS = 3000` after the world that shows it (modes_maps.md §11) | agree |
| statistics | `shared_statistics::serialize` 0x2F5AC0: 3 masks of 28 events, byte/word tables 0x920588 / 0x9205E0, damage, hit times, 10 bytes, u16; per battery 3 arrays + 10 bytes; 11-byte tail | snapshot_rules.md §3 | **agree; confirmed on 365 live blocks** |
| tail before any event | winner **3**, active-in-match mask **0** | code says 3 (`snapshot.rs:492`); the note says "not traced" (§3.3, §5) | agree; update the note |
| own-index 0xEA | closes the match (0x27C690 → 0x27A790 reason 3 → world finish 0x1F96F0 → results UI 0x29AAF0), then FA/FB (live). Its tick must be **now**: a past tick was rejected (F:1146-1149). Resent every 250 ms until acked | 0xEA to each client, resent while unacked (match.md §13) | **confirmed live**. Check that our tick is the current one |
| what the stock client shows then | without a lobby: the loading screen for ever (the lobby "sync" layer; `root.close_sync` removes it). 0x1ED1E0 lets the menu come up only when the lobby is connected. With the results call forced: the results window with a waiting spinner, until data comes | lobby pushes 0x97 (results.md §1); "when the client opens the screen: not traced" (results.md §8) | **new, answers our open question**: the client opens the window itself at the end and waits for the lobby's data. Nobody has shown 0x97 filling it live |
| the lobby reader | 0x2718C0 → 0x26E610 + `player_result` 0x26ECE0 → `show_match_statistic` → 0x29AB20 (F:730-733) | the same chain (results.md §1) | agree |
| C→S 0x31 | helper 0x2717E0: "a match identifier and a boolean" | `u32 order_id, bool left_match` (lobby_requests.md §2.1) | same message; we name the u32 from its source field |
| their F8 | S→C extension with rows built from a world, shown through `root.victory_screen.setData` | not needed (we have a lobby) | n/a. Useful: the Flash object needs `score_components`, `statistics_money`, `_reputation`, `_rating`, `_exp`, or `updatePremiumState` throws (F:806-811) |
| 0xE1 | "not a finish notification" | keep-alive | agree |

### 9. Squads and lobby codec

| point | theirs | ours | verdict |
|---|---|---|---|
| 0xA1 | u32 id; if not 0: u8 n, u8 commander index, n × (account, kit, revision, name, online, ready), u8 local commander, u32 revision; 3 members (0x274720) | social.md §2.1 | agree |
| 0x4A | 1 disband, 2 + name, 3 + account id, 4 leave | social.md §2.3 | agree. We know more: 2 is sent by itself on important message type 3 |
| 0xA4, 0x50 | F:1317-1325 | social.md §1.4 | agree |
| sign-in | 0x2E `u32 session, u8`; 0x80 two bytes; then 0x32 | lobby.md (the 0x80 entry), lobby_requests.md (`2E`) | agree |
| service | codec and a probe only; "not connected to a lobby service" (F:1287-1288) | whole lobby | ours has the breadth; nothing of theirs contradicts it |

### 10. Launch flags and environment

| point | theirs | ours | verdict |
|---|---|---|---|
| `-mt_allocator=768` | needed to load a map (live) | not passed (README.md line 47) | **add it** |
| splash | bare `-splash_screen`; `-no_splash_screen`, `-client` invalid | `-splash_screen=0` (works, step 0) | no conflict |
| debugging | `-log_to_stdout -write_errors_to_stderr -net_log`, `-terminate_on_timeout=100` | not listed | new, useful |
| `-spectator` | breaks match setup with their hook | — | new |
| exe patches | certificate chain call at 0x13509 (`E8 02 F5 FF FF` → `31 C0 90 90 90`); mutex 0x124F9; VRAM < 320 MiB check 0x106411 (`73 0F` → `EB 0F`) | certificate at the callee 0x12A10; mutex the same | agree. The VRAM patch is theirs only: needed where the detected VRAM is small (virtual displays); our client already opens its window |
| stalls | unfocused windows stalled up to 58 s; an auto-minimize branch at 0x3CF8A8 was bypassed without curing it | — | new; Wine evidence only |
| platform | Wine 11.8 + DXVK 2.7.1, one virtual desktop per client; Wine 11.0 failed. Windows: "same exe and arguments", not run | Windows | their live facts are all from Wine |

### 11. Matchmaking: what exists upstream

- **0.26g: none.** `match-server-026` serves one match with a fixed roster: `--players N` places, `--start-players` to begin, the first free place per new session or a `--roster` file of `session team squad name` lines (`roster.rs`, `S:442-484`). No queue, no levels, no team balancing, no lobby service, no hand-off. The client patch (`--direct-match`) calls `connect_to_match` 0x21BEB0 itself from `-server=host:port`, skipping login and lobby; the next match is the F6 key. The stock Play button's target was not found (F:1058-1064). The findings file says so: "Native invites, acceptance, readiness, and joining a match together are still unimplemented" (F:1348-1349).
- **0.21d branch** (`origin/feature/v021d-local-stack`, `docs/v021d/NATIVE-ADMISSION.txt`): rooms in a custom menu (Create / Join / Ready / Start) served by Python, not the game's own queue. Worth knowing for late join: the roster is fixed at static info, so a room loads descriptors for its first members and **reserves the other places with exact identities and loadouts**; a late joiner takes a reserved place through a revisioned grant; the scoreboard shows unused reserved places as grey rows; results list who took part, not who was reserved. In 0.26g a reserved place also counts against the 80 % start rule (modes_maps.md §10).

So the answer to "did they implement correct matchmaking logic for 0.26": no; ours is the only matchmaking for 0.26g.

## Recommended changes

Evidence strength: **live** > **oracle** > **static**.

| # | file | what | why | evidence |
|---|---|---|---|---|
| 1 | `docs/notes/README.md`, `../DEPLOY.md`, the shortcut, `client-patches/make_client_026g.py` | add `-mt_allocator=768` to the client command line; list `-net_log -log_to_stdout -write_errors_to_stderr` for test runs | map loading exhausts the default arena | live (Wine); the allocator is the engine's own |
| 2 | `crates/udp-transport/src/wire.rs` (`parse_message`, the `chunk` test) | also take a 0xE0 chunk in a **multi** datagram: when the bytes after `E0` are a valid chunk header (`time % 50 == 0`, `offset % 480 == 0`, `offset < total ≤ 0x8000`) and `8 + min(480, total − offset)` bytes are there, that is the message | the client joins by default; if the patch's bit-6 guard fails, the first sync would never complete | live (joining); the guard itself is static only |
| 3 | `transport.md` §1 row 17, §2.5, open question 4; `client_patch.md` §3.2 | joining is on by default | as above | live |
| 4 | `servers/match-server/src/v026g/game.rs` `finish_sync` | do not replay a client's **own** inputs to it (`from != index`) | 0.26g moves them; seen as lasting divergence | live |
| 5 | `game.rs` `begin_timing` / `send_sync`; match.md §2.2 step 6, §13 | resync a running client with 0xE8(g+1) alone (keep 0xE5 for the first sync and late join), behind a setting until tried | the only resync proven on 0.26g; 0xE5 clears the client's history and resets its clock for nothing | live for theirs; ours untested |
| 6 | `servers/match-server/src/v026g/mod.rs` (`client_timeout` 10 s), lobby `DEFAULT_LINK_TIMEOUT` 30 s | raise for the first sessions (60-120 s), or say in README.md which settings do it | unfocused clients stalled up to 58 s | live on Wine only; weak for Windows |
| 7 | `game.rs` `finish_sync`, `roster_change` | give 0xE9/0xEA a lead (theirs: 300 ms, on the grid) and keep replayed messages until acked | a late 0xE9 is stamped onto a later tick by that client alone | their policy, live; ours is 0.21d's and worked there |
| 8 | `crates/udp-transport/src/ack.rs` | accept 18 tail bytes in `decode` | the retail parser does | oracle |
| 9 | `game.rs` (`judge`, `sources`) | with two clients, trust the one longest in the match | "lowest slot" restored the healthy client | live |
| 10 | `game.rs` end of match | make sure the own-index 0xEA carries the **current** tick | a past tick is rejected | live |
| 11 | lobby writers (`servers/lobby-server/src/v026g`, `crates/survarium/src/v026g/lobby`) | check that no message goes out with an empty payload | the retail parser asserted on it | oracle (their harness; not seen in a client) |
| 12 | `crates/session/src/v026g/rewards.rs` score | sum the three kits per event, weigh, add in f32, round once | 0x26DC30 | oracle; no visible change with whole weights |
| 13 | notes only | snapshot_rules.md §2.1 (timelimit first: proven), §3.3 / §5 (winner 3, mask 0 before events); match.md §4, §13, §14 (clock fields, 0xE7 sign and rate, hash key syntax: proven live); results.md §8 (the client opens the results window itself and waits for 0x97); transport.md §8 (QoS on by default for the lobby; Wine needs a qwave stub); static_info.md §8 Q7 (slots 7/8 live) | bring the tags up to date | live / oracle |
| 14 | keep as is | 508-byte datagrams; the 0xF6 "upload the horizon item" request; the lobby; matchmaking | ours is the better-founded or the only one | static / design |

Not settled by either side: 0xF1/0xF2 against 0xEF/0xF0 for late inputs; 0xDE/0xF3; the uncompressed 0xE8; multi datagrams from the server; anything on Windows; 0x97 filling the results window; the stock Play button and queue.
