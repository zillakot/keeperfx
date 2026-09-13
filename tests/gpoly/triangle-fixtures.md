# GPU triangle preparation fixtures

Build and export the native oracle, then compare GPU setup and indices:

```sh
cmake -S tests/gpoly -B /tmp/kfx-gpoly -DKFX_GPOLY_ASAN=ON
cmake --build /tmp/kfx-gpoly
ctest --test-dir /tmp/kfx-gpoly --output-on-failure
KFX_GPOLY_TRIANGLE_FIXTURE=/tmp/kfx-gpoly/triangles.bin cargo test --manifest-path tools/frame-replay/Cargo.toml --test gpoly_gpu -- --ignored --nocapture
```

The GPU test requires a real adapter. Its ignored status keeps ordinary CPU-only CI usable; absence of an adapter when explicitly invoked fails the test. No game assets are used.

`triangle_fixture.c` includes the native gpoly implementation through the existing harness. It records original unsorted vertices, expected native spans and expected native pixels for 1,221 triangles. Cases include all vertex orderings, flat tops/bottoms, repeated/collinear vertices, zero height, clipping at every viewport edge, the 16,383/16,384 edge boundary, long edges using reciprocal division, shade transitions, signed texture wrapping, fractional original attributes and 1,200 deterministic randomized triangles. Table identities are checked against every native reciprocal/slope-table entry.

The Rust API `gpoly::GpolyPreparer::encode` copies immutable vertices and records a compute pass into the caller's encoder. Each vertex has integer `i32` X/Y and original signed 16.16 `i64` U/V/shade. The shader sorts vertices, truncates attributes as the native renderer does, validates edge deltas, computes slopes/interpolation and performs clipped scan conversion. Explicit 64-bit products and 96-bit accumulation preserve native wrapping and carry. The output stays on GPU; callers submit the encoder and consume the rows in subsequent passes.

Coordinates must fit [-32,768, 32,767], and viewport dimensions must be in [1, 32,767]. Empty batches and device storage/dispatch limit violations return errors before commands are encoded. Invalid native triangles produce empty rows. `rows` contains eight `u32` words per triangle/viewport row: X, Y, count, zero, start low, start high, step low, step high. Row index is `triangle * height + y`; count zero denotes no coverage. Resources and ordering are caller responsibilities. Preparation does not validate shade lookup ranges or submit/present pixels; callers must preserve safe fallback for unsupported inputs and invalid resource access.

The test pixel shader consumes GPU rows directly and writes a separate image per triangle. Setup readback is only a differential assertion and is never the pixel shader's input. It verifies every row word and every palette index, including target pitch padding. This establishes isolated triangle correctness, not live integration, overlap ordering, other drawing modes, device-failure recovery or performance.

## Binary format

All integer words are little endian. `KFXGTRI1` magic is followed by width, height, pitch and triangle count (`u32` each), 7,968 synthetic texture bytes and 16,384 fade bytes. Each triangle contains three vertices (X/Y words followed by U/V/shade low/high word pairs), `height` expected eight-word rows, and `pitch * height` expected native pixel bytes. Empty expected rows are zero. Initial pixels are palette index 167. Files are generated artifacts and are not committed.
