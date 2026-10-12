# Survarium v0.100b — aim sway ("breath vibration")

Addresses are VAs in `binaries/win32/survarium.exe`. The match server's port is `Breath` in
`servers/match-server/src/combat/weapon.rs`.

## What it moves

The sway does not turn the camera itself. It adds to the horizontal and vertical **recoil
coefficients** before they become recoil clip times, so it moves the first-person camera, and with
it the bullets (`set_fire_bullet_transform` takes the camera the frame drew):

```
horizontal_recoil_value (0x0BC660) = 0.5 - clamp(h_coeff + (is_aimed ? breath_h : 0), -0.5+eps, 0.5-eps)
vertical_recoil_value   (0x0BC710) = clamp(v_coeff + (is_aimed ? breath_v : 0), -0.5+eps, 0.5-eps) + 0.5
```

`eps` = `survarium::epsilon` (1e-7). `is_aimed` is `weapon_core+0x488`. The back recoil has no sway.
Hip fire has none; the breath state keeps ticking regardless. A shot leaves the camera of the frame
before it, so it carries the sway at the previous frame's time.

## Calculator

`breath_vibration_calculator` = `weapon_core+0x374`, one per weapon. Fields: `+0x10` fsm state,
`+0x1C` last tick time (u32 ms), `+0x20` multiplier `m`, `+0x24` target, `+0x28` vertical output,
`+0x2C` horizontal output, `+0x30` scale (set to 1.0 on every call at 0x5A3A0D), `+0x34` breath (s),
`+0x38` hold flag.

`computed_vertical_recoil_time` (0x5A3CA0) and `computed_horizontal_recoil_time` (0x5A3D60) each call
`weapon_core::update_breath_vibration(hold, T, k)` (0x5A39F0): set the hold flag, set the scale to
1.0, then `tick(T, k)` (0x5935F0). So every evaluation of the recoil clips ticks it **twice** at the
same `T`; the second call has `dt = 0` and can only take a transition.

```
if T < last: return
dt = (T - last) * 0.001f * k ; last = T
fsm.tick()                    // the current state's first transition that holds
state.tick(dt)
target = state.target
m = m > target ? max(m - multiplier_decrease_speed*dt, target)
               : min(multiplier_increase_speed*dt + m, target)
t = T * 0.001f * 2π * k       // f32; player::local_time is the identity
breath_h = sinf(t / horizontal_peroid) * horizontal_amplitude * 1.0 * m   // -> +0x2C
breath_v = sinf(t / vertical_peroid)   * vertical_amplitude   * 1.0 * m   // -> +0x28
if !breath_vibration_enable (0x9BBA03, default 1): breath_h = breath_v = 0
```

Arithmetic is x87 at 24-bit precision: `fild` of the ms count, then each `fmul dword` rounds to
single precision once. `k` is the recoil lexeme's time scale, 1 (assumed, not traced).

- **Clock:** `T` is the frame time the player's animation is ticked to, which is the same value the
  client sends as `client_player_update.time_in_ms`. The phase is absolute (`2π·T/1000/period`),
  not counted from aim start or spawn. No randomness.
- **Hold flag:** `(action_mask & 0x80) && (action_mask & 0x0800_0000)`: AIM with hold-breath
  (left shift), from the input bits, not from `is_aimed`.
- **When:** only while the recoil lexemes are built, i.e. in the stand and crouch logic states
  (`player_logic_{stand,crouch}_state::get_recoil_animation_lexeme`, 0x780710). In sprint and jump
  it freezes; the next tick takes the whole elapsed `dt`.

### States (`initialize_logic` 0x5937E0)

| state | target `m` | tick(dt) (0x0CB400 / 0x0CB480 / 0x0CB520) |
|---|---|---|
| normal (initial) | 1.0 | `breath = min(max, breath + dt)` |
| holding | `breath_holding_multiplier` | `breath = max(breath - dt, 0)` |
| shortbreathing | `shortbreathing_multiplier` | `breath = min(max, (max / shortbreathing_repair_time) * dt + breath)` |

Transitions (`fsm::tick` 0x5794F0, in the order added):
- normal → holding when hold
- holding → normal when not hold; holding → shortbreathing when `breath <= 0` (0x0CB320)
- shortbreathing → normal when `breath == max` (`is_ready_for_transition`, 0x0CB570)

**Reset:** `weapon_core::activate` (0x5A5FC5) → `set_breath_holding_params` (0x5934D0): breath = max,
state normal, `m = target = 1`. The last tick time is kept.

## Parameters

Per character, not per weapon: `gameplay/players/default.player` → `player.character_breath_holding_params`
(`breath_holding_params::load` 0x090C80; a missing key keeps its default). No weapon config and no perk
changes them.

| key | offset | value |
|---|---|---|
| max_breath_holding_time | +0x00 | 6 s |
| shortbreathing_repair_time | +0x04 | 3 s |
| breath_holding_multiplier | +0x08 | 0.1 |
| shortbreathing_multiplier | +0x0C | 3 |
| multiplier_increase_speed | +0x10 | 6 /s |
| multiplier_decrease_speed | +0x14 | 2 /s |
| horizontal_amplitude | +0x18 | 0.02 |
| vertical_amplitude | +0x1C | 0.01 |
| horizontal_peroid (sic) | +0x20 | 12 s |
| vertical_peroid (sic) | +0x24 | 6 s |

## Size

Through the shipped first-person clips (AK-74U and Rem 700, standing or crouched), a horizontal
coefficient of 0.02 turns the camera about 0.747° and a vertical one of 0.01 about 0.43° up. At
`m = 1` that is up to about 27 cm at 18 m; holding the breath (`m = 0.1`) about 2.7 cm; out of breath
(`m = 3`) about 70 cm across and 41 cm up.

## srp

- Ticked in `Arsenal::tick_recoil` (with the recoil, stand/crouch only) at every clip event and
  frame time, and in `Arsenal::select` at the selection time with the frame's input
  (`set_target_and_tick`). Both calls of an evaluation are made.
- Drawn at the frame's render step when the weapon is aimed; the next frame's shots use it
  (`Shot.sway`, logged with each shot).
- Not ported: the console switch `breath_vibration_enable` (taken as on), the third-person poses
  (`Snapshot.recoil` has no sway), and the old weapon's calculator during a weapon switch's
  selection.
