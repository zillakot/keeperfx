# Sound registry regression checks

The latest successful named declaration wins, whether it names a built-in ID,
filesystem asset, ZIP asset or sequential family. `getCustomSoundId` still looks
up the loaded custom asset separately. A reused source republishes its named
mapping, so loading that declaration after a numeric override takes effect.

Filesystem reuse compares the resolved path; editing bytes at the same path is
not a hot-reload contract. ZIP reuse compares the source audio bytes exactly;
the manager retains those bytes to distinguish same-name assets from different
map bundles without relying on a hash collision assumption.

A sequential family reuses its buffers only when the resulting IDs are contiguous.
Otherwise it allocates a contiguous range. Every requested variant must load, and
the supported maximum is 32. Missing, invalid or oversized families return failure
and preserve previous registry/cache mappings. This also keeps the existing raw-ID
redirect caller from installing an incomplete range. The family transaction
rolls back newly decoded, unpublished buffers on failure, so repeated invalid
attempts cannot grow the bank. Existing published IDs remain unchanged.

Campaign snapshots restore the active named mappings, variant counts and custom
source cache. The existing caller still restores the bank watermark and raw-ID
redirect snapshot separately. Clearing the custom bank invalidates the manager's
snapshot and removes registry mappings into that bank, while preserving built-in
numeric mappings.

## Run

After configuring the native build dependencies:

```sh
cmake -S . -B out/macos -DKFX_SOUND_REGISTRY_TESTS=ON
cmake --build out/macos --target keeperfx sound_manager_registry_test --parallel 3
ctest --test-dir out/macos --output-on-failure -R '^sound_manager_registry$'
```

The macOS CI job runs this test. It compiles the actual `src/sound_manager.cpp`
with deterministic bank-decoder, filesystem and ZIP boundary doubles. It executes
the production C/C++ registry APIs, source-resolution bridge, sequential expansion
and manager snapshot code; it does not copy their logic into a model.

Coverage includes numeric/custom layering, identical and different sources,
filesystem/ZIP replacement, corrupt overrides, reused/scattered family variants,
single/family/count changes, atomic failed-family publication, oversized families,
campaign restoration and bank reset. The test simulates the separate bank truncation;
it does not execute OpenAL or its buffer truncation, decode real WAV files, parse a complete campaign config,
or establish listening quality or device behavior.

At parent commit `66443002b`, the same regression fixture compiled against the
unchanged production source/header exits with `FAIL: TAB_CLICK` when a numeric
campaign declaration follows the loaded after-base custom cue. With the fix, the
native build, CTest and an AddressSanitizer/UndefinedBehaviorSanitizer run pass.
