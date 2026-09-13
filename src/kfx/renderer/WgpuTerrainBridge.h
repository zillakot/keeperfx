#pragma once
#include "kfx/renderer/GpolyCapture.h"
#include "kfx/renderer/WgpuDraw.h"

#ifdef __cplusplus
extern "C" {
#endif
void kfx_wgpu_terrain_boundary(int allow_terrain);
int kfx_wgpu_native_enabled(void);
struct KfxWgpuNativeResource {
    const uint8_t* bytes;
    size_t length;
    uint32_t width, height, pitch;
};
typedef void (*KfxWgpuNativeOracle)(uint8_t* pixels, uint32_t pitch, void* context);
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
        uint64_t native_commands = 0, verification_cpu_commands = 0;
        uint64_t gpu_triangles = 0, cpu_triangles = 0, replayed_triangles = 0, verified_triangles = 0, rejected_triangles = 0;
    };
    WgpuTerrainBridge(uint64_t fail_after, bool fail_init, bool verify = false);
    ~WgpuTerrainBridge();
    WgpuTerrainBridge(const WgpuTerrainBridge&) = delete;
    WgpuTerrainBridge& operator=(const WgpuTerrainBridge&) = delete;
    void Boundary(bool allow_terrain);
    void Flush();
    int SubmitNative(const KfxGpolyTarget& target, const KfxWgpuDrawCommand& command,
        const KfxWgpuNativeResource* source, const KfxWgpuNativeResource* table,
        KfxWgpuNativeOracle oracle, void* oracle_context);
    const Counters& GetCounters() const { return m_counts; }
    KfxWgpuDrawCounters GetGpuCounters() const;
    const char* GetError() const { return m_error.data(); }
    bool Failed() const { return m_failed; }

private:
    struct Resource {
        uint64_t handle;
        std::vector<uint8_t> bytes;
    };
    static int Sink(void* context, const KfxGpolyTarget* target,
        const KfxGpolySpan* span, const uint8_t* texture, const uint8_t* fade);
    static int TriangleSink(void*, const KfxGpolyTarget*, const KfxWgpuTriangle*,
        const uint8_t*, const uint8_t*, KfxGpolyRasterizer);
    int DrawTriangle(const KfxGpolyTarget&, const KfxWgpuTriangle&,
        const uint8_t*, const uint8_t*, KfxGpolyRasterizer);
    int Draw(const KfxGpolyTarget& target, const KfxGpolySpan& span,
        const uint8_t* texture, const uint8_t* fade);
    uint64_t ResourceFor(std::vector<Resource>& cache, const uint8_t* bytes,
        size_t length, uint32_t width, uint32_t height, uint32_t pitch, size_t limit);
    int Fail(const char* reason);
    void ReplayPending();
    bool RasterizePending(uint8_t* pixels, uint32_t pitch) const;
    bool ExecutePending(KfxWgpuNativeOracle oracle = nullptr, void* oracle_context = nullptr);
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
