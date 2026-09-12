---
type: guide
description: Collect bounded, isolated wall-time baselines of the original game simulation, CPU drawing and SDL presentation.
---

# Original game performance baselines

Build the engine and prepare assets as described in [Mac development](macos.md).
Then run each scenario separately from the repository root:

```sh
python3 scripts/profile-game.py --scene quiet --out out/perf-quiet
python3 scripts/profile-game.py --scene busy --out out/perf-busy
python3 scripts/profile-game.py --scene possession --out out/perf-possession
```

These commands open a native game window and exit after the bounded sample.
Keep the window visible and avoid input during measurement. Other applications,
window occlusion, power mode, temperature and display configuration can affect
results; record those conditions when comparing runs. The runner clones only
assets into a temporary installation, creates empty saves, and records its own
configuration. It disables audio, the API, cursor capture and edge panning.
Personal saves and settings are not loaded or modified.

Defaults are a 640×480 window, 20 simulation turns per second, interpolation on,
60 drawing frames per second, VSync off, warmup to turn 40 and 200 measured turns
(about ten seconds). Possession waits for the controlled creature view before
starting. Options support other resolutions and bounded warmup/durations; use `--help`
for accepted values. Frame cap, VSync, interpolation and turn rate are fixed by
the runner. Keep settings,
engine build, assets and host conditions identical for comparisons.

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
requested settings, actual SDL backend, video driver, VSync, framebuffer/output
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
| `presentation` | Original software renderer's texture lock, cursor composition, indexed-to-RGBA blit, texture unlock/upload and SDL clear/draw/present submission, through cursor cleanup; excludes present-target setup and metadata queries |
| `present_wait` | Nested `SDL_RenderPresent()` call, including any CPU/driver work and blocking inside that call; **already included in presentation** |
| `frame_interval` | Time between starts of successive measured presentation calls; includes simulation, drawing, event handling, pacing and scheduling between them |

All series measure elapsed **wall time**, including descheduling or waiting.
`draw` measures work implemented on the CPU, but is not a thread/process CPU-time
counter. `present_wait` is not a pure VSync wait: drivers can also block on texture
lock/upload or elsewhere. No GPU timestamps, GPU completion latency, process CPU
time, allocation counts or total memory measurements are collected. Do not add
nested series or call them GPU benchmarks. Additional counters are needed before
making CPU-utilization, GPU-cost or memory-regression claims.

The existing on-screen timing display remains unchanged. Its logic scope includes
input and pacing (and can invoke drawing), while its draw scope includes
presentation. The exported scopes are narrower so those overlapping debug values
are not reused as component baselines.

## Collection bounds and outputs

The hook is inactive unless `KFX_PERF_OUTPUT` names a new CSV file whose parent
exists. The runner sets `KFX_PERF_TURN`, `KFX_PERF_TURNS` and
`KFX_PERF_SCENE=dungeon|possession` only for its child. It strips inherited capture
and profiling options; simultaneous frame capture is rejected by the engine.
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

## Asset-free checks and headless validation

```sh
python3 -m unittest discover -s scripts/tests -v
python3 scripts/profile-game.py --headless --scene quiet --turns 20 --out out/perf-smoke
```

Unit tests use synthetic CSV and fake engines. The second command needs local
original assets and validates the collection pipeline using SDL dummy/software;
its report is explicitly labelled headless. It is **not native window performance**
and must not be compared to a live SDL/Metal or future Rust surface baseline.
CI runs redistributable checks without original game assets. Native performance
runs remain local, and their results are not CI performance thresholds.
