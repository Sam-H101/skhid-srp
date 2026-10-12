# Ping display in the 0.21d client

Source: `v021d_disasm/survarium_disasm.txt` (VAs, image base 0x10000), the stock
`survarium.exe` (md5 7f3a4b5b…, the same file as the disassembly), `hud.swf` and
`lobby_menu.swf` from `resources.db` (decompiled with ffdec, and the raw `hud.swf`
tags parsed). [V] means checked in code or data at the address given. [I] means inferred.

## Short answer

**No path in 0.21d shows a measured ping during a match.** The client never
computes a match round trip for the UI. Nothing that srp's match server does
(acks, 0x52/0x43, 0x44, 0x55, the 0x54 relay timing) reaches any ping text.
The "500 + 25" idea has no code path behind it. The 500 ms event horizon is a
local constant and is never shown as ping. A HUD value of 525 cannot come from
this binary's ping code. Either the client being run is not this build, or the
number on screen is not a ping (see "What to check" below).

## 1. Every ping sink in the client

The only strings are `"ping"` (0x801244), `"root.set_ping"` (0x7FFD6C),
`"ping_postfix"` / `"st_ping_postfix"` (localized as "ms") and the
`max_ping_time_in_ms` console variable. I found them by searching the whole exe
for `ping|latency|rtt`. [V]

| Sink | Value | Source |
|---|---|---|
| Scoreboard row, `game_world_ui::set_player_stats_info` 0x181CD6 | `ping` = **constant 0x42 (66)**: `push 42h` at 0x181D69, then `SetMember("ping")` at 0x181D73 | hard-coded [V] |
| Pre-game roster, `game_world_ui::set_pregame` (around 0x1816FF) | `ping` = **constant 0x42 (66)**: `push 42h` at 0x1816FF, then `SetMember("ping")` at 0x181709 | hard-coded [V] |
| Lobby status panel, `root.set_ping` from `lobby_client::read_ping_server_answer` 0x19E5CA | `(game.frame_time_ms − u32_from_packet) × 0.5` | lobby TCP 0x37 [V] |

The HUD does not even show the hard-coded 66:
- `hud.swf` `survarium.controls.PlayerListRenderer` declares `ui_ping:TextField` but never writes it. It sets name, kills, deaths, rank and score only. [V, ffdec AS]
- No PlaceObject in `hud.swf` names an instance `ui_ping`. The string occurs only once, in the ABC constant pool. [V, raw tag scan]
- No DefineEditText in `hud.swf` has initial text containing "525". The bytes "525" appear nowhere in the uncompressed `hud.swf` as text. [V]

The debug net overlays (`network_client::draw_match_stats` 0x1A3C9A, the movies `network_stats_seconds`, `_packets`, `_ports`, `_orders_channel` and so on, enabled by the bytes 0x1D087B0..B7) have no ping, RTT or latency column. Their header strings are counts, bytes, order ids, "MAX TIME", lost and duplicated. [V, string set used by 0x165000–0x16B000]

`hud.swf` has no ping or latency function (`set_*` list in `HUD.as`). Its only `ExternalInterface` calls are for sounds and chat. [V]

## 2. The lobby formula (the only computed ping)

`network_client::on_lobby_packet_received` 0x1A1930 switches on `id − 0x33`, and id **0x37** goes to `read_ping_server_answer` (call at 0x1A19AB). [V]

```
v      = u32 read from the packet                         0x19E5DD
now    = *(game + 0x36BC)   // game frame time, ms         0x19E5E7
ping   = ftol( float((u32)(now - v)) * 0.5f )             0x19E5ED..0x19E606
         // 0.5f = [0x73EB78] (bytes 00 00 00 3F)
lobby_menu(+0x630 movie).Invoke("root.set_ping", ping)    0x19E638
```

- `game+0x36BC` is set to the frame's elapsed ms in `game::tick` (0x1537D8 and 0x153B17). [V]
- The 0.21d client **never sends** the request (lobby 0x28). No lobby_client sender writes id 0x28. The ids it writes are 0x20, 0x21, 0x23, 0x24, 0x25, 0x27, 0x29, 0x2A and 0x2B (0x19D485..0x19EE2E). [V] srp-021d's lobby server never sends 0x37 (grep). [V] So `set_ping` is never called. [V+I]
- Even if it were called, the text would stay hidden. `StatusPanel` sets `panel_ping_block.visible = false` in `configUI` (StatusPanel.as:367), and nothing sets it back to true (grep). [V, AS]
- It is a lobby widget, not part of the match HUD. [V]

The 0.100b client has the same function, with the same `×0.5` formula, at 0x5C76F0. It is also lobby-only. [V, `scratchpad/survarium_disasm.txt`]

## 3. The candidates in the brief

| Candidate | Result |
|---|---|
| Transport RTT (`pkt+0x40` send time, `acknowledge_packets` 0x28D630) | `acknowledge_packets` only feeds the per-time byte statistics (`udp_match_bytes_count_stats::append` at 0x28D83A, timer at 0x28D82B) and lost-packet detection. No RTT is stored for any UI. [V for the calls; I that no other reader exists, because no ping or RTT sink exists] |
| 0x52/0x43 exchange | The client only echoes the value (`process_set_time_request` 0x1A2CF0, see match.md). The round trip is measurable only on the server. [V, per match.md] |
| 0x55 clock offset | `floating_timer::set_offset` shifts the match clock and is not displayed. [V, per match.md] |
| Event horizon 500 = `max_ping_time_in_ms` + 250 | The cc_u32 at 0x87770C has value 250, min 250 and max 1000 (`dynamic initializer` 0x68E125). The horizon is `cc + 0xFA` (0x1DC93A..0x1DC940, or a constant 0xFA in replay mode, 0x1DC933), passed to the `game_world_core` constructor (0x1DC97E). It is a local console value that no server message sets, and it is never shown. [V] |
| Per-player ping from the server | No match message carries one. The scoreboard field is the literal 66 (§1). [V] |

## 4. Answers

1. **Formula and addresses.** In a match there is none. The only computed ping is in the lobby: `(frame_ms − echoed_u32) × 0.5` at 0x19E5CA, shown with `root.set_ping`. It is unreachable in 0.21d and hidden in the SWF. [V]
2. **Inputs the server controls.** For a match HUD ping, none. In the lobby, the u32 in 0x37, but the client never asks for it. [V]
3. **Server-side fix.** None exists, and none is needed for 0.21d. Do not send lobby 0x37: the value would be `now − v`, and a v that is not an echo wraps. [I] A real in-match ping would need a client patch: write `ui_ping`, or change the `push 42h` at 0x181D69 to a measured value. [I]
4. **Gameplay impact.** None from this display. The input and relay deadline is the event horizon (500 ms, from the local cc `max_ping_time_in_ms` = 250 plus 250). It does not depend on any ping value or server message. [V] Input scheduling depends on the match clock (0x52 / 0x55) and on the server's tick + 250 ms relay window (WORK.md), not on this number. [I]

## What to check

The 525 probably does not come from 0.21d's ping code. [I] Find out:
- **Which binary is running.** The shortcuts include `Survarium 0.21d patched (srp…)`, which runs `survarium_srp.exe`. That file differs from stock in 319 bytes: PE header 0x1A7A, VA 0x1A16B1 (`on_match_packet_received` log path), VA 0x1A27A8 (`on_match_ready`), and a code cave at VA 0x6A1E0B. None of these touch `set_player_stats_info`, `read_ping_server_answer` or the UI. [V offsets; I purpose = snapshot / 0x59 patch]
- **Whether it is the 0.100b client** (`E:\sur-res-pro\Survarium`) on the `srp` branch. That is another binary, and these addresses do not apply to it.
- **Which widget shows "525 ms".** In 0.21d the only text with the "ms" postfix is the hidden lobby ping block. The lobby status panel also shows FPS (`root.set_fps` from `lobby_menu::set_fps_stats` 0x176211, called from `game::tick` 0x153D17). [V]
