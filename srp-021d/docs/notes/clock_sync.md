# Clock sync, input timing and finality (0.21d)

Scope: how the 0.21d client keeps its game clock, when it sends an input for tick T, what 0x52/0x43/0x44/0x55 mean, and what the original server's finality rule must have been. VAs use image base 0x10000 and come from `v021d_disasm/survarium_disasm.txt`.
- [V] means read in the disassembly (address given). [I] means inferred.
- Float constants were read from the exe image: 0x876F40 = 1.0, 0x8806EC = 0.05.

**Short answers**
1. **0x55 is a relative correction.** It is spread over time: a delta in ms, applied at ±5 % of real time.
   - A new 0x55 replaces any correction still running. It does not add to it.
   - The client does not clamp it, apart from the i16 range.
2. **The original server code is not in this binary.**
   - Nothing in the binary builds 0x55 or reads 0x43/0x44.
   - Only the simulation core has a server mode (C80D). The time-sync policy has to be designed. A concrete one is given in §5.
3. **An input for tick T leaves the client in the first frame whose game time is at least T** (T + 0..1 frame), and goes on the wire at once.
   - There is no built-in input delay and no "now + x" scheduling.
   - A 255 ms arrival on localhost is not normal client behaviour. The likeliest cause is that the first copy was lost or dropped and the server used the client's 200 ms resend.
4. **The original server rejected late inputs; it did not restamp them.**
   - In server mode, `change_input` rejects an input with `T <= C7F8` (code 2, "too old").
   - Only `enter_match` and `leave_match` are restamped to the next tick ("request CORRECTED").
   - srp must keep discard + 0x58. Restamping a player's input desyncs that player.
5. **For srp:**
   - Keep `T + 250 <= clock_at_receive` → 0x58.
   - Keep 0x52 = clock now. RTT/2 is optional and does not matter on localhost.
   - Add a 0x55 controller driven by 0x43 and 0x44 (§5).
   - Log the arrival lateness and whether the accepted copy was a resend (§3.4).

---

## 1. `floating_timer` and 0x55

### 1.1 Layout (ctor 0x150D34, `get_elapsed_ticks_impl` 0x150E87) [V]
| off | type | meaning |
|---|---|---|
| +0x00 | u64 | accumulated game ticks at the last checkpoint (`base`) |
| +0x08 | u64 | QPC at the last checkpoint (`last`) |
| +0x10 | u64 | QPC deadline: the end of the running correction |
| +0x18 | f32 | main rate, 1.0 (0 while paused) |
| +0x1C | f32 | main rate saved by `pause` |
| +0x20 | f32 | correction rate, ±0.05 |
| +0x24 | f32 | correction rate saved by `pause` |
| +0x28 | f32 | correction speed, 0.05 (`game` ctor 0x15149C–0x1514CC, from 0x8806EC) |

`elapsed = base + (now − last)·rate18 + (min(deadline, now) − last)·rate20` (0x150E95–0x150EE3). `get_elapsed_msec` = elapsed·1000/QPF (0x150EF3). The game's timer is `game+0x10`.

### 1.2 `set_offset(i16 ms)` 0x150D70 [V]
1. It checkpoints by calling `set_time_factor_impl` with the current rates: `base` = elapsed now, `last` = now, and the deadline is kept only if it lies in the future (0x224A53–0x224B52).
2. `rate20 = 0.05 · sign(ms)`, written to +0x24 instead while paused (0x150D8F–0x150DC7).
3. `deadline = last + ms / (rate20·1000) · QPF` (0x150DE4–0x150E40). The extra time accumulated by the deadline is exactly `ms`.

What this means:
- **Units: ms of game time. The value is a delta, not an absolute time.**
  - **Positive moves the client clock forward:** it runs at 1.05× until the correction is done.
  - Negative makes it run at 0.95×.
- **Duration = 20·|ms|.** So −5 ms takes 100 ms and 250 ms takes 5 s. The maximum, 32767 ms, takes about 11 min.
- **A new 0x55 replaces the old one.** The deadline is recomputed from now, so the part of the previous correction not yet applied is dropped. Always send the full remaining error, never an increment.
- **No clamp** beyond the i16. The handler at 0x1A160A–0x1A1626 reads the i16 and passes it straight through, only in network state 2.
- `set_time_in_ms` (0x1548D2) checkpoints and then sets `base = M` (0x1548F5–0x15491B). A correction still running continues after a 0x52.
- Game time drives everything:
  - `game::tick` reads the floating ms each frame (0x153783);
  - the scene's `game_world::tick(dt, t)` stamps local inputs with `t` and simulates up to `t`;
  - `network_client::tick(t)` (0x153AE6) runs `try_send_all(t, true)`;
  - `on_idle(t)` comes from `on_waiting_for_render` (0x154835).

  So 0x55 shifts the input stamps, the simulation time and the 0x44 reports alike.

## 2. What the client sends for clock sync

| msg | when | payload | address |
|---|---|---|---|
| 0x43 | inside the 0x52 handler, right after `set_time_in_ms(M)` | `u32 echo` (0x52's first u32), `u32 floating_ms`. This is M + about 0, because it is read right after the set. Flushed at once (`send_queued_packets` 0x1A2D8B) | 0x1A2D22–0x1A2D8B [V] |
| 0x44 | `try_send_all` when `t_q >= 7A14 + 1000` (`t_q = t/50·50`) and the world runs (`2674 != 0`, an event-horizon item exists) | `u32 floating_ms`, read fresh at 0x1A3309 rather than from `t` | 0x1A32BE–0x1A332E [V] |

- Both go out in the same call. `enqueue` sets the dirty flag (0x1A13C8), and dirty forces the flush (0x1A3342–0x1A3355).
- A 0x44 sample is therefore the client clock at the moment of sending, give or take the network thread's pickup.
- 0x52 resets the 0x46 and 0x44 timers (`7A10 = 7A14 = 0`, 0x1A2DC9) [V].

What the server can compute (with `M` = the match_time sent and `S` = server clock):
- **0x43:** `RTT_total = S_rx − echo`. This equals `d_down + dispatch + d_up`, where `dispatch` is the time until the game thread handled 0x52.
  - The client's lag right after the sync is `L = S − C = (S_rx − client_ms) − d_up` (srp sends `echo = M`).
- **0x44:** `S_rx − C = L + d_up` (+ network-thread jitter, which is always positive).
  - Take the minimum over a few samples. Subtract `d_up ≈ RTT_net/2`, where `RTT_net` is the minimum transport ack RTT, or 0 on localhost.

**The original server.** Searching the symbols turned up no server-side network code [V]:
- There is no `process_clock` or time-sync handler, and no code that sends 0x55.
- The only match transports are `network_client` and the journal replay `journaling::match_client` (0x523887ff), which only re-delivers recorded server packets.
- The C80D server mode exists only in `game_world_core`.

So the original 0x55 policy cannot be recovered from this binary. §5 designs one around the client's mechanics.

## 3. When the input for tick T leaves the client

### 3.1 Path [V]
1. **`game_world::tick(dt, t)` 0x15A9EE.** When the local `player_input` changed, it builds `{idx, input, time = t}` from the raw frame time, with no quantization. It calls `change_input(…, remote=false)` (0x15AAC8), then `pvp_match_core::tick(t)`, which simulates up to `t` (0x15AB69).
   - The follow-up `vtbl+0x1C` call is a no-op `ret 4` in `network_client` (0x1C81AC). Nothing is sent here.
2. **Local `change_input` (0x1CB3AD–0x1CB4BE).** A change in the same 50 ms interval is merged into the pending item: `dx += dx`, `dy += dy` (mouse deltas), `actions |= actions`, and the time is set to the latest raw `t`.
3. **`network_client::tick(t)` → `try_send_all(t, true)` → `synchronize(core, C7F8−C7EC, t)` 0x1A33CE.**
   - `snap_inputs(t)` 0x1CCBAE moves the pending local input from raw `r` to `S = 50 + ceil((r−50)/50)·50` (the next tick at or after `r`), **but only once `t >= S`** (0x1CCC18). Before that the input waits.
   - `synchronize` then walks the event history and sends 0x45 `{input, S}` (`packet_sender::on_player_input_changed` 0x1A2A34) for every changed and not yet sent item with `time % 50 == 0` and `time <= t` (0x1A3446–0x1A345E, 0x1A3540).
4. **Flush.** `enqueue` sets dirty, so `send_queued_packets` runs in the same `try_send_all` call (0x1A3342–0x1A3355). The 100 ms rule applies only to an idle flush with nothing queued.
   - The flush only posts a `send_queued_order` to the network thread (0x239A5A).
5. `try_send_all` 0x1A3135 also computes `t/50·50`. That value only serves the 0x46/0x44 timers (7A10/7A14), not input times.

**Result:**
- The input for tick T is created by a frame in (T−50, T].
- It is sent by the first frame with game time ≥ T, at client time T + 0..1 frame, plus the network thread's pickup.
- Inputs are stamped at the current tick (rounded up to it), not at a future one. There is no built-in delay. The input is applied locally at once at its raw time, then moved to S with a rollback [I from the V path].

### 3.2 Where the client clock stands after 0x52 [V + I]
`set_time_in_ms(M)` makes the clock read M at the moment the game thread handles 0x52 (§1.2).
- The client is therefore **behind** the server by `L = d_down + dispatch`: the game-thread delay until the dispatcher runs; one frame, or longer during a hitch [I].
- Without 0x55, L stays fixed for the whole match. On localhost both clocks use QPC, so there is no drift.
- An input therefore reaches the server at server time `T + L + frame + d_up`. On localhost that is normally about 5–40 ms.

### 3.3 The client's own simulation time [V]
- `game_world::tick` simulates to `t`, the current game time.
- `on_idle(t)` 0x1A3360 is only a catch-up: when `C7F8 + C7EC < t`, it ticks to `grid(t − C7EC)`.
- C7F4 = the last simulated tick, C7F8 = the newest simulated time (`max`, 0x1CC0BE–0x1CC0DA), C7FC = the rollback start, C804 = the last snapshot.

### 3.4 Why some inputs arrived at T + 255..257 on localhost
A constant clock offset does not fit:
- L would have to be about 240 ms, and then a third or more of player 1's inputs would cross 250, not about 4 in 10 minutes.
- srp already logs `|drift| > 200` from 0x44. Check whether player 1 shows a steady −24x.

The client's resend timing fits the numbers exactly [V timings + I]:
- 0x45 is reliable with `{a=200, b=0}`, so it is resent exactly 200 ms after sending (transport_spec §3.1).
- The resend leaves only on a later flush. In play, flushes come from the input and hash sends.
  - Both are tied to the 50 ms grid: inputs are sent when `t` first reaches a tick, and hash sends store `7A10 = t/50·50`.
  - Each flush happens at a tick time plus a small frame offset.
- The original went out at `T + f`, so the deadline is `T + 200 + f`.
  - If the frame that crosses tick `T+200` comes a little earlier after that tick than the original frame did, the deadline has not yet passed. The resend then goes out with the next tick's flush at about **T + 250 + f′ ≈ T + 255**.
  - Otherwise it goes out at about T + 205.
- srp dedups by `(player, time)`. So a discarded input whose first sighting came at T+255 means **the first copy never reached `Game::input`**.

Possible reasons, all [I], to check in this order:
1. **UDP loss on localhost while srp's single-threaded loop stalls.** The Windows socket receive buffer can overflow, for example during the 30 s snapshot refresh (`REFRESH`): 0x59 → about 30 × 0x47 chunks per patched client, CRC checks, and the synchronous `std::fs::write` dumps in `dump()`, plus 0x53 bursts. Correlate the late inputs with "asked players … for a snapshot" and with the dump file times.
2. **A transport parsing edge case.** For example a datagram that srp treats as low-level (`first && tail.is_empty()` with multi=1).
3. **A client hitch of at least 200 ms between the snap and the send.** This gives lateness of 200–250 + f. It explains individual cases but not the tight 255–257 cluster.

To tell these apart, log for each 0x45:
- the lateness `clock − T`;
- the datagram seq;
- whether `received_bits` had a gap just before it;
- whether the same `(player, T)` arrives again 200 ms later. If it does, the first copy got through and this one is a resend.

Note: the original 250 ms finality has the same behaviour [I]. Any input whose first copy is lost needs the 200 ms resend and is often discarded. That was accepted by design: the input is discarded with 0x58 and the sender rolls back, with no desync.

## 4. The original server's finality rule

**[V]:**
- **Horizon.** `pvp_match_core` passes the event horizon 250 on the server and `max_ping_time_in_ms` (0x87770C) + 250 = 500 on the client (0x1DC92D–0x1DC946). The server keeps one history item, the client `(max_ping+250)/50 + 1` (0x1DC8C3–0x1DC8EF).
- **`change_input` 0x1CB12C, the age test.** `h = C80D ? 0 : C7EC` (0x1CB203–0x1CB210). The input is rejected with code 2, "too old", if `T + h <= C7F8` (0x1CB216–0x1CB220).
  - So the server rejects any input at or before its own newest simulated tick.
  - This path has **no** server-only restamp. The server-only branches are in `enter_match` (0x1CBAB0–0x1CBACF) and `leave_match` (0x1CBD97–0x1CBDB6): `T := C7F0 + ((C7F8 − C7F0)/50 + 1)·50`, with the log line "enter/leave match (client %s) request CORRECTED from … to …".
  - Inputs keep the "change input … REJECTED, reason: too old" string.
- **The server's state.** `event_horizon_history_item` and `is_there_event_horizon_item` return the single item in server mode (0x1CAB13, 0x1DCBF9). The server never rolls back.

**[I]:**
- With one history item and no rollback, the server could only simulate ticks that are already final.
- The generic driver (`on_idle` pattern: tick to `grid(t − C7EC)`) gives the server's world time `C7F8 = grid(server_clock − 250)`.
- Then "reject if `T <= C7F8`" is exactly **"final once `T + 250 <= server_clock`"**. srp's rule is the same thing written differently.
- The clients' clocks were meant to equal the server clock. The horizon offset lived in the server's simulation time, not in the clients' clocks.
- (A design where the server clock itself runs 250 behind is mathematically the same; the client code cannot tell the two apart.)

**Should srp restamp late inputs onto the next open tick and relay them? No.**
- The original did not do it for inputs [V above].
- The client cannot move its own input [V]:
  - Its predicted input at T stays until a 0x58 for T removes it (`discard_input` 0x1CB54F).
  - A 0x54 echo carrying its own index at T′ goes through `change_input(remote=true)` (0x1A2F8F–0x1A2F9B). That path has no "is local" filter. It inserts a second item at T′.
  - If the client already has its own different input at T′, it returns 4 and ignores the echo (0x1CB46F–0x1CB48C). The others then have T′ = the late input, the sender has its own input there: a desync.
  - `dx`/`dy` are deltas that get added together (0x1CB49A–0x1CB4BB), so a duplicate turns the view twice.
- With 0x58 + 0x54-echo-at-T′, restamping could be made to work in simple cases [I]. But it has no precedent in the original, it fails on merges, and it delays the action for every peer. The gain is only for inputs that would otherwise be lost.
- **Keep: discard + 0x58** (current srp). The sender drops its input and rolls back, and everyone agrees.

## 5. Recommendation for srp

1. **Finality.** Keep `T + 250 <= clock(now_at_recv) → 0x58 {T}`, otherwise relay.
   - srp already stamps `now` at `recv_from`, which is right.
   - Do not restamp inputs.
   - Raising the horizon to about 300 would absorb most single resends (T+255+d).
     - It is safe only while `H_server + d_down_max < 500 + L_remote`. A relay that reaches a client after its 500 ms horizon is a permanent desync, which is worse than a discard.
     - If used at all, make it a config value with 250 as the default.
2. **0x52.** `match_time = clock(now)`, as now. `echo` may stay = clock; it comes back in 0x43.
   - Adding RTT/2 only offsets `d_down`. It cannot cover the game-thread dispatch delay, which is usually the larger part.
   - It is harmless but optional. Use the minimum transport RTT/2 if you add it, never the 0x43 RTT (that one contains the dispatch delay).
   - Send 0x52 once per sync only, because every 0x52 wipes the client's history.
3. **0x55 controller** [I, designed to fit §1.2]:
   - For each client keep `d_up ≈ min_transport_RTT/2` (≈0 on localhost).
   - Lag samples:
     - from 0x43: `L = (S_rx − client_ms) − d_up`;
     - from every 0x44: `L = (S_rx − C) − d_up`.
     - Filter with the **minimum of the last 3–5 samples**; jitter only makes a sample larger.
   - Target: `L = 0` (client clock = server clock).
   - Act only when no correction is running:
     - `now >= last_0x55_sent + 20·|last_value| + RTT + 1 s`;
     - and at least 2 samples have arrived since.
     - Then, if `|L| >= 10 ms`, send 0x55 `i16 = clamp(round(L), −32767, 32767)`. Positive is right when the client is behind, because it moves the client forward.
   - 0x55 is ordered on channel 0, the same order counter as 0x51/0x52/0x53/0x59 (transport_spec §3.1). Use the normal ordered send.
   - Because 0x55 replaces, a value sent while one is still running must be the full error measured now, never an increment.
   - Log every decision `(L samples, value)`. This also answers §3.4 for player 1.
4. **Diagnostics** (§3.4):
   - log the lateness histogram per player;
   - flag discarded inputs that were never seen before and arrived more than 200 ms late as "probable lost first copy";
   - correlate them with snapshot refresh and dumps. Moving `dump()` off the receive loop, or behind a flag, removes one likely stall.
