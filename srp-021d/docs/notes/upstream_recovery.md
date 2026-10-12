# Upstream's 0.21d recovery compared with ours: what we take

**Version scope:** client **0.21d** (build 2010). Written 2026-10-12.

**What was compared:**
- Upstream: `origin/feature/v021d-local-stack` of `E:\sur-res-pro\srp` (head
  `ac9304113`), `servers/match-server-021d/src/` (coordinator.rs,
  recovery.rs, replay.rs, clock.rs, wire.rs), `client/021d/` and
  `docs/v021d/*.txt`.
- Ours: `servers/match-server/src/game.rs`, which was byte-identical to the
  live server on Skhid on 2026-10-12. The live logs since the 20:47 deploy on
  2026-10-11: 24 matches, 18 resyncs.
- The original 0.21d exe, with its PDB (dumpbin listing in
  `tools/re-kit/v021d/disasm`).

The working notes, with every citation, are in the session scratchpad
(`upstream021d_recovery.md`, `client021d_restore.md`, `ours021d_resync.md`).
The client facts that matter are summarised in `MATCH-SYNC-021D.md`, "Verified
client behaviour".

**Upstream's restore needs none of their client changes.** It was first run
live on the community-patched exe with no DLL. Their `patch_exe.py` only
loads the DLL, and none of the DLL's hooks touches 0x52, 0x53 or 0x54 handling,
the input lists, the history or the clock. What does not carry over to us:
- **Their upload format** (SRP1 on 0x48).
- **Their hash pacing.** The DLL can hash every 50 ms; stock clients hash
  every 100 ms.
- **Their test conditions.** All their runs used Wine on localhost, with
  packet loss simulated by the server dropping its own packets.

## Verdict per point

| Upstream | Ours | Take it? | Why |
|---|---|---|---|
| Inputs since the snapshot go before it (after 0x52) | Replayed after the 0x53 was acked | **Yes, deployed 2026-10-12 04:35 UTC** | See "The change" |
| Clock after a restore = the current match time | Snapshot + 350 ms | **No** | See "Not taken" |
| The lowest slot is the reference, never judged wrong | Per-tick majority, then fallbacks for no majority (a missed 0x54/0x58, late-input share, the last to join) | No | It needs same-tick hashes from both sides. Stock clients hash every 100 ms on their own grids, so few ticks are shared. A reference that is itself wrong makes everyone else resync |
| Late inputs relayed up to 10 s old, no 0x58 | Relayed only within 250 ms, otherwise a 0x58 to the sender | No | The others refuse an input more than 500 ms old (`change_input` 0x1CB203), so a late relay becomes a divergence and a restore. Our 0x58 keeps the sender in step instead |
| After 3 failed restores, disconnect | No limit, 15 s apart | Not without your decision | It would have removed player F (Poland, 0x123, a collapsing link: 13 resyncs in 3 min). It is a policy choice, not a fix. Their limit was never run live |
| Clock offset from reports, less half the minimum ACK round trip over 5 s; median of 3; ±100 ms steps | Round trip from the 0x52 echo (0x43) only; the whole lag in one 0x55 | Not yet | See "Open" |
| The donor's own hash vouches for its snapshot | Same, and a majority that contradicts it rejects it | No change | Ours is at least as strict |
| Rooms through a custom client menu | Automatic queue | No | Not a sync question |

## The change: inputs between the 0x52 and the 0x53

**Before:** `send_sync` sent 0x52 and 0x53 together. `finish_sync` replayed
every input since the snapshot as 0x54 once the transport had acked every
part of the 0x53. The client's clock starts at S + 350 when it handles the
0x52, so the replay arrived about a round trip later. The client refuses a
relayed input once its tick is 500 ms behind its clock. So when the replay
came more than about 200 ms after the 0x52, the first ticks after the
snapshot were refused.

**Live evidence:** the only player with acks of 200 ms or more since 20:15
on 2026-10-11 was p1 player F (Poland, 0x123). Of his resyncs with
an ack of 200 ms or more, 6 of 7 were wrong again within 1 s; with a shorter
ack, 1 of 10 were. This is a correlation, because the replays were not tracked.

**Now:**
- **`send_sync`** sends, in this order:
  1. the 0x58s for the old world, as before;
  2. 0x52;
  3. a 0x54 for every input since the snapshot;
  4. 0x53;
  5. the after-load 0x58s.
- **New inputs** are relayed to a client that is loading its snapshot as
  they come (`Stage::Syncing`).
- **`finish_sync`** no longer replays inputs. It still sends the roster
  changes, the after-load 0x58s again, and the 0x58s for inputs the client
  sent while loading.

**Why it works on the stock client (proven in the disassembly):**
- 0x52 (0x1A2CF0) sets the clock, sends 0x43, then calls `clear()`
  (0x1CA788). The world history is then empty, C804 = 0 and C7F8 = 50.
- `change_input` (0x1CB12C) then takes any T > 0. Its too-old test compares
  against C7F8 = 50, and the too-new test is skipped while the history is
  empty (0x1CB2DF). The node and its event are stored.
- The snapshot load (0x1CAB3B) clears the world history again, but not the
  input lists or the event history. The first move after it plays every
  event after S in one call (0x1CD1E7).
- The transport sends messages in the order they were queued (one FIFO), so
  the 0x54s land after the 0x52 and before the 0x53.

**Why it is never worse than before:**
- **No loss:** the replay is in before the load, whatever the round trip.
  Before, it had to arrive within about 200 ms of the 0x52.
- **0x52 datagram lost, a later one arrives first:** those 0x54s land in the
  old world. They are kept if their tick is still open there (T > the old
  clock − 500, about S + 100 with snapshots 560-690 ms old), and they outlive
  the clear. Before, the 0x53 waited for the 0x52 resend, and the replay came
  after both, later still.
- **A replay datagram lost:** it is resent after the load, as before, but
  sooner than the old replay would have arrived.
- **The after-load 0x58s** still go right behind the 0x53, so their window is
  unchanged. Upstream waits for the 0x43 before sending inputs. We do not,
  because that would hold the 0x53 back a round trip and push those 0x58s
  past the window they need: the S item stays in the client's history only
  until its clock passes S + 550.
- **Duplicates:** the client already has its own relayed inputs and those it
  had before the resync. Their replays are duplicates (code 1) and change
  nothing, as before.
- **Catch-up limit:** the catch-up spans S to S + 350 plus the transfer. That
  is far inside the client's 27-event window, as before.

**Tests:**
- `a_late_joiner_gets_the_inputs_since_the_snapshot` now expects the order
  0x52, 0x54, 0x53.
- `a_resync_starts_the_client_past_its_own_inputs_the_others_have` expects
  the replay between the 0x52 and the 0x53.
- New: `a_client_loading_a_snapshot_gets_new_inputs_at_once_and_the_old_ones_only_once`.

match-server: 85 tests pass.

**Still to check live** (deployed 2026-10-12 04:35 UTC):
- The resync logs should show the inputs counted at the start of the resync
  ("... inputs since the snapshot, ahead of it").
- On slow links, resyncs should stop failing within 1 s.

## Not taken: the clock at the current match time

Upstream sets the client's clock to the current match time and lets the
replayed inputs fill the gap. That removes the 5-12 s in which a resynced
client's inputs arrive too late (median 44 dropped per resync since the
20:47 deploy). The client code shows it cannot be made strictly better for us:
- **It loses the after-load 0x58 window.** Own inputs the server never relayed,
  in (S, M − 550], could then only be dropped in the old world before the
  0x52. The old world's history does not reach back to the first ticks after
  a 560-690 ms old snapshot. That failure was seen live (0x128 player S,
  0x12a player A, 2026-10-11) and is what fix 2 solved.
- **It runs into the 27-entry limit.** The catch-up from S to M must fit in
  27 event times, about 1.2-1.3 s. Our fallback snapshots can be 1.5 s old.
- **It makes inputs fatal if late.** Every input in (S, M − 500] must be in
  the client before the load, so one replay datagram lost and resent after
  the 0x53 is a desync.

A smaller step that could be strictly better: start the clock closer than
S + 350 on short round trips (fix 6 in WORK.md). It needs a round trip we
can trust first; see "Open".

## Open

- **The round trip used for the clock correction.** It comes only from the
  0x43 echo of the latest 0x52. In 0x123 (20:54:50, player F) that
  echo took 463 ms against a link of 25-40 ms. The server corrected 451 ms,
  and his inputs then still arrived 270-470 ms late for 14 s. Whether his
  client stalled or his link did is not proven: the 0x53 acks were just as
  slow. Upstream's minimum ACK round trip over 5 s would avoid one bad
  sample, but it can also undershoot on a jittery link and over-correct.
  Needs a log line comparing both before any change.
- The input lists and the event history are 27-entry rings. This corrects
  match.md §6, which said nothing prunes old input nodes.
