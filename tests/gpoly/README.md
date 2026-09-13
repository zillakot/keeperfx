# Gpoly span extraction checks

Build the asset-free native differential suite with SDL3 development headers available:

```sh
cmake -S tests/gpoly -B out/gpoly-tests -DKFX_GPOLY_ASAN=ON
cmake --build out/gpoly-tests
ctest --test-dir out/gpoly-tests --output-on-failure
out/gpoly-tests/gpoly_capture_test out/gpoly-fixture.bin
```

The test compiles the actual legacy gpoly implementation. It compares unobserved
legacy drawing, capture-observed legacy drawing, and an independent two-word
integer replay. It checks target padding and guards, all clipping edges, negative
coordinates, flat and rejected triangles, shade transitions, texture wrapping,
carry and borrow, resource mutation, ordering, bounded capture failure, and
consumed/declined/error sink outcomes. AddressSanitizer checks an exactly sized
7,968-byte atlas view and 16,384-byte fade table. CPU scan conversion remains in
the legacy implementation; this does not test GPU drawing or runtime performance.

The optional binary fixture contains synthetic overlapping terrain with two
versions of each resource. Integers are little-endian `u32`; coordinates use their
signed bit patterns. Layout:

| Data | Layout |
| --- | --- |
| Magic | Eight bytes `KFXGSPN1` |
| Header | width, height, pitch, span count, texture count, fade count |
| Spans | x, y, count, start low, start high, step low, step high, texture index, fade index |
| Textures | 8,192 bytes each; useful 32-byte rows at stride 256, padding zero |
| Fade tables | 16,384 bytes each |
| Initial target | pitch × height bytes |
| Expected legacy target | pitch × height bytes |

The capture sink always declines consumption so legacy drawing proceeds. A live
GPU sink must copy resources synchronously and return consumed only after owning
the command. Calls are serialized with the renderer; sink registration is global.
Capture capacity exhaustion marks the whole capture unusable, while legacy
drawing continues. Replay requires the original initial target contents.
