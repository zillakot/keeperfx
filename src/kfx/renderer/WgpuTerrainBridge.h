#pragma once
#include "kfx/renderer/GpolyCapture.h"
#include "kfx/renderer/WgpuDraw.h"

#ifdef __cplusplus
extern "C" {
#endif
void kfx_wgpu_terrain_boundary(int allow_terrain);
int kfx_wgpu_native_enabled(void);
int kfx_wgpu_native_cpu_barrier(void);
int kfx_wgpu_native_read_barrier(const void* bytes, size_t length);
void kfx_wgpu_native_invalidate_frame(void);
void kfx_wgpu_native_flush(void);
void* kfx_wgpu_native_context(void);
void kfx_wgpu_native_context_cleanup(void (*cleanup)(void*));
uint64_t kfx_wgpu_native_target(const struct KfxGpolyTarget* target);
/* tail is an optional second immutable segment appended to bytes, so an emitter
 * can name two stable buffers instead of copying them into one. */
struct KfxWgpuNativeResource {
    const uint8_t* bytes;
    size_t length;
    uint32_t width, height, pitch;
    const uint8_t* tail;
    size_t tail_length;
    int cursor;
};
typedef void (*KfxWgpuNativeOracle)(uint8_t* pixels, uint32_t pitch, void* context);
uint64_t kfx_wgpu_native_snapshot(const struct KfxGpolyTarget* target,
    uint32_t width, uint32_t height, uint32_t pitch, uint8_t* checkpoint);
void kfx_wgpu_native_snapshot_release(uint64_t snapshot);
int kfx_wgpu_native_draw(const struct KfxGpolyTarget* target,
    const struct KfxWgpuDrawCommand* command, const struct KfxWgpuNativeResource* source,
    const struct KfxWgpuNativeResource* table, KfxWgpuNativeOracle oracle, void* oracle_context);
#ifdef __cplusplus
}
#endif

#if defined(KFX_RUST_PRESENTER) && defined(__cplusplus)
#include "kfx/renderer/GpolyCapture.h"
#include <array>
#include <cstdint>
#include <map>
#include <vector>
#include "kfx/renderer/WgpuDraw.h"

class WgpuTerrainBridge {
public:
    struct Counters {
        uint64_t gpu_spans = 0, gpu_pixels = 0, cpu_gpoly_spans = 0;
        uint64_t bridge_readbacks = 0, gpu_readback_bytes = 0, native_copy_bytes = 0;
        uint64_t resource_snapshot_bytes = 0, target_creations = 0, failures = 0;
        uint64_t gpu_batches = 0, bridge_initial_index_bytes = 0, cpu_replayed_spans = 0;
        uint64_t verified_batches = 0, verification_cpu_spans = 0, verification_flagged_shades = 0;
        uint64_t resident_sequences = 0, resident_batches = 0, cpu_barriers = 0, target_alias_barriers = 0;
        uint64_t barrier_readbacks = 0, verification_readbacks = 0, invalid_frames = 0, missing_cpu_barriers = 0;
        uint64_t transition_checkpoint_bytes = 0, transition_snapshot_copy_bytes = 0, transition_commands = 0;
        uint64_t native_commands = 0, verification_cpu_commands = 0, gpu_sprite_commands = 0;
        uint64_t gpu_ordered_sprites = 0;
        uint64_t gpu_shadow_commands = 0, shadow_scratch_upload_bytes = 0, shadow_scratch_readback_bytes = 0, shadow_scratch_copy_bytes = 0;
        uint64_t shadow_prior_divergence = 0;
        uint64_t gpu_triangles = 0, cpu_triangles = 0, replayed_triangles = 0, verified_triangles = 0, rejected_triangles = 0;
        uint64_t bridge_solo_batches = 0, bridge_target_flushes = 0, bridge_target_runs = 0;
        uint64_t rejected_commands = 0, rejected_spans = 0;
        uint64_t resource_purge_failures = 0;
    };
    WgpuTerrainBridge(uint64_t fail_after, bool fail_init, bool verify = false, bool resident = false);
    ~WgpuTerrainBridge();
    WgpuTerrainBridge(const WgpuTerrainBridge&) = delete;
    WgpuTerrainBridge& operator=(const WgpuTerrainBridge&) = delete;
    void Boundary(bool allow_terrain);
    void Flush();
    // Ordering boundary at an emitter head; the pending record list already orders a live frame.
    void EmitterBoundary();
    // CPU pixels are unavailable inside a resident lease until this succeeds.
    bool CpuBarrier();
    bool ReadBarrier(const void* bytes, size_t length);
    void BeginResident();
    bool BeginFrame(const KfxGpolyTarget& target, bool discard = false);
    bool EndFrame(bool materialize);
    bool OwnsFrame(const KfxGpolyTarget& target) const;
    void InvalidateFrame();
    bool FrameValid() const { return !m_frame_invalid; }
    // Call only after a successful full CPU overwrite, before the next frame draws.
    void FullRedraw();
    // Detach before destroying a borrowed presenter.
    bool AttachPresenter(void* presenter);
    void DetachPresenter();
    uint64_t ResidentTarget(const KfxGpolyTarget& target, uint64_t* replay_ns = nullptr);
    uint64_t BorrowTarget(const KfxGpolyTarget& target);
    void* Context() const { return m_context; }
    void* CursorContext() const { return m_verify ? nullptr : m_context; }
    bool UsesPresenter() const { return m_borrowed_context; }
    uint64_t Snapshot(const KfxGpolyTarget& target, uint32_t width, uint32_t height,
        uint32_t pitch, uint8_t* checkpoint);
    void ReleaseSnapshot(uint64_t snapshot);
    int SubmitNative(const KfxGpolyTarget& target, const KfxWgpuDrawCommand& command,
        const KfxWgpuNativeResource* source, const KfxWgpuNativeResource* table,
        KfxWgpuNativeOracle oracle, void* oracle_context);
    int SubmitShadow(const KfxGpolyTarget& target, const KfxWgpuDrawCommand& command,
        const KfxWgpuNativeResource* source, const KfxWgpuNativeResource* table, uint8_t* scratch,
        KfxWgpuNativeOracle oracle, void* oracle_context);
    // The single C-side mirror of the Rust packer whitelist in tools/frame-replay/src/draw.rs.
    static bool PacksInBatch(uint32_t kind);
    const Counters& GetCounters() const { return m_counts; }
    KfxWgpuDrawCounters GetGpuCounters() const;
    const char* GetError() const { return m_error.data(); }
    bool Failed() const { return m_failed; }
    bool IsOracleActive() const { return m_oracle_active; }

private:
    /* Bytes the 32x32 gpoly tile read actually touches at pitch 256. */
    static constexpr size_t TEXTURE_READ_BYTES = 31 * 256 + 32;
    struct Resource {
        uint64_t handle;
        std::vector<uint8_t> bytes;
        uint32_t width, height, pitch;
    };
    /* Extent of one asset: a key names one byte range of one extent for one generation,
       so a memo that matched only the pointers could hand back another shape's handle. */
    struct Extent {
        size_t length;
        uint32_t width, height, pitch;
        bool operator==(const Extent&) const = default;
    };
    /* The handle a key resolved to last, so a run of spans over one page does not cross
       the ABI again. Valid only while that handle lives: a generation bump supersedes the
       handle and changes the memo's own generation, and a purge clears every memo. */
    struct KeyMemo {
        const void* key;
        const void* tail_key;
        uint64_t generation;
        Extent extent;
        uint64_t handle;
    };
    static int Sink(void* context, const KfxGpolyTarget* target,
        const KfxGpolySpan* span, const uint8_t* texture, const uint8_t* fade);
    static int TriangleSink(void*, const KfxGpolyTarget*, const KfxWgpuTriangle*,
        const uint8_t*, const uint8_t*, KfxGpolyRasterizer);
    int DrawTriangle(const KfxGpolyTarget&, const KfxWgpuTriangle&,
        const uint8_t*, const uint8_t*, KfxGpolyRasterizer);
    int Draw(const KfxGpolyTarget& target, const KfxGpolySpan& span,
        const uint8_t* texture, const uint8_t* fade);
    static const void* StableKey(const void* bytes, size_t length);
    /* Resolves an asset to a handle the drawing context keeps resident for the key.
       A null key has no name the context can trust, so it takes a per-call handle. */
    uint64_t KeyedResource(uint32_t kind, const void* key, const void* tail_key,
        uint64_t generation, const uint8_t* bytes, const Extent& extent);
    uint64_t TerrainResource(uint32_t kind, KeyMemo& memo, const void* key,
        const uint8_t* bytes, const Extent& extent);
    uint64_t MemoHandle(const KeyMemo& memo, const void* key, const void* tail_key,
        const Extent& extent) const;
    uint64_t TextureResource(const uint8_t* texture);
    uint64_t FadeResource(const uint8_t* fade);
    uint64_t TableResource(const KfxWgpuNativeResource& table, size_t limit);
    // Releases handles no pending command can name any more; false if a release failed.
    bool CollectSuperseded();
    const uint8_t* ReplayAsset(uint64_t handle) const;
    // False when the drawing context refused a release, which leaves residency unproven.
    bool PurgeResources();
    int Fail(const char* reason);
    // Marks the frame for the full CPU redraw RendererSoftware performs on an invalid frame.
    void Invalidate();
    // Kinds the Rust packer whitelist and submit routing accept only as a single-command batch.
    static bool NeedsSoloBatch(const KfxWgpuDrawCommand& command);
    // Only terrain spans and triangles have a CPU rasterizer the bridge can replay.
    bool PendingIsReplayable() const;
    static bool OrderedSprite(const KfxWgpuDrawCommand& command);
    bool PendingTargetChanged(const KfxGpolyTarget& target) const;
    static bool SameRun(const KfxGpolyTarget& a, const KfxGpolyTarget& b);
    void AppendCommand(const KfxWgpuDrawCommand& command, uint64_t source);
    void AppendTriangle(const KfxWgpuTriangle& triangle);
    // Releases owned per-command sources, then drops every pending record.
    void ClearPending();
    // Clears the pending run and counts what the target never received.
    void DiscardPending();
    // True when the pending run was rasterized onto the target; false leaves it untouched.
    bool ReplayPending();
    bool RasterizePending(uint8_t* pixels, uint32_t pitch) const;
    bool PrepareNativeTarget();
    bool ExecutePending(KfxWgpuNativeOracle oracle = nullptr, void* oracle_context = nullptr);
    bool SameTarget(const KfxGpolyTarget& target) const;
    bool FrameView(const KfxGpolyTarget& target, uint32_t& x, uint32_t& y) const;
    uint64_t SubmissionTarget(const KfxGpolyTarget& native);
    void ReleaseViews();
    size_t ExpectedOffset() const;
    bool Materialize();
    bool ValidateCpuLease();
    bool m_resident_enabled = false, m_resident_lease = false, m_gpu_valid = false, m_gpu_dirty = false;
    bool m_frame_invalid = false, m_borrowed_context = false;
    KfxGpolyTarget m_gpu_native_target = {};
    KfxGpolyTarget m_frame_target = {};
    bool m_frame_active = false, m_queue_active = false, m_discard_initial = false;
    struct View { uint32_t x, y, width, height; uint64_t handle; };
    std::vector<View> m_views;
    std::vector<uint8_t> m_expected, m_cpu_checkpoint;
    bool m_oracle_active = false;
    uint8_t* m_shadow_scratch = nullptr;
    // Verification only: the resident GPU scratch as of the last comparison.
    std::vector<uint8_t> m_shadow_prior = std::vector<uint8_t>(65536, 0);
    void* m_context = nullptr;
    uint64_t m_target = 0;
    uint32_t m_width = 0, m_height = 0;
    uint64_t m_fail_after;
    bool m_allow_terrain = false;
    KfxGpolyTarget m_native_target = {};
    // One run per contiguous same-route stretch; the three vectors are read in run order.
    enum RunKind : uint8_t { kRunCommands, kRunTriangles, kRunShadow };
    struct PendingRun { RunKind kind; uint32_t count; KfxGpolyTarget target; };
    std::vector<KfxWgpuDrawCommand> m_pending;
    std::vector<uint64_t> m_pending_sources;
    std::vector<KfxWgpuTriangle> m_triangles;
    std::vector<PendingRun> m_order;
    static constexpr size_t kPendingLimit = 4096;
    static constexpr size_t kPendingSpanLimit = 32768;
    KfxGpolyRasterizer m_rasterizer = nullptr;
    bool m_fail_init, m_verify, m_failed = false;
    std::array<char, 1024> m_error = {};
    // Lookup tables with no identity to key on; the only cache left that compares content.
    std::vector<Resource> m_native_tables;
    std::vector<uint64_t> m_superseded;
    /* Terrain bytes the CPU replay rasterizes, one entry per live terrain handle. Taken
       when the handle is created and dropped when it is released, so resolving a resident
       key copies nothing; identity lives in the key, these are only what a replay needs.
       They also hold the bytes the caller passed, so a mutation behind an unchanged key
       shows up as a KFX_WGPU_DRAW_VERIFY comparison failure. */
    std::map<uint64_t, std::vector<uint8_t>> m_replay_assets;
    KeyMemo m_texture_memo = {}, m_fade_memo = {}, m_table_memo = {};
    std::vector<uint8_t> m_readback;
    Counters m_counts;
};
#endif
