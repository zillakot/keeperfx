# Native terrain bridge checks

The default asset-free test substitutes the GPU ABI and runs under AddressSanitizer.
It checks ordered batches, immutable resources, CPU interleaving, bounded cache
rotation, target changes, initialization/batch failure, invalid shade rejection,
and partial readback failure without publishing incomplete output.

```sh
cmake -S tests/terrain-bridge -B out/terrain-bridge-tests
cmake --build out/terrain-bridge-tests
ctest --test-dir out/terrain-bridge-tests --output-on-failure
```

On macOS, pass `-DKFX_GPU_LIBRARY=/absolute/path/libkeeperfx_frame_replay.a` from a
`live-surface` Rust build to execute the same ordering/resource/resize/recovery
checks on Metal. Metal requires desktop device access. This mode also compares
every successful GPU batch to the independent indexed span oracle. ABI-injected
partial readback failure remains a mock-only test.

The native build option remains `KFX_RUST_PRESENTER=ON`. Select partial terrain
drawing with `KFX_DRAW_BACKEND=wgpu`; unset or `software` keeps software drawing.
`KFX_PRESENT_BACKEND` independently chooses presentation. This backend retains
CPU span setup, all other drawing families, and explicit full-target CPU/GPU
composition bridges at audited world bucket boundaries.

`KFX_WGPU_DRAW_VERIFY=1` checks every batch against a CPU span oracle before
publishing the GPU result. `KFX_WGPU_DRAW_STATS=/absolute/path.json` writes cumulative
counts at presentation and shutdown. GPU API upload/readback counters measure
actual widened u32 transfers; bridge index counts describe native byte inputs.
`cpu_gpoly_spans` counts declined captured gpoly spans only. It does not count
other CPU rasterizers. `verification_cpu_spans` and `cpu_replayed_spans` distinguish
test oracle work from error recovery. `KFX_WGPU_DRAW_FAIL_INIT=1` injects startup
failure; `KFX_WGPU_DRAW_FAIL_AFTER=N` fails the batch after N successful batches.
These validation modes are unsuitable for performance measurement.

`kfx_wgpu_native_draw` provides the synchronous extension seam for new primitive
families. It flushes earlier terrain, snapshots optional source/table resources,
and returns 1 only after GPU indices have been committed to the native target.
On 0 the caller runs its existing CPU drawing once. Its optional oracle callback
receives a separate output buffer and pitch; it must redirect all destination
aliases and guard recursive submission. Verification mode declines commands
without an oracle. This API intentionally retains full-target synchronization
until command coverage can preserve GPU ownership across native callers.
