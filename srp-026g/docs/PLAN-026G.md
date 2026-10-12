# Work order for the 0.26g server

> **Version scope: client 0.26g** (build 2777, January 2015). The research behind each step is in
> `srp-026g/docs/notes/` (start with its `README.md`). Section
> references like "lobby.md §3" point there.

The repo is `srp-026g` (branch `v0.26g`, a clone of srp-021d). Each step
names the notes it builds on, the code it touches, how it is tested and
when it is done. Steps 1–3 come first and in order; after them, step 4
(data) and step 5 (client patch) can run in parallel with step 3.

## Status (2026-10-12, end of the autonomous run)

Steps 0–8 are coded and pass synthetic tests (735 tests in the workspace,
877 after the step 9 work, 951 after the economy, matchmaking and feedback work of the same day, including an end-to-end run of the real servers with two scripted clients:
`srp-026g/servers/srp/tests/v026g_end_to_end.rs`). **Since 2026-10-12 the
server is live on 026g.skhid.link and real players play matches on it**
(SRP-026G.md). Details per step and
what the live sessions showed are in `srp-026g/WORK.md` (the entries dated
2026-10-12, newest at the top).

| Step | State |
|---|---|
| 0 test bench | done |
| 1 transport | coded (`crates/udp-transport`), tested against a client model |
| 2 login | coded (`servers/login-server/src/v026g.rs`) |
| 3 lobby | coded (`crates/survarium/src/v026g/lobby`, `crates/session/src/v026g`, `servers/lobby-server/src/v026g`) |
| 4 data | coded (`crates/survarium/src/v026g`) |
| 5 client patch | patcher finished and self-tested; full build **installed 2026-10-12** in the local test game folder (old exe kept as `survarium_srp.cert-only.bak.exe`); the upload runs in live matches since 2026-10-12 |
| 6 matches | coded (`match_msgs`, `servers/match-server/src/v026g`) |
| 7 results | coded (`crates/session/src/v026g/rewards.rs`, shown by the lobby) |
| 8 chat, friends, squads | coded; clans refused |
| 9 optional | coded 2026-10-12: clans, item modifications / cosmetics / reroll / craft / repair, premium shop and bundles, daily bonus, tutorial, nickname change, autobuy, admin text; then the economy by the sources of the time, matchmaking by level, silent-client return, chat feedback; 1004 tests pass (2026-10-12, with the update deployed that day); local test with real clients: `srp-026g-local-test.bat` |

`cargo run -p srp` now starts the 0.26g servers (login TCP 1234, lobby UDP
1235, match UDP 1236, web 8088), not the 0.21d ones. The first live session
was on 2026-10-12; its checklist is in
`srp-026g/docs/notes/README.md` ("How to run the 0.26g stack
locally"). What is open now: OPEN-ISSUES.md, "0.26g".

## Ground rules for the port

- **Add, don't overwrite.** New wire code goes in
  `crates/survarium/src/v026g/` next to `v021d/`, and new servers' code in
  new modules. The 0.21d code stays as a reference until 0.26g works, then
  it is deleted from this repo (srp-021d keeps it).
- **Every message gets a byte-exact test** built from the layouts in the
  notes, as srp-021d has for 0.21d.
- **The client checks almost nothing.** Every writer must respect the size
  limits in the notes. The hazard list in lobby.md §3 and results.md §6
  becomes tests.
- Live tests use a local 0.26g client. Nothing touches the live 0.21d
  server until hosting is decided (the owner said "later").
- No commits unless the owner asks; a `WORK.md` entry per change.

## Step 0: a test bench (small)

**Goal:** a 0.26g client that can be pointed at a local srp.

- Copy srp's `certs/survarium_login_server.crt` over the client's
  `game/resources/ssl/survarium_login_server.crt` (keep the original as
  `.orig`). The bundled one has another key and expired in 2015.
- A shortcut:
  `survarium.exe -server=127.0.0.1:1234 -minimum_resources_memory_size=768`
  (the launcher normally passes `-server`; check whether it also passes
  `-account_name` / `-account_password`, login_chat.md §7).
- Packet capture for every live test from here on (srp's own datagram log,
  step 1).
- **Changes the game folder: needs the owner's yes first.**

**Done when:** the client starts and shows its login screen.

**Done 2026-10-12.** What it took:
- The certificate above, installed (original kept as `.crt.orig`).
- **Exe patch: certificate check.** Every stock 0.26g client stops at start
  with "Resources archive is corrupted": the code-signing certificate it
  checks expired on 2015-06-06 (details in `client_patch.md` §0). The data
  is fine (CRCs match the archive). `patch_026g.py ... --cert-only` writes
  `survarium_srp.exe` with only that patch; the shortcut runs it.
- **0.26g option names:** `-server=host:port` (not `-client=`) and
  `-splash_screen=0` (not `-no_splash_screen`). Shortcut:
  `Survarium 0.26g (srp, Ukrainian).lnk`.
- **Window:** 0.26g shares `Documents\survarium\user.cfg` with 0.21d
  (`r_resolution 3440x1440`). It needs the per-user compatibility flag
  `HIGHDPIAWARE` (HKCU `...\AppCompatFlags\Layers`, set for both 0.26g
  exes, as 0.21d's already had), then `survarium-borderless.ps1` (it now
  also finds `survarium_srp`).
- Packet capture: Wireshark is not installed; srp will log raw datagrams
  itself instead (agreed 2026-10-12, built in step 1).

## Step 1: the UDP transport (large, the foundation)

**Why first:** the lobby and matches both run on it, so nothing past login
works without it.

**Notes:** transport.md (all of it; §9 has worked bytes), match.md §1.

**Code:**
- Move the transport out of `servers/match-server/src/transport.rs` into a
  shared crate (e.g. `crates/udp-transport`), used by both the lobby and
  match servers. Rewrite it for 0.26g:
  - 3-byte header `u8 seq, u8 rseq, u8 ack0`, ack tail bytes at the end of
    the datagram, 128-bit ack window as a 7-bit varint (§2);
  - 8-bit sequence arithmetic, half range 128;
  - the per-id table, one per connection kind: fixed length, variable
    length, delivery kind, lifetime and resend time. Matches (transport.md
    §4): only E4, E8, F6 are variable; plain or unique. **Lobby
    (transport.md §4.1): every id is variable length and unique, and most
    server ids are also ordered**;
  - message header `id [u8 x]`, x = unique id or order id by the table;
  - **an ordered channel for the lobby** (its own u8 counter from 0; the
    client holds a message until its predecessor arrives). Matches have
    none;
  - **before coding:** trace the client's multipart ordered receive
    (0x3F4A50: expiry and limits). Inventory 0x89 and account 0x87 need
    it, and it is not in the notes yet;
  - length prefixes only on variable ids (`u8` up to 254, else `FF u16`);
  - multipart `FF id u8unique part [count]`;
  - low-level ids FA initiate, FB confirm, FC keep-alive, FD instant
    disconnect;
  - at most 252 messages per datagram, 508 bytes per datagram;
  - never send a header-only datagram, never echo a sequence the client
    has not sent (the client drops the whole datagram), start the server
    sequence at 0.
- The connect message (0xD8 for matches; the lobby's sign-in 0x2E travels
  the same way) and the port messages (0xD9 per port).
- **Raw datagram log** (agreed 2026-10-12; replaces Wireshark): an opt-in
  setting (e.g. `SRP_PACKET_LOG=<dir>`) that writes every datagram in and
  out, with time, direction, peer and hex bytes, one file per connection,
  plus the decoded messages (id, unique, length). Off by default; never log
  login passwords or digests. Used for every live test from step 2 on.

**Tests:** encode/decode round trips; the worked bytes from transport.md
§9; the ack window across the 255→0 wrap; multipart split and join;
a datagram the client would drop must never be produced.

**Done when:** the tests pass, and in step 3 the client's connect is
acknowledged and it keeps the connection for a minute (keep-alives flow).

## Step 2: login (small)

**Notes:** login_chat.md §4 and §6.

**Code:** `servers/login-server/src/lib.rs`, the `session` crate.
- Accept version `"0.26g0"`.
- Read the new password message: kind `02` + 40-char SHA-1 hex digest
  (remembered account), or `03` + digest + `u8 len` + plain text.
- Check passwords by the digest. Simplest: store argon2id of the SHA-1 hex
  digest. A 0.26g database starts empty, so no migration is needed (only
  if 0.21d accounts are ever carried over).
- Reply `08 u32 session u32 ip u16 port` pointing at the lobby's UDP
  address (check the IP byte order live).
- Renumber the error codes (bans 0x12–0x1F by reason, access level 0x20,
  already signed in 0x21, invalid version 0x22, EULA 0x24, queue 0x25).
- Remove what 0.26g no longer uses: the browser server, the UDP presence
  listener, the sign-out handler. Sessions now end when the lobby link
  drops (step 3).
- `srp` main: stop starting the browser server.

**Tests:** byte-exact login exchanges for both password kinds, each error
code, a wrong password.

**Done when:** the client signs in and then tries to reach the lobby
address (visible in the capture).

## Step 3: the lobby, minimum (large)

**Goal:** a stock 0.26g client reaches the lobby, shows its gear, money and
level, and can change equipment.

**Notes:** lobby.md (§2 layouts, **§3 hazards and push order**),
lobby_requests.md (§1 sign-in, §2 actions, §4 the acks it expects, §6).

**Code:**
- `servers/lobby-server/src/lobby_server/mod.rs`: replace the TCP accept
  and `frame.rs` with the step 1 transport; sign-in is the connect payload
  `2E u32 session bool need_shop_items`.
- `crates/survarium/src/v026g/lobby.rs` (new): the writers, each with its
  new id:
  - 0x80 signed in, 0x85 client state, 0x83/0x84 permitted/denied;
  - **0x87 account**: up to 3 profiles (kits), a shared item table of at
    most 69 items, slots as indexes into it, the 128-bit mask of 68
    modifiers, account-wide level, experience and autobuy;
  - 0x89 inventory, the new item descriptor (item id, dict id, condition
    ×10000, repair/reroll times, up to 8 mods, up to 3 upgrades);
  - 0x8A prices and 0x95 shop as raw LZMA2 (stored, uncompressed chunks);
  - 0x8B money, 0x8C level and skills, 0x8D six service prices, 0x8E
    reputations plus the quest faction byte, 0x8F nickname, 0x94 premium,
    0x96 faction mask, 0x92 quests (empty at first), 0x9C, 0x9D, 0x9F.
- **The push order from lobby.md §3**; inventory 0x89 before account
  0x87.
- The account model in `session`: one account with up to 3 kits instead
  of per-profile levels and skills.
- Because the client never asks for its account or money, push 0x87,
  0x89 and 0x8B after every change.
- Requests (`actions.rs`): `46 00` move item, `48` skills, `49 01/02`
  autobuy and active kit, `47` shop buy/sell. Queries are separate ids
  with a leading `00` byte (34, 37, 39, 3B, 3C, 3D, 3E, 40, 41, 42, 43,
  45). Anything not implemented gets `84 <op>` (denied), never silence if
  the client waits for an answer (lobby_requests.md §4).
- Never send ids outside 0x80..0xAA or 0x81, 0x86, 0xA0, 0xA7 (they crash
  the client or drop the datagram; 0xA7 and 0x86 have no transport entry,
  transport.md §4.1); a test enforces it.
- The push order of lobby.md §3 holds only among the **ordered** ids. The
  unique ids (80, 82, 9A, A1, A2, A4, AA) are not ordered against them.

**Tests:** byte-exact writers; the push order; the hazard rules as tests
(item table ≤ 69, dict ids exist, search step ≠ 0, string limits).

**Done when:** live, the client sits in the lobby for 10 minutes, shows
the right gear and money, and an equipment change survives a re-login.

## Step 4: 0.26g game data (medium, in parallel with step 3)

**Notes:** data.md (§7 lists what to redo), modes_maps.md.

**Code:** `crates/survarium/src/v026g/`, embedded files in
`resources/binary-config-examples/` with a `_026g` suffix.
- `Dictionary::parse` for the 0.26g `db_static_dictionaries`: 384 items,
  categories 25–29, `item_level` in each item's config, no boosters, perks
  or skills lists, and the new sections (bundles, maps, matchmaking
  settings, reward multipliers).
- Prices from `base_cost` instead of the hand-made `prices.rs`.
- Skills from `gameplay/skills/skills_tree.options`: 26 skills, named
  modifiers, values as fractions, prerequisites and exclusive pairs.
  Writes the 68-slot modifier block (names and bit order: static_info.md
  §3.1; mask high qword first). `meele_hit_ultimate` and `meele_hit_range`
  (skills 206, 210) are not among the 68 names.
- Drops: four tables (normal, new player, daily bonus, drop card).
- Quests: 369 quests, regenerate the type numbers.
- Item parameters (weight, wear): regenerate the table; the carry limit
  is 30 kg.
- Map list: maps_dict ids per mode (modes_maps.md §8): mode 2 = 61…105
  odd and 109, mode 3 = 62…106 even and 110, mode 4 = 117–140. Exclude
  test and broken ids (11, 45, 107, 108, 114, 115, 141–151).
- Starting loadouts from 0.26g `test_player_profiles` (one is over 30 kg:
  fix or drop it).

**Tests:** the parsers read the real 0.26g files; spot checks of a few
items, skills and maps against the dumps in `tools/re-kit/v026g/data/`.

**Done when:** the lobby in step 3 shows real 0.26g items with names,
prices and weights, and skills can be spent.

## Step 5: the client patch (medium, high risk; in parallel)

**Notes:** client_patch.md.

**Code:** `scripts/client-patch-026g/patch_026g.py`.
- The mutex patch is ready (`74 09` → `90 90` at 0x124F9).
- Snapshot upload: the hooks at 0x27AEA1 (first sync) and 0x27A505 (the
  0xF6 request). Decide where the ~369 bytes of code go:
  - **a new PE section** (cleaner, keeps the debug option), or
  - over `game_world_core::on_hash_mismatch` 0x325A60 (the draft; loses
    `-debug_hash_mismatch` on the patched client).
- Build the patched exe as a separate file; the original stays untouched.
- **Patching the exe needs the owner's yes.**

**Tests:** `--check` mode on the original exe (passes now); disassemble the
patched sites; a live run once step 6 can request a snapshot.

**Done when:** two patched clients run on one PC, and one of them uploads
its world when the server asks.

## Step 6: matches (large)

**Notes:** match.md, static_info.md, transport.md, modes_maps.md.

**Code:** `servers/match-server/src/{server.rs, game.rs, snapshot.rs}`,
`crates/survarium/src/v026g/match_start.rs`.
- Message ids: client→server 0xD8–0xE1, server→client 0xE3–0xF6.
- **Static info 0xE4**: `u8 local index`, then raw LZMA2 (stored chunks)
  holding the match options, in this order: 20 profiles (team, name, clan
  tag, protection, up to 3 kits with 23 slots and the modifier block, the
  item list), `u8 map_id`, `u8 N`, `u32 match id`, `u16 match level`, 28
  f32 score weights, the 20 squad bytes. Byte layout: static_info.md §6.
- Lobby seat 0x82 gains a u32 game time.
- Times: 1-byte ticks (mod 256, relative to the receiver), plus a 1-byte ms
  for the precise variants. Inputs: tick, two half-floats (pass through
  byte for byte, never re-encode), u32 action mask.
- Relays: single (0xEB), precise (0xEC), multi-player per tick (0xED/0xEE);
  the kit choice ("profile input", 0xDE→0xF3–0xF5).
- Time sync: 0xE5 `{u8 generation, u16 echo, u32 time}`, reply 0xDB; ping
  0xE6. Rework the clock estimate around the client's 16-bit clock.
- Snapshot 0xE8 `{u8 generation, u8 compressed = 0, body}`; the body has 6
  sections (`snapshot.rs` must find 5 after statistics instead of 3).
- **Sync generation:** each resync sends 0xE5(g+1) then 0xE8(g+1); the
  client drops an older 0xE8.
- Hash reports 0xDF carry time / 50 as one byte; keep the majority vote.
- Late inputs: the client now moves them to its own tick instead of
  rejecting them, so the server's horizon rule matters even more. Decide
  between moving (0xEF/0xF0) and discarding (0xF1) (open question).
- Map choice by maps_dict id; modes 2, 3 and 4. Timings come from the
  mode config (900 s, countdown 20 s, 10 batteries, respawn 8 s, tickets
  100 or 150).
- `wait_player_percent` 0.8: size the roster to the players present, or a
  solo test waits.

**Tests:** byte-exact messages; a replay of a recorded 0.26g session once
one exists; the resync generation rules.

**Done when:** two patched clients play a full mode 2 match to the end
with no resync loop, then a mode 4 (Team Deathmatch) match.

## Step 7: results and rewards (medium)

**Notes:** results.md.

**Code:** `crates/survarium/src/v026g/results.rs`, `session` reports.
- 0x97 match stats: new header fields (unused u32, two team values, game
  mode), clan tags, the redesigned full player result (score components,
  bonus blocks, Elo before/after, quest money and reputation, play time,
  the dropped item).
- 0x93 player details now carry the clicked player's account.
- After the screen the client sends `3C 00`: answer with the quests.
- Team Deathmatch winner: more tickets left.
- Rewards: carry the 0.21d rules over, adjusted to the 0.26g reward
  multipliers in the data.
- The sign-in field `pending_match_stats_id` is gone: push unseen results
  after sign-in instead.

**Done when:** after a match each player sees a correct results screen and
the rewards land in the account.

## Step 8: chat and squads (medium)

**Notes:** social.md §6, login_chat.md §5.

**Code:** `servers/lobby-server/src/messaging_server/`.
- Fold chat into the lobby connection: ids 0x4C–0x50 in, 0xA2–0xA4 out;
  no separate socket or chat sign-in.
- Subscription: 10 × u32 + u8 language; the new channel 9.
- Clan tag strings in text (sender type 5), friend lists and important
  messages.
- Friend and ignore lists pushed by the server at sign-in and after each
  change; never the old `u8 result` reply (it crashes the client).
- Squads: 0xA1 squad info (same body, commander index now used), actions
  `4A 01..04`, the new Play gating rules.
- 0x9F online stats: 0 now only shows the maintenance notice.
- 0xAA forced sign-out when the same account signs in elsewhere.

**Done when:** two clients chat, add each other as friends, form a squad
and queue together.

## Step 9: later, optional

- Clans (0x9A, `49 06..23`, channels 3 and 9, clan-team play `30`).
- Crafting, upgrades, rerolls, repair (`46 01..0A`, 0x9B).
- Drops and drop cards (0x98), daily bonus (0x99).
- Premium shop and discounts (0xA8/0xA9), bundles (0x95 with contents).
- Tutorial and video link (0x9D/0x9E), admin text (0xA5/0xA6).

Until then each one gets a deny (`84 <op>`) or an empty push.

## After step 8: going live

- Hosting: the owner decides later (a second server process beside 0.21d,
  or other ports on Skhid).
- The players' download: the patched 0.26g client with the certificate and
  a launcher shortcut.
- The pages at the project root get their 0.26g versions (`SRP-026G.md`,
  `CLIENT-FACTS-026G.md`, `MATCH-SYNC-026G.md`).
