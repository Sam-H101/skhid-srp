# Survarium 0.100b breath vibration (aim sway): spec for srp

Addresses are VAs in `survarium_disasm.txt` / `binaries/win32/survarium.exe`. Constants were read from the exe:
`0x9430A4` = 0.001f, `0x89E584` = 6.2831855f (2π), `0xA7B6C4` = 1.0f, `0x8A29A4` = 0.5f, `0x4C1E02C` = `survarium::epsilon` (srp `EPSILON_7`).

## 1. What it outputs

The breath sway does not rotate the camera directly. It adds an offset to the horizontal and vertical **recoil
coefficients**, which `computed_{horizontal,vertical}_recoil_time` then turn into recoil clip times:

```
horizontal_recoil_value (0x0BC660) = 0.5 - clamp(h_coeff + (is_aimed ? breath_h : 0), -0.5+eps, 0.5-eps)
vertical_recoil_value   (0x0BC710) = clamp(v_coeff + (is_aimed ? breath_v : 0), -0.5+eps, 0.5-eps) + 0.5
```

`breath_h` is `weapon_core+0x3A0` (calculator+0x2C) and `breath_v` is `weapon_core+0x39C` (calculator+0x28).
`is_aimed` is the byte at `weapon_core+0x488` (0x0AB310), which is srp's `WeaponState::aimed`. The back recoil is not affected.
The return value is multiplied by the clip length (`computed_*_recoil_time`, 0x5A3CA0 / 0x5A3D60).

`breath_vibration_calculator` is `weapon_core+0x374` (built in the weapon_core ctor at 0x5AA7D7). Its layout:
`+0x10` current fsm state, `+0x14` `base_player*`, `+0x18` `breath_holding_params*`, `+0x1C` last tick time (u32 ms),
`+0x20` current multiplier `m`, `+0x24` target multiplier, `+0x28` vertical output, `+0x2C` horizontal output,
`+0x30` scale (always 1.0), `+0x34` breath (seconds), `+0x38` hold flag.

### `breath_vibration_calculator::tick(u32 T, float k)` (0x5935F0)
```
if T < last: return                              // time going backwards: outputs are left as they were
dt   = (T - last) * 0.001f * k ; last = T
fsm.tick()                                       // transitions, see below
state.tick(dt)                                   // breath bookkeeping
target = state.target
m = (m > target) ? max(m - multiplier_decrease_speed*dt, target)
                 : min(m + multiplier_increase_speed*dt, target)
t = player.local_time(T) * 0.001f * 2π * k       // player::local_time is the identity (0x08E7C0, vtable slot 0x68)
breath_h = sinf(t / horizontal_peroid) * horizontal_amplitude * 1.0 * m     // -> +0x2C
breath_v = sinf(t / vertical_peroid)   * vertical_amplitude   * 1.0 * m     // -> +0x28
if !breath_vibration_enable (byte 0x9BBA03, console cc_bool, default 1): breath_h = breath_v = 0
```
The waveform is two plain sines with no noise or randomness. `period` is in seconds. The vertical frequency is twice the
horizontal one, so the sway traces a figure eight. `t` is stored as an f32, then `divss` by the period and passed to `sinf`.

The scale at calculator+0x30 is set to **1.0** on every call in `weapon_core::update_breath_vibration` (0x5A3A0D,
`movss xmm0,[0xA7B6C4]` -> `[ecx+0x3A4]`). The comment on `RecoilState::clip_times` in srp's `weapon.rs`, which says
"breath sway is off: its multiplier is set to 0", is wrong.

### Breath states (`initialize_logic` 0x5937E0; `breath_state_*` 0x0CB300–0x0CB5B4)
Each state has `breath` (shared float at calculator+0x34), `target` (+0x24) and `params`:

| state | target multiplier | tick(dt) |
|---|---|---|
| normal (initial) | 1.0 | `breath = min(breath + dt, max_breath_holding_time)` |
| holding | `breath_holding_multiplier` | `breath = max(breath - dt, 0)` |
| shortbreathing | `shortbreathing_multiplier` | `breath = min(breath + dt*max/shortbreathing_repair_time, max)` |

Transitions are checked first-added-first in `fsm::tick` (0x5794F0), before the state's tick:
- normal → holding when `hold == true`
- holding → normal when `hold == false`
- holding → shortbreathing when `breath <= 0` (`insufficient_breath`, 0x0CB320)
- shortbreathing → normal (`true_predicate`), allowed only when `is_ready_for_transition` holds, i.e. `breath == max` (0x0CB570).
  The also-registered short → holding transition can never fire first.

The hold button is evaluated in `computed_*_recoil_time` (0x5A3D08…). It reads `player.input().action_mask` and is true when
`(mask & 0x80) && (mask & 0x8000000)`, that is AIM together with hold-breath (LSHIFT while aiming). It uses the **input
bits**, not `is_aimed`.

### Reset
`weapon_core::activate` (0x5A5FC5) calls `set_breath_holding_params(&base_player+0xDC)` (0x5934D0). That sets `breath = max`,
the state to normal, and `m = target = 1.0`. `last` is **not** reset. `deactivate` (0x5A40E9) sets params to null.
The calculator is per weapon_core.

### Parameters
They are per character, not per weapon. `base_player_creation_params` loads `character_breath_holding_params`
(0x5DE0FA, loader `breath_holding_params::load` 0x090C80) from `gameplay/players/default.player`, the only `.player`
file. Its values:

| key | struct offset | value |
|---|---|---|
| max_breath_holding_time | +0x00 | 6 s |
| shortbreathing_repair_time | +0x04 | 3 s |
| breath_holding_multiplier | +0x08 | 0.1 |
| shortbreathing_multiplier | +0x0C | 3 |
| multiplier_increase_speed | +0x10 | 6 /s |
| multiplier_decrease_speed | +0x14 | 2 /s |
| horizontal_amplitude | +0x18 | 0.02 (recoil-coefficient units) |
| vertical_amplitude | +0x1C | 0.01 |
| horizontal_peroid (sic) | +0x20 | 12 s |
| vertical_peroid (sic) | +0x24 | 6 s |

- **Weapons:** neither `ak_74u.options` (dict 13) nor `rem_700.options` (dict 14) has any breath key. The weapon affects
  the sway only through its recoil clips and through `is_aimed`.
- **Perks:** "iron breath" and "steady breath" exist only as UI dictionary entries. No client code writes these
  params (`player_parameters_modifyer::apply` does not touch them).

### Angular size (srp's `PlayerModel` with the shipped clips; probe in `breath_probe_test.rs.txt`)
The values are the same for dict 13 and dict 14, standing or crouched:
- horizontal coefficient +0.02 → **0.747° yaw** (23.5 cm at 18 m)
- vertical coefficient +0.01 → **0.43° pitch** (13.6 cm at 18 m)

By state:
- **Normal (`m = 1`):** sway up to ±0.75° yaw and ±0.43° pitch, so up to about 27 cm at 18 m. This matches the observed 15–35 cm misses.
- **Holding (`m = 0.1`):** about 2.7 cm.
- **Shortbreathing (`m = 3`):** about 70 cm horizontally and 41 cm vertically.

**Moving or stance:** speed has no direct term. The calculator only ticks while the recoil lexemes are evaluated, and
only the stand and crouch logic states build them (`player_logic_{stand,crouch}_state::get_recoil_animation_lexeme`,
0x780710). In jump, sprint and other states the breath state freezes. On the next tick it gets the whole elapsed `dt` at
once, while the phase is absolute.

## 2. Time source, randomness, reproducibility
- **The time source is the frame time.** `T` is the "current time" that `n_ary_tree::computed_animation_time` (0x11C680)
  passes to the lexeme time calculator. Its delegate signature is `(len, offset, start, ?, T, time_scale)`.
- **`T` is the update's time.** `T` is the time that `player::tick(time)` (0x5D62B0) ticks the animation tree to, which is
  the `network_client::tick` time (0x5C6576). That same value is written into the `client_player_update.time_in_ms` of
  the frame (`serialize_current_state` 0x5E23B0 and the vfunc call at 0x5D686B).
- **`k`** is the lexeme's time scale. Recoil lexemes set no special scale, so k = 1. This is assumed, not traced to the
  end.
- **The phase is absolute.** It is `2π·T/1000/period`. It is not measured from aim start or from spawn.
- **No randomness.** No RNG, no `random32` / `normal_random`, and no per-weapon seed is involved.
- **The server can reproduce it from the update stream:** the phase from `time_in_ms`, the multiplier and breath from
  each frame's mask and time. The client sends every frame's update, batched every 33 ms.
- Remaining inexactness:
  - The client ticks at animation sub-steps, and each fsm transition takes effect one tick late. This gives at most one
    frame of ramp error at transitions, about 2 cm.
  - The f32 rounding of `t`.
  - The k = 1 assumption.

## 3. Applicability
- **Bullet direction:** yes. It changes the recoil clip times of the first-person pose. The head and camera of that pose
  feed `set_fire_bullet_transform` (from `render`), which is the same path srp already uses for recoil.
- **Only when aimed:** `is_aimed` (`+0x488`) gates the offsets. Hip fire has no sway, although the breath state keeps
  ticking (normal refills breath; hold needs the AIM bit anyway).
- Shots of frame N use the camera rendered at frame N-1, so they use breath at the previous frame's `T`.
- The third-person recoil lexemes probably carry the same calculators, which only moves hit boxes slightly (low priority).

## 4. What the client sends
Nothing reflects the sway. `client_player_update` holds `player_input` (angular velocity and acceleration, action mask),
`player_state` (position, yaw, look_pitch) and `time_in_ms`. Look_pitch and yaw are the player's mouse orientation,
which includes the player's counter-movement but not the sway. `weapon_core::serialize` (0x5A3810) has no breath fields.
The server has to recompute the sway.

## 5. Recommended port for srp
Track this per weapon, in `WeaponState`:
```rust
struct Breath { last_ms: u32, m: f32, breath: f32, state: Normal|Holding|Short, offsets: (f32, f32) /* (h, v) */ }
// on activate (Arsenal activate_pending / draw): breath = 6.0, state = Normal, m = 1.0 (keep last_ms)
```

Each replayed frame, at `time_ms`, while the stance is stand or crouch and the weapon is active (where `update_recoil`
runs; at least once per frame):
```rust
fn tick(&mut self, t: u32, mask: u32, p: &BreathParams) {
    if t < self.last_ms { return; }
    let dt = (t - self.last_ms) as f32 * 0.001; self.last_ms = t;
    let hold = mask & 0x80 != 0 && mask & 0x0800_0000 != 0;
    self.state = match self.state {                       // fsm::tick before state tick
        Normal if hold => Holding,
        Holding if !hold => Normal,
        Holding if self.breath <= 0.0 => Short,
        Short if self.breath == p.max => Normal,
        s => s,
    };
    let target = match self.state {
        Normal => { self.breath = (self.breath + dt).min(p.max); 1.0 }
        Holding => { self.breath = (self.breath - dt).max(0.0); p.hold_mult }
        Short => { self.breath = (self.breath + dt * p.max / p.repair).min(p.max); p.short_mult }
    };
    self.m = if self.m > target { (self.m - p.dec * dt).max(target) } else { (self.m + p.inc * dt).min(target) };
    let ph = ((t as f64 * 0.001f32 as f64 * std::f32::consts::TAU as f64) as f32);
    self.offsets = ((ph / p.h_period).sin() * p.h_amp * self.m, (ph / p.v_period).sin() * p.v_amp * self.m);
}
```

At the `render` step of `Arsenal` (weapon.rs, `self.drawn_recoil = weapon.recoil.clip_times()`), when `weapon.aimed`,
add `offsets.0` to `horizontal` and `offsets.1` to `vertical` before the existing clamp:
`0.5 - clamp(h + bh)`, `clamp(v + bv) + 0.5`. The `drawn_recoil` of the next frame's shots then carries the sway of the
previous frame's `T`, as on the client.

Optionally, do the same for the `Snapshot.recoil` used in third-person poses.

Load the 10 params from `gameplay/players/default.player` → `player.character_breath_holding_params` with the existing
`Resources::config`. Also fix the misleading comment on `clip_times`.
