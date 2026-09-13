# Primitive drawing oracle

`fixture.c` includes the actual legacy primitive implementation. Its synchronous
submission sink records each immutable GPU command and invokes the guarded legacy
oracle on a separate indexed target. AddressSanitizer checks both the generator
and the CPU drawing calls. The asymmetric GlassMap distinguishes axis order and
repeated translucent hits in circle octants.

Generate `primitives.bin` with CMake/CTest, then set `KFX_PRIMITIVE_FIXTURE` to its
absolute path and run the ignored `gpu_actual_legacy_primitives` Rust library
test. A missing fixture fails the test. CI generates it before the GPU suite.

Cases cover pixels (including unclipped row wrapping), filled/outline boxes,
HV lines, filled/outline circles, all four transparency flag combinations,
clipping, negative/zero radii, reversed and degenerate endpoints. Reversed
vertical lines preserve the legacy x-derived y coordinates; fixtures use
coordinates whose resulting writes remain within the allocated target.

These fixtures prove exact GPU palette-index output for these primitive paths.
General striped-line coverage, sprites, fonts, images and effects remain separate
migration work. No benchmark or complete-frame GPU claim follows from this test.
