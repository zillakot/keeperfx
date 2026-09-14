---
type: product
description: Bounded plan of tooling and process improvements for the graphics track — GPU time attribution, offscreen measurement, deterministic scenes, command-stream replay, bounds property tests, CI structure, documentation hygiene and the parity policy.
---

# Development tooling plan

This plan covers tooling and process for the graphics track only; the migration
sequence itself stays in the
[single-stream wgpu renderer design](../architecture/wgpu-single-stream-renderer.md)
and the [rust port plan](rust-port-plan.md#graphics-and-performance-track).

Each item names the incident that motivated it, the deliverable, the acceptance
criterion, a size estimate and its dependencies. Sizes are S (under a session),
M (a session) and L (more than one session).

| # | Item | Size | Depends on |
| --- | --- | --- | --- |
| 1 | [GPU time attribution](#1-gpu-time-attribution) | M | 2 for reliable capture; counters and serialised mode delivered, trace profiler open |
| 2 | [Offscreen measurement mode](#2-offscreen-measurement-mode) | M | None |
| 3 | [Deterministic scene mode](#3-deterministic-scene-mode) | M | None |
| 4 | [Command-stream capture and offline replay](#4-command-stream-capture-and-offline-replay) | L | 1 for per-pass timing, 3 for a stable capture |
| 5 | [Bounds-superset property test](#5-bounds-superset-property-test) | M | None |
| 6 | [CI structure](#6-ci-structure) | M | 5 shares the fixture matrix |
| 7 | [Documentation hygiene](#7-documentation-hygiene) | S | None |
| 8 | [Parity policy](#8-parity-policy) | S | None |

## 1. GPU time attribution

**Observed.** After terrain binning landed, `gpu_minimap_ns` at 1080p read 16.4 ms
and appeared to triple, although none of the binning commits touch the minimap.
The per-pass windows are not exclusive: a pass that waits on a predecessor charges
the stall to itself, so the measurement redistributed a roughly constant total.

**Deliverable.** Counter and statement parts delivered with the tight-bin-boxes PR;
the trace profiler is open.

The frame-wide timestamp pair this plan named does not exist, and the reason is the
plan's own finding measured properly. Two designs were built and both are recorded
here because the negative result is the deliverable.

A first-begin-to-last-end span per frame was implemented first and discarded: it
reported about a second per frame natively, because grouping absolute stamps by frame
lets one stale timestamp swallow the whole window, and an encoder span also counts the
host gaps between submissions as GPU time. What shipped instead is
`gpu_pass_union_ns`, the union of the frame's timed pass intervals closed once per
frame, which a single bad interval can only inflate by its own length.

**The union collapses nothing on this adapter.** Measured on a busy 1080p frame it is
8.006 ms against an 8.006 ms window sum, and at 640x480 3.910 against 3.910 — the pass
windows are disjoint in GPU time. The overlap the incident inferred is therefore not
concurrency between passes: each window *contains* its own stall. The same run
serialised (`KFX_WGPU_GPU_TIMING=2`, `profile-game.py --serial-gpu-timing`) reports
**3.549 ms**, so 56 % of the unserialised window total is waiting inside the windows.
`gpu_pass_union_ns` is published as an upper bound on GPU occupancy and nothing more;
the serialised sum is the exclusive number, and it is what the acceptance tables cite.

The statement that per-pass windows include dependency stalls is in the renderer
design, the [live guide](../live-rust-presentation.md), the profiler's own limitations
and [performance baselines](../performance-baselines.md).

Still open: a profiling script that launches an isolated game session under
`xctrace record --template "Metal System Trace"` and summarizes per-pass GPU time
from the trace.

**Acceptance.** Two earlier criteria are withdrawn because neither discriminates.
`gpu_pass_union_ns <= presentation` is wrong: `presentation` is a host scope that ends
at hand-off while the GPU runs past it, and the capped 1080p run measures 8.006 against
4.765 ms. `frame_interval >= gpu_pass_union_ns >= max(gpu_*_ns)` is satisfied by the
window sum itself, so it cannot tell an overlap-free counter from the counter it
replaces, and it is not an invariant either — a GPU-bound frame breaks it. The union is
published only as an upper bound on GPU occupancy.

What is left is a property only a serialised run can establish: **the frame interval
must exceed the serialised GPU sum**, and **the serialised sum is the figure an
acceptance table cites for GPU cost**. Met on a busy 1080p pair: 17.284 ms interval
against 3.549 ms of serialised GPU work. Remaining acceptance for the trace profiler: a
per-pass summary for a busy 1080p run whose totals agree with that serialised sum, after
which every published pass attribution cites the profiler rather than the counter
windows.

**Size.** M. **Dependencies.** Item 2 for a capture that does not depend on a
visible window.

## 2. Offscreen measurement mode

**Observed.** Hours of GPU timing were lost because the presenter could not
acquire a drawable: the engine aborted captures with
`Rust surface acquisition skipped` and `Rust presenter shutdown after 0 frames`
while the console was locked and a full-screen call window occluded the game.
Window focus and occlusion were not observed continuously, and other worktrees
ran concurrent work during the same windows.

**Deliverable.** A wgpu presentation path that renders to an offscreen texture
with no swapchain, selected by the profiling runner, so timing does not depend on
an unlocked, unoccluded display. Alongside it, environment guards in the runner:
detect a locked console session (`CGSSessionScreenIsLocked` via `ioreg`), detect
occlusion (zero presented frames over the measured window), detect background CPU
load above a configured threshold, and refuse the run or annotate its report.
Formalize the timing lock file `/private/tmp/keeperfx-timing.lock` inside the
runner so builds and other measurements never overlap a timing window.

**Acceptance.** A busy 1080p timing run completes with the console locked and
produces the same per-pass counters as an unlocked run within the usual run
spread; a run started under a locked session on the swapchain path, under
detected occlusion, or under excess background load is refused with a named
reason; a second runner invocation blocks on the lock file instead of measuring.

**Size.** M. **Dependencies.** None.

## 3. Deterministic scene mode

**Observed.** [`capture-frame.py`](../../scripts/capture-frame.py) was
non-deterministic for the busy scene under both backends, so whole-frame A/B
comparisons between the software and GPU paths had to be abandoned and the
comparison fell back to per-family fixtures.

**Deliverable.** A [game control](../native-game-control.md) option that fixes the
RNG seed, the camera position and the creature population and disables
interpolation, so two runs reaching the same turn produce identical frames.

**Acceptance.** Two independent sessions at the same turn, same settings and same
backend produce byte-identical indexed framebuffers; the same holds across the
software and GPU backends for the families already at parity.

**Size.** M. **Dependencies.** None.

## 4. Command-stream capture and offline replay

**Observed.** Every per-pass and per-family measurement so far requires a live
game session on a Mac with a display, which is what made items 1 and 2 expensive
and keeps the frame-level cost model out of CI.

**Deliverable.** A capture that writes one frame's stream records, view table,
asset arena contents and tile lists to a file from a live session, and a replay
in the Rust crate that consumes that file as both a benchmark and a parity check,
runnable in CI without the game.

**Acceptance.** A captured busy 1080p frame replays offline with pixels identical
to the live frame and reports per-pass GPU time; the replay runs as a CI job on
the standard runner.

**Size.** L. **Dependencies.** Item 1 for the per-pass timing it reports, item 3
for a capture that can be reproduced.

## 5. Bounds-superset property test

**Observed.** Four separate bounds defects surfaced in one day: a shadow scratch
prior read outside its declared box, a mixed-target replay heap overflow, an
empty ordered-sprite rectangle, and five emitters declaring whole-screen bounds.
Each was found by review or by a crash, not by a test.

**Deliverable.** A property test that, for every record kind, generates random
commands, runs the kernel and asserts that every written pixel lies inside the
record's declared bin box and that every replay or oracle buffer is sized for the
run's actual targets. Plus AddressSanitizer enabled on every C fixture in CI.

**Acceptance.** The property test covers every record kind in the stream and fails
on a deliberately widened or narrowed box; each of the four known defects is
reproduced by the test against the pre-fix code; the C fixture jobs run under ASan
with no suppressions beyond documented third-party ones.

A partial down payment landed with the tight-bin-boxes PR:
[`draw_record_binning_gpu.rs`](../../tools/frame-replay/tests/draw_record_binning_gpu.rs)
renders each sprite, triangle and bitmap case twice — once through the tight box and
once through the emitter's whole-target bounds with binning off — and asserts the
readbacks are identical, with a mutation check (`an_undersized_box_is_caught`) that
shrinks every derived box by one pixel and requires the comparison to fail. It is
enumerated rather than randomised and does not cover every kind, so the property test
above still stands; the two fixture hooks it uses, `tight_record_boxes` and
`erode_record_boxes`, are what a randomised version would build on.

**Size.** M. **Dependencies.** None; shares the fixture matrix with item 6.

## 6. CI structure

**Observed.** Four fixture suites had never run in CI before PR #23 because they
sat behind earlier steps in one sequential job; a GCC-only warning surfaced only
on the remote runner after a clean local build; and adding one field to
`KfxWgpuNativeResource` required edits in eleven emitters.

**Deliverable.** Split [frame replay](../../.github/workflows/frame-replay.yml)
into a job matrix, one entry per fixture suite, so a failure in one suite cannot
hide the others. Add a local container or script that builds the C fixtures with
the CI's GCC flags. Add an initialization helper for `KfxWgpuNativeResource` so a
new ABI field can be added without touching every emitter.

**Acceptance.** Every fixture suite reports its own CI status and a deliberate
failure in one leaves the others reported; the local script reproduces a known
GCC-only warning that AppleClang does not emit; adding a field to the resource
struct touches the helper and its users' intent, not all eleven call sites.

**Size.** M. **Dependencies.** None.

## 7. Documentation hygiene

**Observed.** Reviewers found three passages contradicting the code in one day,
and single PRs were updating status text in three documents at once.

**Deliverable.** The coverage ledger and the acceptance counters are the single
source of truth for status; each PR updates one status section, not three
documents; a periodic check confirms that documented counters, environment
variables and file paths still exist in the code.

**Acceptance.** A PR that changes drawing status touches one status section; the
check reports zero stale counter names, environment variables or paths.

**Size.** S. **Dependencies.** None.

## 8. Parity policy

**Observed.** Carve-outs from exact parity were being argued per PR — uninitialized
triangle steps, shadow scratch contents written by unrelated `big_scratch` users —
with no recorded rule to argue against.

**Deliverable.** The policy is recorded in the renderer design under
[Decisions](../architecture/wgpu-single-stream-renderer.md#decisions): exact
indexed-byte parity remains the acceptance for the port, with three carve-outs for
undefined legacy behavior, deliberately new features behind profile flags, and the
post-retirement regression baseline.

**Acceptance.** Reviews cite the decision rather than re-deriving it; a deviation
is either a named carve-out or a defect.

**Size.** S. **Dependencies.** None.

## Recommended order

1. **Item 2, offscreen measurement.** Every other measurement is blocked by the
   display, so unblocking capture pays for itself immediately.
2. **Item 1, GPU time attribution.** The next optimization decisions depend on
   knowing which pass actually costs the time.
3. **Item 5, bounds-superset property test.** Cheap, independent, and it closes
   the defect class that cost the most review time.
4. **Item 4, capture and offline replay.** It turns the frame cost model into a
   CI artifact, and items 1 and 2 are its prerequisites.
5. **Item 3, deterministic scene mode.** It restores whole-frame A/B comparison,
   valuable but not blocking the 1080p work.
6. **Item 6, CI structure.** Needed before the fixture count grows further, and
   the matrix is easier once item 5 defines the suites.
7. **Item 7, documentation hygiene.** Ongoing discipline rather than a blocker,
   so it lands alongside whatever else is in flight.
