#!/usr/bin/env bash
# Runs every GPU fixture suite on the Windows runner's software adapter, one suite at
# a time, so a backend difference is attributed to the suite that shows it instead of
# stopping the job at the first failure. Exits non-zero if any suite failed.
#
# A row is `pass` only when the suite exited 0, reported a non-zero passed count and
# printed no stand-down. A suite that exits 0 having run nothing is `skip`, which is a
# gap in the matrix rather than evidence of a working backend, so it is annotated and
# fails the job: a leg that proved nothing about its backend must not report success.
set -u

: "${ASSET_FLAGS:=--no-default-features --features packed-arena}"

results=out/windows-suites.md
logs=out/windows-suite-logs
mkdir -p "$logs"
: > "$results"
status=0

row() {
  echo "| \`$1\` | $2 | $3 |" >> "$results"
}

# A stray pipe in a wgpu error string would otherwise break the table row.
cell() {
  printf '%s' "$1" | tr -d '\r' | sed 's/|/\\|/g'
}

suite() {
  local name=$1
  shift
  local log="$logs/$name.log"
  local code=0
  echo "::group::$name"
  # Streamed rather than captured and dumped, so a slow suite shows progress.
  "$@" 2>&1 | tee "$log"
  code=${PIPESTATUS[0]}
  echo "::endgroup::"
  local passed
  passed=$(grep -oE 'test result: ok\. [0-9]+ passed' "$log" |
    grep -oE '[0-9]+ passed' | grep -oE '[0-9]+' | awk '{total += $1} END {print total + 0}')
  # Anchored to the guards' own format, so build chatter cannot reclassify a suite.
  local stood_down
  stood_down=$(grep -m 1 -E '^skipping[ :]' "$log")
  if [ "$code" -ne 0 ]; then
    row "$name" fail "exited $code; see the \`$name\` group in this job's log"
    status=1
  elif [ -n "$stood_down" ]; then
    echo "::error::$name stood down: $stood_down"
    row "$name" skip "$(cell "$stood_down")"
    status=1
  elif [ "$passed" -eq 0 ]; then
    echo "::error::$name exited 0 without running a test"
    row "$name" skip "exited 0 without running a test"
    status=1
  else
    row "$name" pass "$passed passed"
  fi
}

# shellcheck disable=SC2329 # invoked through `suite`
replay() {
  # shellcheck disable=SC2086
  cargo test $ASSET_FLAGS --locked --manifest-path tools/frame-replay/Cargo.toml "$@"
}

echo "| Suite | DX12 software adapter | Note |" >> "$results"
echo "| --- | --- | --- |" >> "$results"

suite lib replay --lib -- --ignored --nocapture --test-threads=1
for name in \
  gpoly_gpu \
  draw_triangles_gpu \
  draw_trig_gpu \
  draw_shadow_gpu \
  draw_sprite_layers_gpu \
  draw_sprite_interning_gpu \
  draw_bitmap_gpu \
  draw_minimap_gpu \
  draw_map_view_gpu \
  draw_transition_gpu \
  draw_movie_gpu \
  draw_raw_gpu \
  draw_lenses_gpu \
  draw_target_resources_gpu \
  draw_views_gpu \
  draw_frame_order_gpu \
  draw_terrain_binning_gpu \
  draw_record_binning_gpu \
  draw_asset_bytes_gpu \
  draw_one_submit_gpu \
  draw_limits_gpu \
  draw_status_gpu; do
  suite "$name" replay --test "$name" -- --ignored --nocapture --test-threads=1
done
export KFX_WGPU_GPU_TIMING=2
suite draw_replay_host_gpu replay --test draw_replay_host_gpu -- --ignored --nocapture --test-threads=1
unset KFX_WGPU_GPU_TIMING

echo
cat "$results"
exit $status
