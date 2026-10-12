# DLSS / upscaling for the Survarium 0.21d client: feasibility

Status: research only, nothing implemented. Date: 2026-10-10.

## TL;DR

- **In-process DLSS (any version) cannot work.** `survarium.exe` is a 32-bit PE32 process. The DLSS SDK and Streamline ship only 64-bit (x86_64, arm64, arm64ec) Windows binaries, so `nvngx_dlss.dll` cannot be loaded into our process. OptiScaler, DLSSTweaks and DLSS Enabler all work in-process by replacing an *existing* DLSS/FSR/XeSS integration, and the game has none.
- **"DLSS 5"** (GTC, March 2026) is a neural-rendering pass that game studios integrate; it is not an upscaler you can inject. It needs an RTX 50 GPU and engine inputs. No unofficial library can add it to this game.
- **The engine is better prepared than expected.** It has a deferred G-buffer, depth, a TAA pass with **projection jitter**, and an **object motion-vector buffer**. Real DLSS-SR is possible in principle, but only through a heavy custom bridge to a 64-bit helper process (see Path C).
- **Practical recommendation:** use out-of-process or driver-level tools. NVIDIA Smooth Motion (driver frame generation, DX11, RTX 40/50) or Lossless Scaling (any GPU, borderless window) for frame generation. DLDSR, NIS, Lossless Scaling LS1/FSR1 or Magpie for spatial scaling. These need no binary changes.

## 1. What "DLSS 5" is, and the injector landscape

**DLSS 5** was announced at GTC on 2026-03-16. It is a "3D-guided neural rendering" pass. It takes the frame's colour and motion vectors, plus geometry, and re-renders material and lighting detail (skin, hair, fabric). It has per-asset semantic masks and developer controls ("Structure/Tone Intensity"). It is RTX 50-only and ships in fall/Q4 2026 through studio integrations (NBA 2K27, Starfield, AC Shadows, RE Requiem…). It is not a driver toggle and not injectable. The upscaler that currently ships is **DLSS 4.5 SR** (2nd-gen transformer, Jan 2026, RTX 20+); Dynamic 6x MFG is RTX 50-only.

| Tool | Kind | API | 32-bit game? | Needs game MV/depth/jitter? | GPU | Fits Survarium? |
|---|---|---|---|---|---|---|
| NVIDIA DLSS SR via NGX/Streamline SDK | in-process SDK | DX11/DX12/VK | **No.** x64/arm64 libs only | Yes, all three, plus an exposure/reset flag | RTX 20+ | No (32-bit) |
| DLSS 5 neural rendering | in-process, studio integration | DX12/VK via Streamline | No | Yes, and more (masks) | RTX 50 | No |
| OptiScaler | in-process DLL that swaps an *existing* DLSS2+/FSR2+/XeSS call | DX11 (DLSS native; FSR3/4 via a background DX12 device, about 10% cost), DX12, VK | No 32-bit build documented; its DLSS backend needs the 64-bit NGX DLL anyway | Takes them from the game's existing upscaler call | Any (FSR/XeSS), RTX for DLSS | No: there is no upscaler call to hijack |
| DLSSTweaks | wrapper over an existing DLSS integration (DLAA, ratios, presets) | DX11/DX12 | No | Game must already have DLSS | RTX | No |
| DLSS Enabler | emulates DLSS/DLSS-G on non-RTX GPUs in DLSS-capable DX12 games | DX12 | No | Game must already have DLSS2/3 | Any DX12 | No |
| DXVK (+ dxvk-nvapi) / vkd3d | DX→Vulkan translation | DX9-11→VK (DXVK supports 32-bit) | DXVK yes; DLSS still needs a 64-bit NGX in-process | Translation adds no MV/jitter | Vulkan 1.3 | Translation possible, DLSS still not |
| ReShade | in-process post-process injector (32-bit OK) | DX9-12/VK | Yes | Depth access only, no MV | Any | Only sharpening/CAS/FSR1-like spatial filters; no real upscaler |
| Lossless Scaling (LSFG 3.x, LS1/FSR1/etc.) | out-of-process capture + overlay | captures any window | **Yes** (process-agnostic) | No (optical flow on final image; HUD gets interpolated) | DX11-capable GPU; about GTX 1060 for FG | **Yes** |
| Magpie (open source) | out-of-process window upscaler (FSR1, Anime4K…) | any window | **Yes** | No | Any | Yes (spatial only) |
| NVIDIA Smooth Motion | driver frame generation | DX11, DX12 | Driver-level; not documented for 32-bit, so **test it** | No | RTX 50; RTX 40 since Aug 2025 | Likely yes |
| NVIDIA NIS | driver spatial scaler/sharpen | any | Yes | No | Any GeForce | Yes |
| NVIDIA DLDSR | driver AI *down*-sampling (render above native, then downscale) | any | Yes | No | RTX 20+ | Yes, for image quality, not FPS |

## 2. The 32-bit blocker (verified)

- The NVIDIA DLSS SDK `lib/` ships only `Windows_x86_64`, `Windows_aarch64`, `Windows_arm64ec`, `Linux_x86_64` and `Linux_aarch64`. There is **no Win32 library**.
- Streamline 2.14.1 builds only `amd64` (the default), `arm64` and `arm64ec`.
- Unity's DLSS docs state explicitly that DLSS is not supported for "Windows using x86 architecture (Win32)".
- A 32-bit process cannot load a 64-bit DLL, so in-process DLSS is impossible without a 64-bit recompile, and there is no source to recompile.
- What remains:
  1. **Out-of-process** upscalers or frame generation: Lossless Scaling, Magpie, or the driver features (Smooth Motion, NIS, DLDSR).
  2. **A cross-process bridge** (Path C): a 64-bit helper process that owns its own D3D11 device and NGX on the same adapter. The game shares colour, depth and MV textures with it through DXGI shared handles and keyed mutexes. The helper runs DLSS-SR into a shared output, and the game copies that output back and presents. This works in theory, but nobody ships it, so it would be fully custom.
  3. DX11→VK translation (DXVK) does not change bitness, so DLSS still needs a 64-bit loader. This is a dead end on its own.

## 3. Engine facts from the binary (`binaries\x86\survarium.exe`)

- **No separate render DLL.** The game folder holds only `survarium.exe`/`.pdb`, `bugtrap.dll`, the launcher, the service and the updater. `render_pc_dx11` is statically linked into the exe. Imports are `dxgi.dll`, `d3d11.dll`, `d3dx11_43.dll` and `D3DCOMPILER_43.dll`. There are also `d3d9.dll`/`D3DPERF_*` strings, used for PIX markers.
- **API and feature level:** `vostok::render::device::create` (VA 0x266BDF) calls `D3D11CreateDevice` at 0x266C6C. It passes driver type 0 with an explicit adapter, and the feature levels {11_0, 10_1, 10_0, 9_3, 9_2, 9_1}. That is DX11 at FL 11_0.
- **Deferred renderer with a persistent depth buffer:** it has `stage_gbuffer`, `gbuffer_pass`, `effect_gbuffer_depth`, `geometry_depth_pass`, `$user$depth`, `copy_depth_rt`, `hiz_depth*` (Hi-Z occlusion), "deferred lighting", "deferred decals", SSAO, SSR and LPV (`r_lpv_*`).
- **Motion vectors exist:**
  - Render target `$user$object_motion_vectors` / `t_object_motion_vectors`.
  - `effect_motion_vectors_accumulation`.
  - `stage_postprocess::accumulate_motion_vectors` at VA 0x127716.
  - `prev_view_matrix_parameter` / `prev_view`.
  - The motion vectors appear to be *per-object* (skinned and moving meshes). Camera motion is reconstructed from depth plus the previous view matrix. DLSS wants full-screen MVs, so a small extra pass, or the TAA shader's own reprojection, would have to produce them.
- **TAA with jitter:**
  - It has `effect_temporal_antialiasing` and the cvar `r_use_motion_vectors_in_taa`.
  - The UI option `st_options_2taa` means "2x TAA". `r_antialiasing_method` selects between FXAA (`r_fxaa_quality_*`) and TAA.
  - `temporal_projection_matrix_modifier::push_jittering` (VA 0x13275F, called from 0xEABD1) applies a **2-phase** subpixel jitter. It alternates (+0.25, −0.25) and (−0.25, +0.25) px on frame parity, scaled by 1/resolution.
  - DLSS expects a long, Halton-like jitter sequence, about 8 or more phases. That would need a patched table and an index counter, which a code cave can do.
  - SSAO also has temporal filtering (`r_ssao_use_temporal_filtering`, `ssao_prev_accumulator`).
- **Resolution cvars:** `r_resolution`, `r_resolution_x`, `r_resolution_y`, `r_fullscreen`, `r_vsync`, `r_force_no_vsync`, `r_max_fps`, `r_monitor_index`, and `enable all screen resolutions`. There is **no render-scale / internal-resolution cvar**: scene resolution equals backbuffer resolution. Only `r_ssao_screen_ratio` scales a sub-pass. Real upscaling therefore also needs the scene RTs decoupled from the swapchain size, which means many render-target allocations in `stage_*`.
- **Conclusion:** the inputs DLSS-SR needs (depth, MVs, jitter) are mostly present, which is rare for a 2014 engine. Bitness is the hard blocker, not the engine.

## 4. Verdict and paths

| Path | What you get | Effort | Risk |
|---|---|---|---|
| **A. Driver/out-of-process (recommended)**: Smooth Motion (RTX 40/50) or Lossless Scaling LSFG for FG; DLDSR/NIS/LS1/Magpie for scaling. Run the game in borderless/windowed mode (`r_fullscreen 0`); cap base FPS. | FG roughly 2x perceived FPS; spatial upscale or AA | Minutes; zero binary changes | Low. FG adds latency and HUD/crosshair artifacts. Smooth Motion on a 32-bit DX11 title is unverified, so test it. Overlay capture does not touch the Scaleform heap race. |
| **B. ReShade (32-bit DX11) sharpening / spatial AA** | CAS sharpening on top of the native TAA | Low | Low-medium. An extra in-process hook alongside our patched exe and the Scaleform crash surface. |
| **C. Real DLSS-SR via a 64-bit helper process** | Real DLSS 4.5 SR / DLAA | Very high (weeks). A d3d11 proxy or hooks in the 32-bit exe; decouple render resolution; Halton jitter patch (code cave at 0x6A1E0B area); full-screen MV pass; shared textures and keyed-mutex sync; a 64-bit NGX host. | High. Cross-process sync stalls, device-loss and alt-tab handling (already a known crash area), and per-frame latency. Each frame costs extra GPU copies, which can eat the DLSS gain at 1080p. |
| **D. DLSS 5 / OptiScaler / DLSSTweaks / DLSS Enabler** | n/a | n/a | Not applicable: 32-bit, no existing upscaler hook, and DLSS 5 needs a studio integration on RTX 50 |

There is no anti-cheat concern on our private server. The crash surface ranks as follows, safest first: driver features, then out-of-process overlays, then ReShade, then Path C.

**Recommendation:**
1. Use Path A. Try NVIDIA Smooth Motion in the NVIDIA App per-program settings for `survarium.exe`. If it is unavailable for this exe or GPU, use Lossless Scaling in borderless mode.
2. For sharpness, keep the in-game 2x TAA and add DLDSR or NIS.
3. Treat Path C as a separate R&D project, worth it only if real DLSS-SR is a hard requirement.

## Sources

- DLSS 5 announcement: https://www.guru3d.com/story/nvidia-unveils-dlss-5-with-neural-rendering-for-more-realistic-games/ ; https://www.ubergizmo.com/2026/03/nvidia-dlss-5/ ; https://www.mysmartprice.com/gear/laptops-pc/nvidia-announces-dlss-5-with-3d-guided-neural-rendering-for-rtx-50-series-graphics-cards/
- DLSS 4.5: https://www.guru3d.com/story/nvidia-dlss-45-expands-ai-frame-generation-to-6x-and-improves-image-sharpness/ ; https://www.notebookcheck.net/Nvidia-DLSS-4-5-brings-2nd-gen-Transformer-and-6x-multi-frame-gen-to-RTX-50-Blackwell-RTX-20-and-RTX-30-stand-to-benefit-with-caveats.1197464.0.html
- DLSS SDK libs (no Win32): https://github.com/NVIDIA/DLSS/tree/main/lib ; https://developer.nvidia.com/rtx/dlss/get-started
- Streamline architectures (amd64/arm64/arm64ec): https://github.com/NVIDIA-RTX/Streamline
- Unity: DLSS not supported on Win32 x86: https://docs.unity3d.com/cn/2023.2/Manual/deep-learning-super-sampling.html
- OptiScaler README (DX11 backends, needs existing DLSS2+/FSR2+/XeSS): https://cdn.jsdelivr.net/gh/optiscaler/optiscaler@master/README.md ; https://www.guru3d.com/story/optiscaler-integrates-amd-across-dx11-and-dx12-titles/
- DLSSTweaks: https://nexusmods.com/site/mods/550 ; https://guru3d.com/story/dlsstweaks-tweaking-tool-can-enable-dlaa-support-in-dlss-compatible-games
- DLSS Enabler: https://www.dsogaming.com/news/dlss-enabler-simulates-dlss-4-multi-frame-generation-on-any-dx12-gpu-and-game/
- dxvk-nvapi: https://github.com/jp7677/dxvk-nvapi
- Smooth Motion: https://www.guru3d.com/story/nvidia-adds-smooth-motion-frames-driverlevel-frame-interpolation ; https://www.dsogaming.com/news/smooth-motion-is-finally-available-to-nvidia-rtx-40-series-gpus
- Lossless Scaling: https://store.steampowered.com/app/993090 ; https://steamcommunity.com/app/993090/discussions/0/4418677017727367960
- Magpie: https://www.tomshardware.com/news/magpie-run-amd-fsr-on-any-game
