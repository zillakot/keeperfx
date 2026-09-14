---
type: guide
description: Collect isolated native presentation comparisons, wall-time distributions, process CPU time and scoped Rust allocation counts.
---

# Game performance measurements

Build the engine and prepare assets as described in [Mac development](macos.md).
Then run each scenario separately from the repository root:

```sh
python3 scripts/profile-game.py --scene quiet --out out/perf-quiet
python3 scripts/profile-game.py --scene busy --out out/perf-busy
python3 scripts/profile-game.py --scene possession --out out/perf-possession
```

The original presenter is selected explicitly by default. With the optional Rust
presenter built on macOS, add `--backend rust` for live Rust/wgpu measurements.
The runner requires Cocoa/Metal and rejects a fallback to SDL in a requested Rust
run. Actual Rust adapter, surface format and present mode are recorded separately
from requested settings. Rust measurements require a native window; headless
smoke tests support only the original presenter.

These commands open a native game window and exit after the bounded sample.
Keep the window visible during measurement. In an opted-in local game the hook
suppresses gameplay command input (including mouse-look) from startup through
completion, while SDL event polling continues. Other applications,
window occlusion, power mode, temperature and display configuration can affect
results; record those conditions when comparing runs. The runner clones only
assets into a temporary installation, creates empty saves, and records its own
configuration. It disables audio, the API, cursor capture and edge panning.
Personal saves and settings are not loaded or modified.

Defaults are a 640×480 window, 20 simulation turns per second, interpolation on,
60 drawing frames per second, VSync off, warmup to turn 40 and 200 measured turns
(about ten seconds). Possession waits for the controlled creature view before
starting. Options support other resolutions and bounded warmup/durations; use `--help`
for accepted values. VSync, interpolation and turn rate are fixed by the runner;
the frame cap is 60 FPS unless [`--uncapped`](#uncapped-measurement-mode) removes it.
Keep settings, engine build, assets and host conditions identical for comparisons.

## Scenarios and reproducibility

| Scenario | Fresh starting state and input |
| --- | --- |
| `quiet` | Original campaign (`keeporig`) level 1; initial dungeon camera, no commands |
| `busy` | Original campaign level 20; initial dungeon camera, no commands; existing AI dungeon and populated hero stronghold |
| `possession` | Original campaign level 1; enter the first eligible owned creature by thing index through the normal control function, then no commands |

The busy case exercises a larger live world. It does not promise a crowded camera,
combat, or a late-game stress test. Population snapshots in the engine metadata
show the actual loaded and evolved workload. Campaign and level overrides must
be reported as different scenarios, with their resulting population and view.

Each run records engine and asset identities, isolated configuration, command,
requested settings, actual presentation backend, video driver, VSync, framebuffer/output
sizes and engine frame limit. Start/end snapshots record simulation turn, creature
and total thing counts, active camera position/angles/zoom, controlled thing index
and the five game RNG states. These seeds are observations after startup; this
runner does not fix every startup clock/random source or replay commands. Matching
settings and asset hashes reproduce the procedure, not an identical simulation
state. Do not interpret similar distributions as gameplay determinism.

## What the measurements mean

Raw CSV durations are integer nanoseconds from `std::chrono::steady_clock`.
Reports convert them to milliseconds and retain sample counts, mean, median,
p90, p95, p99 and maximum. Percentiles use linear interpolation between ordered
samples. Separate distributions have different sample counts because simulation
and drawing run at different rates.

| Series | Boundary and interpretation |
| --- | --- |
| `simulation` | One `update()` call; excludes input polling, packet exchange and turn pacing |
| `draw` | One `keeper_screen_redraw()` call drawing world and HUD into CPU pixels; excludes light-area setup, focus waiting, direct-message overlays and presentation |
| `presentation` | Per-frame cursor composition, palette/pixel processing and upload, rendering/present submission and cursor cleanup. Original SDL includes texture lock, indexed-to-RGBA blit, texture unlock/upload and clear/draw/present. Rust includes its surface acquisition, uploads, submission and polling. Excludes target setup and metadata queries. |
| `present_wait` | Nested SDL or Rust present call, including host work and blocking inside that call; **already included in presentation**. Backend implementations distribute work differently, so this is a diagnostic, not a common GPU/VSync-cost measurement. |
| `frame_interval` | Time between starts of successive measured presentation calls; includes simulation, drawing, event handling, pacing and scheduling between them |

All series measure elapsed **wall time**, including descheduling or waiting.
`draw` measures work implemented on the CPU, but is not a thread/process CPU-time
counter. `present_wait` is not a pure VSync wait: drivers can also block on texture
lock/upload or elsewhere. No GPU timestamps or GPU completion latency are
collected. Do not add nested series or call them GPU benchmarks.

Separate measured-window counters record process user and system CPU time using
`getrusage(RUSAGE_SELF)` on macOS/Linux or `GetProcessTimes` on Windows. They include
worker threads and exclude startup, warmup, sample-file output and shutdown. Reports
include CPU milliseconds per turn and presentation and average core equivalents
(process CPU time divided by measured wall time). These are process-wide counters,
not component CPU costs; they do not relabel the per-scope wall-time samples. Older
reports without these counters retain an explicit unavailable value.

Builds with live Rust support also record successful Rust global-allocator
allocation/reallocation calls and requested bytes over that window. They exclude
C/C++, SDL and driver/GPU allocations and do not measure retained memory. The
original backend normally does not activate Rust allocation paths; zero Rust
allocations there cannot establish a total-process memory advantage. Full heap
allocation and memory comparisons require additional measurement with explicit
coverage. Resource counter snapshots themselves add small unquantified overhead.

The existing on-screen timing display remains unchanged. Its logic scope includes
input and pacing (and can invoke drawing), while its draw scope includes
presentation. The exported scopes are narrower so those overlapping debug values
are not reused as component baselines.

## Coarse drawing breakdown

Add `--draw-breakdown` to collect four non-overlapping children of `draw`:

| Series | Boundary |
| --- | --- |
| `draw_scene` | `draw_view()` and `draw_frontview_engine()` setup and bucket construction before dispatch |
| `draw_raster` | Complete isometric/possession `display_drawlist()` dispatch, including terrain, sprites, shadows and bucket overlays |
| `draw_front_raster` | Complete front-view `display_fast_drawlist()` dispatch, including textured quads, sprites and bucket overlays |
| `draw_overlays` | View HUD, messages, hand and tooltip blocks, plus the common post-view overlay block in `redraw_display()` |
| `draw_unaccounted` | Derived per frame: `draw` minus the four children above |

Each child accumulates all visits within one draw and emits exactly one sample,
including zero for an unvisited scope. The engine rejects overlapping or unfinished
children. The report requires complete, ordered child blocks immediately preceding
their enclosing draw, and rejects a child sum exceeding its parent. These children
are already included in `draw`; do not add them to it. Zero does not prove a family
was absent: uninstrumented paths remain in `draw_unaccounted`.

These are whole dispatch timings, with no per-primitive or per-pixel clocks.
Terrain and sprites remain interleaved inside each dispatch; use a separate sampling
profile to investigate those families. Unaccounted time includes framebuffer setup,
view orchestration, smoothing/lens effects outside the hooks, other paths and timer
bookkeeping. Scene preparation also includes existing state updates at that boundary.
These scopes describe host work and do not establish GPU execution time.

The flag sets `KFX_PERF_DRAW_BREAKDOWN=1`; the default explicitly sets it to `0`.
Both modes keep the existing top-level timing boundaries and the run's frame cap. Compare
serial matched runs from the same executable with and without this flag to quantify
the added clock, bookkeeping and sample overhead before interpreting the breakdown.
The default mode still has inactive hook calls. Four extra records per drawn frame
share the existing 100,000-record limit; long runs can reach that limit sooner.

## Drawing-backend counters

Every report states the drawing backend that was actually active (`software`,
`wgpu`, or `wgpu-fallback` once the bridge has failed), so a silent fallback is
visible instead of being read as a GPU result. A backend that changes inside the
measured window invalidates the run.

With GPU drawing active the engine samples the bridge, drawing-context and frame
counters once per presented frame and keeps only per-frame deltas for the
measured window, emitted through the existing JSON sidecar; there is no per-frame
file I/O. The report gives per-frame min, mean, p95 and max plus the window total
for queue submits, full-target compute dispatches, blocking device polls and
their measured host wait time, frame checkpoints and GPU-to-GPU checkpoint copy
bytes, aggregate validation waits, kernel-flagged invalid frames, skipped status
publishes, command and asset upload bytes, GPU readback bytes, full-target
readbacks, buffer allocations and bytes, batches, commands and ordered sprites.
`checkpoint_copy_bytes` and `validation_waits` are structurally zero: a queued
frame records its batches straight into the root and publishes its validation flag
through a mapped ring instead of a blocking poll. `flagged_invalid_frames` counts
frames a raster kernel found an out-of-range lookup in — each one is presented as
drawn and then redrawn — and `status_stalls` counts publishes skipped because
every ring slot was still mapped, which defers a flag rather than losing it. `dispatches` counts every compute dispatch the drawing context
issues, including the single-workgroup ordered-sprite passes.
`host_staged_asset_bytes` is a gauge sampled at frame end holding the CPU copies
the drawing context stages, not GPU memory, so its window total is meaningless.

`arena_evictions`, `arena_overflows` and `arena_bytes_uploaded` cover the
persistent asset arena: LRU reclaims, allocations that could not be satisfied and
so rejected their batch, and bytes written into the arena.
`arena_bytes_resident` is a second gauge holding the arena extent suballocated so
far, including free-listed slots and power-of-two class padding, so it bounds the
live working set rather than tracking it exactly and its window total is
meaningless. It counts expanded arena bytes — one `u32` per source byte, the GPU
footprint — so dividing by four gives the real asset bytes that the arena's
source-byte capacity is stated in. `upload_bytes` still counts every asset write, arena or not.

The first presentation only establishes the counter baseline, so there is exactly
one fewer counter frame than presentation sample. `wait_ns` is host time blocked
inside device polls and is already contained in the enclosing `draw` and
`presentation` wall-clock scopes; it must not be added to them.

**GPU execution time is opt-in and per pass.** With `KFX_WGPU_GPU_TIMING=1` on an
adapter exposing `TIMESTAMP_QUERY`, each compute and render pass carries
`timestamp_writes` and the `gpu_*_ns` counters report GPU duration per pass kind.
`TIMESTAMP_QUERY_INSIDE_ENCODERS`, which this Metal adapter lacks, is needed only
to stamp outside a pass, so copy-only submissions stay untimed. Every other counter
here is host-side and none of them may be read as a GPU timing. `gpu_untimed_passes`
is nonzero when the resolve ring was saturated, and the per-pass totals then
under-report.

**A pass window is not exclusive.** It runs from that pass's own begin stamp to its
own end stamp, so it includes whatever the pass waited through; a `gpu_*_ns` figure
attributes cost rather than measuring it, and the sum of the windows decomposes
nothing. `gpu_pass_union_ns` is the union of the frame's timed pass intervals,
closed once per frame, so windows overlapping in time count once. It is an upper
bound on the frame's GPU occupancy and, measured, equals the window sum on this
Metal adapter: the windows are disjoint and each holds its own stall. Neither is
bounded by the `presentation` scope, which is host-side and ends at hand-off while
the GPU runs past it; the frame interval is the bound that holds.
`KFX_WGPU_GPU_TIMING=2` (`profile-game.py --serial-gpu-timing`) drains the queue
after every timed submission, so each window then holds its pass alone — the only
exclusive measurement here, and it serialises the frame, so it is a diagnostic and
not a performance baseline.

Counters cover the drawing context the bridge owns. Surface acquisition,
presentation by the Rust presenter, and a cursor that owns its own drawing context
rather than borrowing the bridge's are not counted, so the counters explain the
drawing scopes rather than the whole frame.

## Collection bounds and outputs

The hook is inactive unless `KFX_PERF_OUTPUT` names a new CSV file whose parent
exists. The runner sets `KFX_PERF_TURN`, `KFX_PERF_TURNS` and
`KFX_PERF_SCENE=dungeon|possession` only for its child. It strips inherited capture
and profiling options, backend selectors and Rust fault/verification hooks;
simultaneous frame capture is rejected by the engine.
The runner rejects existing output directories and output outside ignored `out`.

The hook reserves at most 100,000 records (about 2.4 MB on a 64-bit build), takes
clock readings only while active, and performs no sample file I/O until the run
ends. Population/seed snapshots occur outside timed work. This is low-overhead
instrumentation by design, not a measured zero-overhead guarantee: timer calls,
record insertion and metadata queries still perturb execution. A future backend
must use the same boundaries and collector for a fair comparison.

Warmup is bounded to 1–600 turns and measurement to 20–1,200 turns. The runner
also enforces a wall-clock process timeout, covering startup failures, menus,
pauses, missing possession candidates or stalled turns. Sample overflow, invalid
options, wrong local-game/view state and failed presentation invalidate the run.
Only a complete engine sidecar plus validated samples can produce a report.

The output directory contains raw CSV, its engine JSON sidecar, JSON and Markdown summaries,
the isolated settings and process/game logs. The JSON summary includes the input
identity and request manifest. Keep complete local
results for later comparison; original game data and captured results stay under
ignored `out`. PRs may record aggregate timings and conditions without uploading
original assets.

## Uncapped measurement mode

```sh
python3 scripts/profile-game.py --scene busy --resolution 1920x1080 --uncapped --out out/perf-uncapped
```

`--uncapped` writes `FRAMES_PER_SECOND=0` into the isolated configuration instead
of `60`. Everything else is unchanged: `DELTA_TIME=ON`, `TURNS_PER_SECOND=20`,
`VSYNC=OFF`, the same scenes, warmup, durations and collector.

### What uncapped means in this engine

`FRAMES_PER_SECOND` is parsed by
[`parse_draw_fps_config_val`](../src/config_keeperfx.c) into
`start_params.num_fps_draw_main`, which
[`redetect_screen_refresh_rate_for_draw`](../src/main.cpp) turns into
`fps_limit_current`: `-1` means the display refresh rate, a positive value is that
value, and `0` leaves `fps_limit_current` at zero. The frame limiter in
[`gameplay_loop_draw`](../src/game_loop.c) is guarded by `fps_limit_current > 0`;
with zero it is skipped entirely, so nothing accumulates `process_frame_time`, the
`SDL_Delay(1)` in that block never runs and no frame is suppressed by the limiter.

The simulation rate is independent of it. With interpolation on,
`gameplay_loop_timestep` skips `keeper_wait_for_next_turn` entirely, so the only
pacing is `game.process_turn_time`, which accumulates elapsed seconds times
`turns_per_second`. Drawing fills the gap: `gameplay_loop_logic` spins
`gameplay_loop_draw` until the next turn is due. Uncapped therefore means drawing
as fast as the host allows while the simulation still targets 20 turns per second.

VSync is off on both presenters and independently confirmed. `vsync_enabled` is
false, so the SDL path calls `SDL_SetRenderVSync(renderer, 0)`
([`RendererSoftware`](../src/kfx/renderer/RendererSoftware.cpp)) and the wgpu path
selects `Immediate`, falling back to `Mailbox` and otherwise failing rather than
silently configuring `Fifo` ([`live.rs`](../tools/frame-replay/src/live.rs)). The
runner already rejects a Rust run whose reported present mode is neither.

### Runner and report contract

The engine reports `fps_limit_current` as `fps_limit` in its metadata sidecar. The
runner requires it to equal the requested cap: `60` by default, `0` under
`--uncapped`. A run whose engine did not apply the requested cap is rejected, in
both directions, so a stale configuration cannot be read as an uncapped result.

`report.json` carries a `frame_cap` object (`uncapped`, `requested_fps_limit`,
`engine_fps_limit`, `label`) and an `observed` object with the derived
frames-per-second and turns-per-second figures. `report.md` states the cap beside
its title, beside the engine frame limit, and after every frame-rate figure, and
reports observed turns per second beside observed FPS. The uncapped limitation
list replaces the capped-FPS caveat with its own.

`benchmark-presenters.py` accepts the same `--uncapped` and applies it to every
run in the experiment. `compare` refuses a set of reports that mixes capped and
uncapped runs, and the cap is part of each scene's recorded identity, so an
uncapped run can never be paired against a capped one.

### Limits

An uncapped figure is host wall-clock pacing of this process on this host under
the conditions of that run. It is not a portable frame-rate claim, and uncapped
numbers must never be compared with capped ones.

Observed turns per second is the load-bearing check. The measured window is paced
by the simulation, so a value below 20.00 means the host could not sustain the
simulation while drawing; such a run measures a degraded loop, not a drawing
ceiling, and its FPS figure describes that degraded loop.

Uncapped runs present far more frames than capped ones for the same turn count, so
the 100,000-sample limit is reached sooner; keep `--turns` bounded and check the
sample counts in the report. Presentation tails also widen, because the swapchain
is queried continuously rather than once per vertical interval.

### Recorded uncapped matrix

Source `e00fcd322`, Apple M5, Metal, `Bgra8Unorm`, present mode `Immediate`, one
engine and asset set across every run, 200 measured turns each, collected serially
under the timing lock with audio, control/API and readback disabled. One run per
cell: descriptive, not a repeated experiment.

First, the cap semantics themselves, busy 640×480 on the SDL presenter:

| Frame cap | Frame interval mean / p95 ms | Observed FPS | Turns/s | Presentations |
| --- | ---: | ---: | ---: | ---: |
| 60 FPS | 16.665 / 17.491 | 60.01 | 20.00 | 600 |
| uncapped | 2.901 / 8.699 | 344.67 | 20.00 | 3,447 |

The cap holds the frame interval at the 16.67 ms period; removing it drops the
interval to 2.9 ms and leaves the simulation at exactly 20.00 turns per second.

| Presenter | Drawing | Scene | Logical | Frame interval mean / p95 ms | FPS | Turns/s | Draw mean | Presentation mean |
| --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| SDL | software | busy | 640×480 | 3.085 / 11.131 | 324.17 | 20.02 | 2.083 | 0.954 |
| SDL | software | quiet | 640×480 | 5.404 / 13.758 | 185.06 | 20.00 | 2.348 | 3.021 |
| SDL | software | busy | 1920×1080 | 10.139 / 12.772 | 98.63 | 19.99 | 8.737 | 1.289 |
| SDL | software | quiet | 1920×1080 | 13.346 / 18.537 | 74.93 | 20.00 | 9.190 | 4.104 |
| wgpu | software | busy | 640×480 | 4.629 / 12.987 | 216.01 | 20.01 | 2.268 | 2.305 |
| wgpu | software | quiet | 640×480 | 4.475 / 12.643 | 223.45 | 19.99 | 2.372 | 2.084 |
| wgpu | software | busy | 1920×1080 | 13.353 / 18.281 | 74.89 | 19.99 | 8.514 | 4.713 |
| wgpu | software | quiet | 1920×1080 | 10.755 / 16.461 | 92.98 | 20.02 | 9.236 | 1.474 |
| wgpu | wgpu | busy | 640×480 | 12.994 / 17.574 | 76.96 | 19.99 | 1.520 | 11.307 |
| wgpu | wgpu | quiet | 640×480 | 10.649 / 13.643 | 93.91 | 20.00 | 1.476 | 9.107 |
| wgpu | wgpu | busy | 1920×1080 | 85.258 / 100.505 | 11.73 | **11.73** | 1.609 | 83.163 |
| wgpu | wgpu | quiet | 1920×1080 | 67.449 / 70.853 | 14.83 | **14.83** | 1.366 | 65.923 |

Per-frame GPU pass means for the GPU-drawing rows, summed over pass kinds:

| Scene | Logical | raster | minimap | target_trig | ordered_sprite | shadow_mask | Sum | Submits | Blocking waits |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| busy | 640×480 | 4.450 | 2.169 | 1.833 | 0.582 | 0.272 | 9.571 | 45.5 | 0 |
| quiet | 640×480 | 3.472 | 1.469 | 1.636 | 0.213 | 0.201 | 7.287 | 31.5 | 0 |
| busy | 1920×1080 | 57.644 | 18.604 | 8.489 | 6.589 | 0.194 | 91.649 | 38.4 | 0 |
| quiet | 1920×1080 | 35.188 | 10.909 | 10.678 | 1.444 | 0.220 | 58.478 | 31.4 | 0 |

`gpu_untimed_passes` was zero in all four, so no pass went unstamped. Passes can
overlap on the device, so the sum is an upper bound on attributed GPU time per
frame, not a serial total; the busy 1080p sum exceeds that row's frame interval
for exactly that reason.

**The two 1080p GPU-drawing rows do not measure a ceiling.** Their turns per second
is 11.73 and 14.83, below the requested 20, and equals their FPS exactly: the frame
is longer than a turn, so the loop presents once per turn and the simulation falls
behind. Those figures describe a degraded loop.

What bounds each measured ceiling:

- **Software drawing** is bound by CPU drawing. `draw` is 2.1–2.4 ms at 640×480 and
  8.5–9.2 ms at 1080p, and the frame interval tracks it.
- **The wgpu presenter costs about 2.3 ms per frame at 640×480** with `present_wait`
  at 0.005 ms. Surface acquisition, the index and palette uploads and the palette
  pass — not the present call — are what hold the wgpu software path at 216 FPS
  where SDL reaches 324 FPS. Capped at 60 that cost is invisible.
- **GPU drawing is bound by GPU execution.** `draw` falls to ~1.5 ms because the CPU
  only encodes, and the host then blocks in `presentation` (9–11 ms at 640×480,
  66–83 ms at 1080p) for the frame's GPU work to complete. The per-pass totals
  match, and raster is 60% of them at 1080p.

Every figure here is uncapped host wall-clock timing on this host.

## Repeated native A/B comparison

After all builds and other performance work stop, collect five serial pairs for
each scene with one executable containing both presenters:

```sh
python3 scripts/benchmark-presenters.py --out out/presentation-ab \
  --conditions 'Record display/scaling, power mode, window visibility and background load here'
```

Replace the conditions text with actual observations. The driver runs 30 separate
processes, alternates original/Rust order between pairs and rotates scene order.
Every run uses the same default warmup, duration and settings; optional
`--warmup-turns`, `--turns` and `--resolution` apply to both paths. Keep the window
visible and unobscured throughout. Never run simultaneous benchmark windows.

The driver rejects mismatched binary, asset, configuration, host or actual output
identities within each scene. It preserves raw per-run artifacts beneath `runs/`
and records order and conditions in `manifest.json`. Failures stop collection and
remain in place. Inspect state/population snapshots for workload differences;
the runner does not make startup randomness deterministic.

`comparison.json` and `comparison.md` give each run equal weight. They report the
median and range of run medians and run p95s, and paired percentage changes.
The median of run p95s is labelled as such; it is not a pooled p95. With original
duration O and Rust duration R, time saved is `100 * (O - R) / O` and speedup factor
is `O / R`. Negative time saved means a regression. Keep absolute milliseconds
beside percentages and report scenes separately. Five pairs support descriptive
results; varied signs or wide ranges warrant an inconclusive result or further
complete rounds, not a significance claim.

Both paths remain capped at 60 FPS by default. Lower presentation duration means
reduced host overhead; 60 versus 60 FPS does not prove an uncapped gameplay
speedup. `--uncapped` collects a matched uncapped experiment instead, and the
driver refuses to mix the two; its results remain a separate matrix and do not
reinterpret the capped one. Rust VSync-off measurements require actual Immediate
or Mailbox present mode; silent FIFO fallback is rejected.

The timing matrix does not replace the live milestone's pixel, lifecycle,
gameplay, audio and save/reload checks in the [Rust port plan](product/rust-port-plan.md).
The profiling runner disables sound and gameplay commands, so run those checks
separately. Scoped Rust allocation counts also leave total-process allocation
and memory comparisons outstanding.

## Recorded live presentation result

[PR #9](https://github.com/zillakot/keeperfx/pull/9) records the final 30 serial runs
on source `506b703a35b84f4adb1bbe92d5f109166b3f8481`: five pairs per scene,
200 simulation updates and 600 presentations per run. Both paths used Metal,
640×480 framebuffer/output, VSync off, 20 turns/s, interpolation on and a 60 FPS
cap. Control/API, audio and verification/readback were disabled. Rust used
`Bgra8Unorm` and Immediate mode; no run fell back or failed collection.

| Scene | Presentation median ms, SDL → Rust | Paired median reduction, median (range) | Paired p95 increase, median |
| --- | ---: | ---: | ---: |
| Quiet | 0.644 → 0.623 | 3.811% (-0.940 to 6.107%) | 6.636% |
| Busy | 0.634 → 0.604 | 1.038% (-15.914 to 17.456%) | 9.358% |
| Possession | 0.676 → 0.627 | 3.072% (-11.269 to 9.868%) | 9.336% |

Absolute values are medians of per-run statistics; percentages are calculated
within pairs before aggregation. Every scene's median-reduction range crosses
zero, and presentation p95 increased in all 15 pairs. Process CPU savings also
had ranges crossing zero in every scene. Both paths stayed near the cap; there is
no demonstrated uncapped FPS gain. These results supersede earlier pre-control
9–13% presentation improvement claims and do not justify changing the SDL default.

All 120 recorded artifact hashes, common engine/assets/config identities, sample
counts, scheduled order and aggregate recomputation were independently checked.
The experiment remains descriptive: startup RNG/populations varied, and background
load, temperature, visibility/occlusion and display placement were not continuously
controlled or independently verified. Rust allocation counters do not cover the
whole process or GPU. PR #9 retains the full tables and evidence boundaries.

The [active graphics migration](product/rust-port-plan.md#active-delivery-full-wgpu-drawing)
starts by separating CPU scene preparation, rasterization and overlays, evaluating
higher framebuffer resolution and choosing a pre-rasterization command boundary,
then continues through GPU implementation and complete drawing coverage. The
runner's [uncapped mode](#uncapped-measurement-mode) supplies the ceiling figures
that migration needs, in a matrix kept separate from the capped one. Existing
presenter results do not validate the drawing migration.

## Asset-free checks and headless validation

```sh
python3 -m unittest discover -s scripts/tests -v
python3 scripts/profile-game.py --headless --scene quiet --turns 20 --out out/perf-smoke
```

Unit tests use synthetic CSV and fake engines. The second command needs local
original assets and validates the collection pipeline using SDL dummy/software;
its report is explicitly labelled headless. It is **not native window performance**
and must not be compared to a live SDL/Metal or Rust surface baseline.
CI runs redistributable checks without original game assets. Native performance
runs remain local, and their results are not CI performance thresholds.
