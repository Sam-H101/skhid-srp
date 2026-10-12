# Lockstep matches: sync, desync and resync

> **Version scope: client 0.21d only.** 0.21d matches are lockstep (every
> client simulates; the server relays inputs). 0.100b matches are
> server-authoritative and work differently (SRP-0100B.md). All addresses are
> 0.21d's.

How a 0.21d match works, why clients fall out of step, and everything done
about it up to 2026-10-11. The byte-level detail is in
`srp-021d/docs/notes/match.md` (§2 messages, §3 snapshots, §6
verified client facts) and `clock_sync.md`. The case-by-case history is in
`srp-021d/WORK.md`.

## The model

- **Every client runs the whole simulation.** The server simulates nothing. It
  relays inputs, keeps the match clock, collects each client's world hash, and
  decides who is out of step.
- So every client must apply **exactly the same inputs at the same ticks**, and
  start from the same data. Every player's profile, including skill modifiers,
  goes out once in 0x51, the same bytes to everyone; the roster is fixed for
  the match.
- A tick is 50 ms. The world hash is a CRC-32 of the serialized world.
- **The client never makes a world for another client.** The patched client
  uploads its own (option B, on request 0x59). That is the only source of
  snapshots for late joiners and resyncs. The initial world is the first one.

## Messages

Client → server: 0x40 connect {session, debug}, 0x41 per port, 0x42 ready
(loaded), 0x43 time reply, 0x44 clock report, **0x45 input** {f32 dx, f32 dy,
u32 actions, u32 time}, **0x46 hash** {time, CRC}, plus the snapshot upload.

Server → client: 0x50 ports, **0x51 static info** (match options + all
profiles + local index), **0x52 set time** (then the client clears its
world), **0x53 snapshot**, **0x54 relay** {player, input, time}, **0x55 clock
offset** (i16 ms), 0x56/0x57 player entered/left, **0x58 discard** (drop your
own input at tick T), 0x59 upload request.

Transport: 12-byte header, 64 ack bits, datagrams up to 508 bytes (0xFF
multipart for bigger), resend after no ack, 10 s silence = timed out.

## Timing

| Constant (`game.rs`) | Value | Meaning |
|---|---|---|
| `HORIZON_MS` | 250 | An input is relayed only if it reaches the server within 250 ms of its tick. Later ones are dropped and the sender gets a 0x58 |
| `CLIENT_HORIZON_MS` | 500 | The client refuses a 0x54/0x58 for a tick more than ~500 ms behind its clock (`max_ping_time_in_ms` 250 + 250). Such a message is lost, and that client's world goes wrong |
| `SYNC_LEAD_MS` | 350 | A client started from a snapshot at S gets clock S + 350 (0x52) |
| `RESYNC_FRESH_MS` / `RESYNC_FRESH_WAIT` | 700 / 3 s | A resync or a newcomer starts only from a snapshot ≤ 700 ms old; after 3 s waiting, ≤ `FRESH_SNAPSHOT_MS` (1500) |
| `RESYNC_REQUEST_GAP` | 1 s | Asks for a fresh upload this often while one is awaited |
| `DIVERGENCE_CONFIRM_MS` | 1000 | Hash disagreement must persist 1 s before a resync |
| `RESYNC_INTERVAL` | 15 s | At most one resync of a client per 15 s |
| `LOADING_TIMEOUT` | 60 s | A client that connected but never sent 0x42 is dropped |
| Clock correction | 0x55 at 5 % speed | ~5 s to correct 250 ms |

A snapshot is ~560-660 ms old when sent, so a resynced client starts about
210-310 ms behind the match clock. Until 0x55 corrects that (~5 s), most of
its inputs arrive too late and are dropped with a 0x58: **every resync costs
the player about 5 s of control**.

## Verified client behaviour (disassembly, `match.md` §6)

- A resync (0x52 + 0x53) does **not** clear the per-player input lists. A
  client keeps every input it made, its own included. An own input the server
  never relayed stays in its world, and in nobody else's, until a 0x58 removes
  it.
- `discard_input` (0x1CB54F) refuses T ≤ the snapshot time. It removes only a
  node whose time is exactly T, and only if the world history has an item
  older than T (0x1CEE73). The history is a fixed ring of about 500 ms
  (0x1CA765 recycles the oldest item). So a 0x58 for a tick older than the
  client's history is refused, and that input stays for good.
- A local `change_input` (0x1CB3AD) merges into the player's first own node
  later than the current grid tick and **moves it to the new input's tick**.
  After a resync sets the clock back, the first mouse move rewrites an input
  that was already relayed.
- The sender (0x1A34ED) sends a node only when a history item has exactly its
  time and the changed bit.
- Each player's input list and the event history are rings of 27 entries
  (allocators `<0x18, 0x1B>` and `<0x168, 0x1B>`; pop when full at 0x1CF267
  and 0x1CA856). With mouse movement that is about 1.35 s of events. Older
  inputs are pushed out silently.
- After 0x52 the world history is empty (`clear` 0x1CA788), and
  `change_input` then skips the too-new test (0x1CB2DF): every relayed input
  past the snapshot of the old world is taken. Inputs and events outlive the
  snapshot load (0x1CAB3B), and the first move after it plays every event
  after the snapshot in one call (0x1CD1E7). While the history is empty the
  client simulates nothing, records none of the player's own input, and
  refuses every 0x58. Its reply to 0x52 is 0x43, sent before the clear.
  There is no snapshot ack: srp's "0x53 ack" is the transport ack of the
  last part.
- The client never compares hashes; it only reports them (0x46).
- A snapshot holds only active players (the mask). A player who left keeps
  his old object on the other clients, while a rejoiner who loads a snapshot
  builds a fresh one.
- `max_ping_time_in_ms` is a client option registered at 0x68E125 (250, max
  1000). It sets the client's horizon. Untested whether a player can set it.

## Root causes found (all proven from logs plus disassembly)

| Cause | Fixed |
|---|---|
| Resync only after 1 s of persistent hash disagreement (it used to fire on transient flags) | 11:23 |
| Inputs sent while a client loaded the resync snapshot were dropped without a 0x58, so the client kept them | 15:59 |
| After the clock set-back, the client's next local input merged into an own input already relayed. The upload is now asked for only at clock ≥ its last relayed tick + 350, with a blanket 0x58 for own unrelayed ticks before 0x52 | 16:38 |
| A rejoiner got a fresh player object while others kept the old one, so it was wrong from its entry tick. Now 0x56 goes to the others first and the rejoiner starts from a snapshot taken after its entry | 15:59 |
| An own input made before the resync arrived after `finish_sync`, which runs on the 0x53 ack, not on the load. It was relayed at a tick past the new clock, and the client pulled it back (fix 1: `Resynced::made_before`, log "was made before its resync") | 20:03 |
| The 0x58s for the first ticks after the snapshot went to the old world, whose history no longer reached back. Now split at the new clock: later ticks before 0x52, earlier ones after the load and again at `finish_sync` (fix 2) | 20:03 |
| An own tick dropped before the resync blocked a new input there as a "copy". `inputs_seen` now forgets unrelayed ticks after the snapshot (fix 3) | 20:03 |
| With 2 players and no majority, the likely right client was resynced. Now a client that got a 0x54/0x58 past its horizon at or before the disagreement is blamed (fix 4, log "they are out of step") | 20:03 |
| Clients that loaded late started from the initial world, up to 1043 ms behind, and lost 10-20 s of inputs. Newcomers now wait for a fresh snapshot, and no snapshot whose tick is not final is used | 20:15 |
| A match everyone left ended only at its time limit | 16:53 |
| The inputs since the snapshot were replayed only after the 0x53 was acked, a round trip after the client's clock started, so on a slow link the first ticks after the snapshot were past its horizon (0x123 20:54-20:58: 6 of 7 resyncs with an ack ≥ 200 ms failed within 1 s). They now go between the 0x52 and the 0x53, and new inputs are relayed to a loading client at once. See `srp-021d/docs/notes/upstream_recovery.md` | 2026-10-12 04:35 |

All times are 2026-10-11 UTC.

## What is still not explained

From the 111 resyncs in 0x125-0x132 (before the 20:03 fixes):
- 47 were late 0x54/0x58 reaching the client past its horizon. These are spiky
  links (player A 38). The fixes can't stop these. They only resync the right
  client sooner.
- 48 had no logged cause, 21 of them two players flagged together.

Still open:
- A player entering for the first time well after the start sometimes differs
  only in his own player's animation state (0x131 player D; 0x124 player E's
  resync from his entry tick at 20:25). Unproven.
- Ideas not built: make `SYNC_LEAD_MS` depend on the round trip (shorter lag
  after a resync); test `-max_ping_time_in_ms` for spiky players.

## Evidence tools

- Snapshot dumps on the server: `SRP_SNAPSHOT_DIR` (`.../_data/snapshots`).
  Names carry `_r<run>`; only those are unique across restarts. Kinds:
  diagnostic (the flagged client's world **before** its resync), checked,
  after_resync.
- `tools/re-kit/log-analysis/snapdiff.py`: splits a snapshot (mask, players,
  bullets, stats, objects, rules) and diffs two. A player's state starts with
  pools, then 23 bytes of movement whose first floats are pitch, yaw and the
  last turn input. A whole number of 0.1° input steps of difference means a
  lost or extra aim input, not float drift.
- Per-resync log lines: the input trail around the divergence (R relayed, D
  too late, U sent while loading, B made before its resync, C changed copy),
  the in-flight tracking ("reached it after its 500 ms horizon"), and the
  summary each player gets when it leaves.
