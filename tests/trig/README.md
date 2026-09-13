# General triangle oracle

`fixture.c` includes the actual legacy `bflib_render_trig.c`, with GPU interception disabled. CTest writes `trig.bin`: original vertices, immutable generated texture/fade/ghost assets and cumulative native output. No proprietary assets are used. AddressSanitizer checks row padding and allocation guards.

```sh
cmake -S tests/trig -B out/trig-tests
cmake --build out/trig-tests
ctest --test-dir out/trig-tests --output-on-failure
KFX_TRIG_FIXTURE="$PWD/out/trig-tests/trig.bin" cargo test --manifest-path tools/frame-replay/Cargo.toml --test draw_trig_gpu -- --ignored --nocapture
```

The 3,382 cases cover modes 0–4, 7, 8, 10–19, 22 and 23: four edge configurations, six input orders, all clipping edges, degenerate/reversed input, negative and wrapped texture coordinates, fractional attributes, full shade range, palette composition and overlapping ordered batches. Additional GPU checks reject invalid shades, undefined native increments and released resources before target writes, including mixed ordered-sprite/triangle batches.

The kernel preserves native LP64 arithmetic, including the non-truncating `__ROL4__` helper and 64-bit `__CFADDL__` carry. GPU input coordinates/extents are limited to the native ±32767 domain; attributes are limited to ±0x04000000 fixed-point units. General modes 5, 6, 9, 20, 21 and 24–26 and native 32-bit-long arithmetic remain unimplemented.

The generic command source contains three little-endian x/y/u/v/shade i32 vertices followed by 0, 7,968, 8,192 or 65,536 texture bytes. `source_x` is the mode, `source_y` is texture byte length, and `source_width` is 64. The table contains 16,384 fade bytes followed by 65,536 ghost bytes. Resource reads and horizontal increment validity are checked on GPU before any batch writes. Accepted drawing performs its own setup directly from those original vertices.

The native adapter snapshots the existing terrain-sized 7,968-byte texture extent. GPU lookup validation rejects samples beyond that snapshot. Mode10 is GPU-tested with synthetic mask assets but deliberately not intercepted natively: native creature shadows still produce `big_scratch` on CPU, which must be replaced with a GPU mask before enabling the mode. General mode7 near-FP subdivisions are intercepted before CPU reorder/setup.

The native LL/RL interpolated setup leaves horizontal increments uninitialized when its integer extent is zero. Example: `(1,0),(2,1),(3,3)` with mode4. This implementation rejects that command atomically; it does not define new native pixels for the existing undefined case. Native fallback behavior is unchanged. Out-of-domain vertices and unsupported modes decline before submission; a failed GPU preflight follows the existing bridge's terminal-failure/CPU fallback path.

On macOS, set `KFX_GPU_LIBRARY` to the live-surface static library to also build `trig_native_bridge`. Its 36 calls test interception, exact Metal output, native oracle verification and target guards for all18 native-enabled modes. The bridge remains transitional: CPU initial-index upload and GPU readback occur at composition boundaries.
