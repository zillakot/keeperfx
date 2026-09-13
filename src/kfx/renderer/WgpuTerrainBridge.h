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
struct KfxWgpuNativeResource {
    const uint8_t* bytes;
    size_t length;
    uint32_t width, height, pitch;
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
#include <vector>
#include "kfx/renderer/WgpuDraw.h"

class WgpuTerrainBridge {
public:
    struct Counters {
        uint64_t gpu_spans = 0, gpu_pixels = 0, cpu_gpoly_spans = 0;
        uint64_t bridge_readbacks = 0, gpu_readback_bytes = 0, native_copy_bytes = 0;
        uint64_t resource_snapshot_bytes = 0, target_creations = 0, failures = 0;
        uint64_t gpu_batches = 0, bridge_initial_index_bytes = 0, cpu_replayed_spans = 0;
        uint64_t verified_batches = 0, verification_cpu_spans = 0;
        uint64_t resident_sequences = 0, resident_batches = 0, cpu_barriers = 0, target_alias_barriers = 0;
        uint64_t barrier_readbacks = 0, verification_readbacks = 0, invalid_frames = 0, missing_cpu_barriers = 0;
        uint64_t transition_checkpoint_bytes = 0, transition_snapshot_copy_bytes = 0, transition_commands = 0;
        uint64_t native_commands = 0, verification_cpu_commands = 0, gpu_sprite_commands = 0;
        uint64_t gpu_ordered_sprites = 0;
        uint64_t gpu_shadow_commands = 0, shadow_scratch_upload_bytes = 0, shadow_scratch_readback_bytes = 0, shadow_scratch_copy_bytes = 0;
        uint64_t gpu_triangles = 0, cpu_triangles = 0, replayed_triangles = 0, verified_triangles = 0, rejected_triangles = 0;
    };
    WgpuTerrainBridge(uint64_t fail_after, bool fail_init, bool verify = false, bool resident = false);
    ~WgpuTerrainBridge();
    WgpuTerrainBridge(const WgpuTerrainBridge&) = delete;
    WgpuTerrainBridge& operator=(const WgpuTerrainBridge&) = delete;
    void Boundary(bool allow_terrain);
    void Flush();
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
    uint64_t ResidentTarget(const KfxGpolyTarget& target);
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
        const void* key = nullptr;
        uint64_t generation = 0;
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
    uint64_t ResourceFor(std::vector<Resource>& cache, const void* key, uint64_t generation,
        const uint8_t* bytes, size_t length, uint32_t width, uint32_t height, uint32_t pitch,
        size_t limit);
    int Fail(const char* reason);
    void ReplayPending();
    bool RasterizePending(uint8_t* pixels, uint32_t pitch) const;
    bool PrepareNativeTarget();
    bool ExecutePending(KfxWgpuNativeOracle oracle = nullptr, void* oracle_context = nullptr);
    bool SameTarget(const KfxGpolyTarget& target) const;
    bool FrameView(const KfxGpolyTarget& target, uint32_t& x, uint32_t& y) const;
    uint64_t SubmissionTarget();
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
    void* m_context = nullptr;
    uint64_t m_target = 0;
    uint32_t m_width = 0, m_height = 0;
    uint64_t m_fail_after;
    bool m_allow_terrain = false;
    KfxGpolyTarget m_native_target = {};
    std::vector<KfxWgpuDrawCommand> m_pending;
    std::vector<KfxWgpuTriangle> m_triangles;
    KfxGpolyRasterizer m_rasterizer = nullptr;
    bool m_fail_init, m_verify, m_failed = false;
    std::array<char, 1024> m_error = {};
    std::vector<Resource> m_textures, m_fades;
    std::vector<uint8_t> m_readback;
    Counters m_counts;
};
#endif
