#pragma once

#define KFX_ARENA_KINDS(X) \
    X(sprite) \
    X(ordered_sprite) \
    X(cursor) \
    X(trig) \
    X(terrain_tile) \
    X(terrain_fade) \
    X(native_table) \
    X(minimap) \
    X(shadow) \
    X(target_trig_geometry) \
    X(target_trig_table) \
    X(image) \
    X(raw_image) \
    X(tiled_image) \
    X(movie) \
    X(map_view) \
    X(bitmap) \
    X(lens) \
    X(other)

enum { KFX_ARENA_KIND_COUNT = 19 };
struct KfxArenaKindCounters {
    unsigned long long bytes;
    unsigned long long misses;
    unsigned long long hits;
    unsigned long long source_bytes;
    unsigned long long distinct_lengths;
    unsigned long long length_overflows;
};
#ifdef __cplusplus
inline constexpr const char* KfxArenaKindNames[] = {
#define KFX_ARENA_NAME(kind) #kind,
    KFX_ARENA_KINDS(KFX_ARENA_NAME)
#undef KFX_ARENA_NAME
};
static_assert(sizeof(KfxArenaKindNames) / sizeof(*KfxArenaKindNames) == KFX_ARENA_KIND_COUNT);
static_assert(sizeof(KfxArenaKindCounters) == 6 * sizeof(unsigned long long));
#endif
