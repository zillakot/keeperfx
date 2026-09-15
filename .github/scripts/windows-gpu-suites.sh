#!/usr/bin/env bash
# Runs every GPU fixture suite on the Windows runner's software adapter, one suite at
# a time, so a backend difference is attributed to the suite that shows it instead of
# stopping the job at the first failure. Exits non-zero if any suite failed.
set -u

results=out/windows-suites.md
mkdir -p out
: > "$results"
status=0

suite() {
  local name=$1
  shift
  echo "::group::$name"
  if "$@"; then
    echo "| \`$name\` | pass | |" >> "$results"
  else
    echo "| \`$name\` | fail | see the \`$name\` group in this job's log |" >> "$results"
    status=1
  fi
  echo "::endgroup::"
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
