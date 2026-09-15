#ifndef KFX_UPLOAD_COUNTERS_H
#define KFX_UPLOAD_COUNTERS_H

#define KFX_UPLOAD_FIELDS(X) \
    X(upload_queue_writes) \
    X(upload_queue_bytes) \
    X(upload_queued_bytes) \
    X(upload_ring_overflows) \
    X(upload_overflow_bytes) \
    X(upload_oversized_frames) \
    X(upload_padding_bytes) \
    X(upload_records_capacity) \
    X(upload_records_used) \
    X(upload_records_high_water) \
    X(upload_indices_capacity) \
    X(upload_indices_used) \
    X(upload_indices_high_water) \
    X(upload_uniforms_capacity) \
    X(upload_uniforms_used) \
    X(upload_uniforms_high_water) \
    X(upload_arena_dirty_bytes) \
    X(capture_schema) \
    X(arena_representation) \
    X(upload_transport) \
    X(arena_source_bytes) \
    X(arena_logical_upload_bytes) \
    X(arena_transfer_bytes) \
    X(upload_cpu_copy_bytes) \
    X(upload_cpu_copy_ns) \
    X(upload_expand_ns) \
    X(upload_staging_copy_ns) \
    X(upload_api_ns) \
    X(upload_copy_encode_ns) \
    X(upload_copy_commands) \
    X(upload_copy_bytes) \
    X(upload_init_bytes) \
    X(staging_buffers_created) \
    X(staging_capacity_bytes) \
    X(staging_inflight_bytes) \
    X(staging_peak_bytes) \
    X(staging_map_wait_ns) \
    X(staging_fallbacks) \
    X(staging_padding_bytes) \
    X(snapshot_copy_bytes) \
    X(snapshot_pack_bytes)

#define KFX_UPLOAD_ROUTES(X) \
    X(record_ring) \
    X(index_ring) \
    X(uniform_ring) \
    X(immutable_ordered_commands) \
    X(ordered_tile_lists) \
    X(drawing_dimensions) \
    X(ordered_sprite_commands) \
    X(sprite_target_dimensions) \
    X(ordered_sprite_layer) \
    X(minimap_target_view) \
    X(shadow_arena_region) \
    X(snapshot_triangle_commands) \
    X(snapshot_triangle_tiles) \
    X(snapshot_image_commands) \
    X(snapshot_image_tile_lists) \
    X(immutable_gpoly_vertices) \
    X(gpoly_viewport) \
    X(gpoly_row_layout) \
    X(arena_assets) \
    X(snapshot_tables) \
    X(arena_flush) \
    X(immutable_asset_versions) \
    X(triangle_immutable_assets) \
    X(snapshot_triangle_fallback_assets) \
    X(immutable_lens_sources_and_maps) \
    X(gpu_snapshot_sampling_arena) \
    X(ordered_sprite_identity_layer) \
    X(sprite_artwork_and_run_boundaries) \
    X(minimap_semantic_cells_and_styles) \
    X(immutable_shadow_artwork) \
    X(persistent_asset_arena) \
    X(effect_target_view) \
    X(compatibility)

#define KFX_UPLOAD_ROUTE_FIELDS(route) \
    KFX_UPLOAD_FIELD(upload_##route##_creates) \
    KFX_UPLOAD_FIELD(upload_##route##_create_bytes) \
    KFX_UPLOAD_FIELD(upload_##route##_reservations) \
    KFX_UPLOAD_FIELD(upload_##route##_payload_bytes) \
    KFX_UPLOAD_FIELD(upload_##route##_writes) \
    KFX_UPLOAD_FIELD(upload_##route##_write_bytes)
#define KFX_UPLOAD_ALL_FIELDS \
    KFX_UPLOAD_FIELDS(KFX_UPLOAD_FIELD) \
    KFX_UPLOAD_ROUTES(KFX_UPLOAD_ROUTE_FIELDS)
#define KFX_UPLOAD_GAUGES(X) \
    X(upload_records_capacity) \
    X(upload_records_used) \
    X(upload_records_high_water) \
    X(upload_indices_capacity) \
    X(upload_indices_used) \
    X(upload_indices_high_water) \
    X(upload_uniforms_capacity) \
    X(upload_uniforms_used) \
    X(upload_uniforms_high_water) \
    X(capture_schema) \
    X(arena_representation) \
    X(upload_transport) \
    X(staging_capacity_bytes) \
    X(staging_inflight_bytes) \
    X(staging_peak_bytes)

#define KFX_UPLOAD_COUNTER_COUNT 239

#endif
