#include "kfx/renderer/WgpuTerrainBridge.h"
#include "kfx/renderer/WgpuShadow.h"
#include "kfx/renderer/WgpuTargetResource.h"
#include "kfx/renderer/KfxWgpuFrame.h"
#ifdef KFX_RUST_PRESENTER
#include "kfx/renderer/WgpuDraw.h"
#include <chrono>
#include <cstdio>
#include <cstring>
#include <exception>
#include <limits>

namespace {
struct OwnedResource {
    void* context = nullptr;
    uint64_t handle = 0;
    ~OwnedResource()
    {
        if (context == nullptr || handle == 0) return;
        char error[1024] = {};
        kfx_wgpu_draw_resource_release(context, handle, error, sizeof(error));
    }
    uint64_t Take() { const uint64_t taken = handle; handle = 0; return taken; }
};
}

static WgpuTerrainBridge* active_bridge = nullptr;
static void (*context_cleanup)(void*) = nullptr;
extern "C" void kfx_wgpu_native_context_cleanup(void (*cleanup)(void*)) { context_cleanup = cleanup; }

extern "C" int kfx_wgpu_native_enabled(void)
{ return active_bridge != nullptr && !active_bridge->Failed() && !active_bridge->IsOracleActive(); }

extern "C" int kfx_wgpu_native_cpu_barrier(void)
{ return active_bridge == nullptr || active_bridge->IsOracleActive() || active_bridge->CpuBarrier(); }

extern "C" int kfx_wgpu_native_read_barrier(const void* bytes, size_t length)
{ return !active_bridge || active_bridge->IsOracleActive() || active_bridge->ReadBarrier(bytes, length); }

extern "C" void kfx_wgpu_native_invalidate_frame(void)
{ if (active_bridge && !active_bridge->IsOracleActive()) active_bridge->InvalidateFrame(); }

extern "C" void kfx_wgpu_native_flush(void)
{ if (active_bridge != nullptr && !active_bridge->IsOracleActive()) active_bridge->EmitterBoundary(); }

extern "C" void* kfx_wgpu_native_context(void)
{ return active_bridge && !active_bridge->Failed() ? active_bridge->CursorContext() : nullptr; }

extern "C" uint64_t kfx_wgpu_native_target(const KfxGpolyTarget* target)
{ return active_bridge && target ? active_bridge->BorrowTarget(*target) : 0; }

extern "C" void kfx_wgpu_terrain_boundary(int allow_terrain)
{
    if (active_bridge != nullptr && !active_bridge->IsOracleActive()) active_bridge->Boundary(allow_terrain != 0);
}

extern "C" int kfx_wgpu_native_draw(const KfxGpolyTarget* target,
    const KfxWgpuDrawCommand* command, const KfxWgpuNativeResource* source,
    const KfxWgpuNativeResource* table, KfxWgpuNativeOracle oracle, void* oracle_context)
{
    return kfx_wgpu_native_draw_named(target, command, source, nullptr, table, oracle,
        oracle_context);
}

extern "C" int kfx_wgpu_native_draw_named(const KfxGpolyTarget* target,
    const KfxWgpuDrawCommand* command, const KfxWgpuNativeResource* source,
    const KfxWgpuNativeKey* name, const KfxWgpuNativeResource* table,
    KfxWgpuNativeOracle oracle, void* oracle_context)
{
    if (active_bridge == nullptr || active_bridge->IsOracleActive() || target == nullptr || command == nullptr) return 0;
    return active_bridge->SubmitNative(*target, *command, source, table, oracle, oracle_context,
        nullptr, name);
}

extern "C" int kfx_wgpu_native_draw_sprite(const KfxGpolyTarget* target,
    const KfxWgpuDrawCommand* command, const KfxWgpuSpriteAssets* assets,
    KfxWgpuNativeOracle oracle, void* oracle_context)
{
    if (active_bridge == nullptr || active_bridge->IsOracleActive() || target == nullptr ||
        command == nullptr || command->kind != KFX_WGPU_DRAW_SPRITE || assets == nullptr ||
        assets->artwork == nullptr || assets->ranges == nullptr ||
        assets->remap == nullptr) return 0;
    return active_bridge->SubmitNative(*target, *command, assets->artwork, assets->table,
        oracle, oracle_context, assets);
}

extern "C" int kfx_wgpu_native_draw_parts(const KfxGpolyTarget* target,
    const KfxWgpuDrawCommand* command, const KfxWgpuNativeResource* source,
    const KfxWgpuNativePart* parts, unsigned count, const KfxWgpuNativeResource* table,
    KfxWgpuNativeOracle oracle, void* oracle_context)
{
    if (active_bridge == nullptr || active_bridge->IsOracleActive() || target == nullptr ||
        command == nullptr || count > KFX_WGPU_NATIVE_PARTS ||
        (count != 0 && parts == nullptr)) return 0;
    for (unsigned i = 0; i < count; ++i)
        if (parts[i].resource.bytes == nullptr || parts[i].resource.tail != nullptr) return 0;
    return active_bridge->SubmitNative(*target, *command, source, table, oracle, oracle_context,
        nullptr, nullptr, parts, count);
}

extern "C" uint64_t kfx_wgpu_native_snapshot(const KfxGpolyTarget* target,
    uint32_t width, uint32_t height, uint32_t pitch, uint8_t* checkpoint)
{
    if (!active_bridge || !target || active_bridge->IsOracleActive()) return 0;
    return active_bridge->Snapshot(*target, width, height, pitch, checkpoint);
}

extern "C" void kfx_wgpu_native_snapshot_release(uint64_t snapshot)
{ if (active_bridge && snapshot) active_bridge->ReleaseSnapshot(snapshot); }

uint64_t WgpuTerrainBridge::Snapshot(const KfxGpolyTarget& target, uint32_t width,
    uint32_t height, uint32_t pitch, uint8_t* checkpoint)
{
    Flush();
    if (m_failed || !target.pixels || !width || !height || width > target.width ||
        height > target.height || target.width > target.pitch || width > pitch) return 0;
    try {
        if (m_context == nullptr) {
            if (m_fail_init) { Fail("injected GPU drawing initialization failure"); return 0; }
            m_context = kfx_wgpu_draw_create(m_error.data(), m_error.size());
            if (!m_context) { Fail(nullptr); return 0; }
        }
        if (m_gpu_valid && !SameTarget(target)) {
            ++m_counts.target_alias_barriers;
            if (!Materialize()) return 0;
            m_gpu_valid = false;
            if (m_frame_active) EndFrame(false);
        }
        uint32_t view_x, view_y;
        if (m_frame_active && !FrameView(target, view_x, view_y)) {
            ++m_counts.bridge_target_flushes;
            if (!EndFrame(true)) return Fail(nullptr);
        }
        m_native_target = target;
        if (!ValidateCpuLease() || !PrepareNativeTarget()) { Fail(nullptr); return 0; }
        uint64_t snapshot = kfx_wgpu_draw_target_snapshot(m_context, SubmissionTarget(m_native_target), 0, 0,
            width, height, pitch, m_error.data(), m_error.size());
        if (!snapshot) { Fail(nullptr); return 0; }
        m_counts.transition_snapshot_copy_bytes += static_cast<uint64_t>(width) * height * sizeof(uint32_t);
        if (checkpoint) {
            if (!CpuBarrier()) { ReleaseSnapshot(snapshot); return 0; }
            for (uint32_t row = 0; row < height; ++row)
                std::memcpy(checkpoint + static_cast<size_t>(row) * pitch,
                    target.pixels + static_cast<size_t>(row) * target.pitch, width);
            m_counts.transition_checkpoint_bytes += static_cast<uint64_t>(width) * height;
        }
        return snapshot;
    } catch (const std::exception& error) { Fail(error.what()); }
    catch (...) { Fail("native snapshot exception"); }
    return 0;
}

void WgpuTerrainBridge::ReleaseSnapshot(uint64_t snapshot)
{
    char error[1024] = {};
    if (m_context && snapshot)
        kfx_wgpu_draw_target_snapshot_release(m_context, snapshot, error, sizeof(error));
}

extern "C" int kfx_wgpu_native_shadow(const KfxGpolyTarget* target,
    const KfxWgpuDrawCommand* command, const KfxWgpuNativeResource* source,
    const KfxWgpuNativeResource* table, uint8_t* scratch,
    KfxWgpuNativeOracle oracle, void* oracle_context)
{
    if (active_bridge == nullptr || target == nullptr || command == nullptr || scratch == nullptr) return 0;
    return active_bridge->SubmitShadow(*target, *command, source, table, scratch, oracle, oracle_context);
}

int WgpuTerrainBridge::SubmitShadow(const KfxGpolyTarget& target, const KfxWgpuDrawCommand& command,
    const KfxWgpuNativeResource* source, const KfxWgpuNativeResource* table, uint8_t* scratch,
    KfxWgpuNativeOracle oracle, void* oracle_context)
{
    Boundary(false);
    // The route takes one command but keeps its place in the ordered record list.
    m_shadow_scratch = scratch;
    int accepted = SubmitNative(target, command, source, table, oracle, oracle_context);
    m_shadow_scratch = nullptr;
    return accepted;
}

WgpuTerrainBridge::WgpuTerrainBridge(uint64_t fail_after, bool fail_init, bool verify, bool resident)
    : m_resident_enabled(resident), m_fail_after(fail_after), m_fail_init(fail_init), m_verify(verify)
{
    active_bridge = this;
    kfx_gpoly_set_sink(Sink, this);
    kfx_gpoly_triangle_sink = TriangleSink;
    kfx_gpoly_triangle_context = this;
}

WgpuTerrainBridge::~WgpuTerrainBridge()
{
    EndFrame(true);
    if (m_context && context_cleanup) context_cleanup(m_context);
    ReleaseViews();
    if (active_bridge == this) active_bridge = nullptr;
    if (kfx_gpoly_sink_context == this) kfx_gpoly_set_sink(nullptr, nullptr);
    if (kfx_gpoly_triangle_context == this) {
        kfx_gpoly_triangle_sink = nullptr;
        kfx_gpoly_triangle_context = nullptr;
    }
    if (m_context != nullptr && !m_borrowed_context) kfx_wgpu_draw_destroy(m_context);
}

KfxWgpuDrawCounters WgpuTerrainBridge::GetGpuCounters() const
{
    KfxWgpuDrawCounters counters = {};
    char error[1024] = {};
    if (m_context != nullptr) kfx_wgpu_draw_counters(m_context, &counters, error, sizeof(error));
    return counters;
}

bool WgpuTerrainBridge::FrameView(const KfxGpolyTarget& target, uint32_t& x, uint32_t& y) const
{
    if (!m_frame_active || !target.pixels || target.pitch != m_frame_target.pitch ||
        !target.width || !target.height) return false;
    const uintptr_t base = reinterpret_cast<uintptr_t>(m_frame_target.pixels);
    const uintptr_t address = reinterpret_cast<uintptr_t>(target.pixels);
    if (address < base) return false;
    const size_t offset = address - base;
    const size_t row = offset / m_frame_target.pitch;
    if (row >= m_frame_target.height) return false;
    x = offset % m_frame_target.pitch;
    y = static_cast<uint32_t>(row);
    return x < m_frame_target.width &&
        target.width <= m_frame_target.width - x && target.height <= m_frame_target.height - y;
}

bool WgpuTerrainBridge::OwnsFrame(const KfxGpolyTarget& target) const
{
    uint32_t x, y;
    return !m_frame_invalid && FrameView(target, x, y);
}

bool WgpuTerrainBridge::BeginFrame(const KfxGpolyTarget& target, bool discard)
{
    if (m_failed || !target.pixels || !target.width || !target.height || target.width > target.pitch)
        return false;
    if (!EndFrame(false)) return false;
    uint32_t flags = 0;
    if (m_context != nullptr &&
        kfx_wgpu_draw_frame_status(m_context, &flags, m_error.data(), m_error.size()) == 1 &&
        flags != 0) {
        // A kernel found an out-of-range lookup one or two frames ago; that frame was
        // presented as drawn, so recovery is the full redraw, not a rollback.
        Invalidate();
        FullRedraw();
    }
    if (m_context && context_cleanup && m_frame_target.pixels &&
        (target.pixels != m_frame_target.pixels || target.width != m_frame_target.width ||
        target.height != m_frame_target.height || target.pitch != m_frame_target.pitch))
        context_cleanup(m_context);
    m_gpu_valid = false;
    m_resident_lease = false;
    m_frame_target = target;
    m_frame_active = true;
    m_discard_initial = discard;
    if (discard) { m_gpu_valid = false; m_gpu_dirty = false; }
    m_resident_enabled = true;
    BeginResident();
    return true;
}

bool WgpuTerrainBridge::EndFrame(bool materialize)
{
    Flush();
    bool valid = !m_frame_invalid;
    if (materialize) valid = CpuBarrier() && valid;
    if (m_queue_active) {
        if (kfx_wgpu_draw_frame_end(m_context, m_error.data(), m_error.size()) != 1) {
            Fail(nullptr);
            valid = false;
        }
        m_queue_active = false;
    }
    m_frame_active = false;
    m_discard_initial = false;
    return valid;
}

void WgpuTerrainBridge::ReleaseViews()
{
    for (const auto& view : m_views)
        kfx_wgpu_draw_target_release(m_context, view.handle, m_error.data(), m_error.size());
    m_views.clear();
}

uint64_t WgpuTerrainBridge::SubmissionTarget(const KfxGpolyTarget& native)
{
    uint32_t x, y;
    if (!FrameView(native, x, y)) return m_target;
    if (!x && !y && native.width == m_width && native.height == m_height)
        return m_target;
    for (const auto& view : m_views)
        if (view.x == x && view.y == y && view.width == native.width && view.height == native.height)
            return view.handle;
    const uint64_t handle = kfx_wgpu_draw_target_view(m_context, m_target, x, y,
        native.width, native.height, m_error.data(), m_error.size());
    if (handle) m_views.push_back({x, y, native.width, native.height, handle});
    return handle;
}

size_t WgpuTerrainBridge::ExpectedOffset() const
{
    uint32_t x, y;
    return FrameView(m_native_target, x, y) ? static_cast<size_t>(y) * m_width + x : 0;
}

bool WgpuTerrainBridge::SameTarget(const KfxGpolyTarget& target) const
{
    uint32_t x, y;
    if (m_frame_active && m_gpu_native_target.pixels == m_frame_target.pixels && FrameView(target, x, y)) return true;
    return m_gpu_native_target.pixels == target.pixels && m_gpu_native_target.width == target.width &&
        m_gpu_native_target.height == target.height && m_gpu_native_target.pitch == target.pitch;
}

void WgpuTerrainBridge::BeginResident()
{
    if (!m_resident_enabled || m_failed || m_resident_lease) return;
    m_resident_lease = true;
    ++m_counts.resident_sequences;
}

bool WgpuTerrainBridge::ValidateCpuLease()
{
    if (m_verify && m_resident_lease && m_gpu_valid) {
        for (uint32_t row = 0; row < m_height; ++row) {
            if (std::memcmp(m_gpu_native_target.pixels + static_cast<size_t>(row) * m_gpu_native_target.pitch,
                    m_cpu_checkpoint.data() + static_cast<size_t>(row) * m_width, m_width) != 0) {
                ++m_counts.missing_cpu_barriers;
                std::snprintf(m_error.data(), m_error.size(), "CPU target changed during GPU lease without a barrier");
                return false;
            }
        }
    }
    return true;
}

bool WgpuTerrainBridge::Materialize()
{
    if (m_frame_invalid) return false;
    if (!m_gpu_dirty) return true;
    if (!ValidateCpuLease()) {
        Fail(nullptr);
        return false;
    }
    if (kfx_wgpu_draw_readback(m_context, m_target, m_readback.data(), m_readback.size(),
            m_width, m_error.data(), m_error.size()) != 1) {
        Fail(nullptr);
        return false;
    }
    for (uint32_t row = 0; row < m_height; ++row)
        std::memcpy(m_gpu_native_target.pixels + static_cast<size_t>(row) * m_gpu_native_target.pitch,
            m_readback.data() + static_cast<size_t>(row) * m_width, m_width);
    ++m_counts.bridge_readbacks;
    ++m_counts.barrier_readbacks;
    m_counts.gpu_readback_bytes += static_cast<uint64_t>(m_width) * m_height * sizeof(uint32_t);
    m_counts.native_copy_bytes += static_cast<uint64_t>(m_width) * m_height;
    m_gpu_dirty = false;
    return true;
}

bool WgpuTerrainBridge::CpuBarrier()
{
    ++m_counts.cpu_barriers;
    Flush();
    const bool valid = Materialize();
    m_resident_lease = false;
    m_gpu_valid = false;
    return valid;
}

bool WgpuTerrainBridge::ReadBarrier(const void* bytes, size_t length)
{
    if (!bytes || !length) return true;
    const KfxGpolyTarget& owner = m_frame_active ? m_frame_target :
        (m_gpu_valid ? m_gpu_native_target : m_native_target);
    const uintptr_t first = reinterpret_cast<uintptr_t>(bytes);
    const uintptr_t screen = reinterpret_cast<uintptr_t>(owner.pixels);
    const size_t extent = owner.height ? static_cast<size_t>(owner.pitch) * (owner.height - 1) + owner.width : 0;
    if (!extent || (first <= screen ? screen - first >= length : first - screen >= extent)) return true;
    ++m_counts.cpu_barriers;
    Flush();
    const bool dirty = m_gpu_dirty;
    if (!Materialize()) return false;
    if (m_verify && m_gpu_valid && dirty) m_cpu_checkpoint = m_readback;
    return true;
}

void WgpuTerrainBridge::InvalidateFrame()
{
    Fail("native GPU frame invalidated; awaiting full redraw");
    if (!m_frame_invalid) ++m_counts.invalid_frames;
    m_frame_invalid = true;
}

/* Cached handles outlive a failed frame; the arena residency behind them is only
 * proven for frames that completed. A refused release leaves residency the caller was
 * told is gone, so the result is the caller's answer to whether the frame recovered. */
bool WgpuTerrainBridge::PurgeResources()
{
    bool purged = true;
    if (m_context != nullptr) {
        for (const auto* cache : {&m_native_tables, &m_remap_tables})
            for (const auto& resource : *cache)
                purged = kfx_wgpu_draw_resource_release(m_context, resource.handle,
                    m_error.data(), m_error.size()) == 1 && purged;
        purged = kfx_wgpu_draw_resources_purge_keyed(m_context, m_error.data(),
            m_error.size()) == 1 && purged;
    }
    m_native_tables.clear();
    m_remap_tables.clear();
    purged = CollectSuperseded() && purged;
    m_replay_assets.clear();
    m_texture_memo = m_fade_memo = {};
    m_table_memos = m_remap_memos = {};
    if (!purged) ++m_counts.resource_purge_failures;
    return purged;
}

void WgpuTerrainBridge::FullRedraw()
{
    if (m_queue_active) kfx_wgpu_draw_frame_abort(m_context, m_error.data(), m_error.size());
    // The shadow mask chain is cross-frame GPU state that a CPU redraw invalidates.
    if (m_context != nullptr)
        kfx_wgpu_draw_shadow_scratch_reset(m_context, m_error.data(), m_error.size());
    m_shadow_prior.assign(m_shadow_prior.size(), 0);
    const bool purged = PurgeResources();
    m_queue_active = false;
    m_frame_active = false;
    ClearPending();
    m_resident_lease = false;
    m_allow_terrain = false;
    m_frame_invalid = false;
    /* A refused purge leaves residency the caller was told is gone: the CPU keeps this
       frame, and the resources behind it stay allocated for the life of the context. */
    if (!purged) Invalidate();
    m_gpu_dirty = false;
    m_gpu_valid = false;
}

bool WgpuTerrainBridge::AttachPresenter(void* presenter)
{
    if (m_context != nullptr || m_failed) return false;
    m_context = kfx_wgpu_draw_context(presenter, m_error.data(), m_error.size());
    m_borrowed_context = m_context != nullptr;
    return m_borrowed_context;
}

void WgpuTerrainBridge::DetachPresenter()
{
    if (!m_borrowed_context) return;
    EndFrame(true);
    if (m_context && context_cleanup) context_cleanup(m_context);
    ReleaseViews();
    PurgeResources();
    if (m_target) kfx_wgpu_draw_target_release(m_context, m_target, m_error.data(), m_error.size());
    m_context = nullptr;
    m_target = 0;
    m_width = m_height = 0;
    m_borrowed_context = false;
}

uint64_t WgpuTerrainBridge::ResidentTarget(const KfxGpolyTarget& target, uint64_t* replay_ns)
{
    Flush();
    if (replay_ns) *replay_ns = 0;
    if (m_queue_active) {
        const auto start = replay_ns ? std::chrono::steady_clock::now() : std::chrono::steady_clock::time_point{};
        const int result = kfx_wgpu_draw_frame_flush(m_context, m_error.data(), m_error.size());
        if (replay_ns) *replay_ns = std::chrono::duration_cast<std::chrono::nanoseconds>(
            std::chrono::steady_clock::now() - start).count();
        if (result != 1) {
            Fail(nullptr);
            return 0;
        }
    }
    return !m_failed && !m_frame_invalid && m_resident_lease && m_gpu_valid &&
        target.pixels == m_gpu_native_target.pixels && target.width == m_width && target.height == m_height &&
        target.pitch == m_gpu_native_target.pitch ? m_target : 0;
}

uint64_t WgpuTerrainBridge::BorrowTarget(const KfxGpolyTarget& target)
{
    Flush();
    if (m_failed || m_frame_invalid || !m_resident_lease || !m_gpu_valid ||
        target.pixels != m_gpu_native_target.pixels || target.width != m_width ||
        target.height != m_height || target.pitch != m_gpu_native_target.pitch) return 0;
    m_gpu_dirty = true;
    return m_target;
}

void WgpuTerrainBridge::Boundary(bool allow_terrain)
{
    if (!allow_terrain) {
        // The pending record list keeps issue order, so a live frame needs no flush here.
        if (!m_frame_active) CpuBarrier();
        else if (!m_resident_lease || m_verify) Flush();
    } else BeginResident();
    m_allow_terrain = allow_terrain;
}

void WgpuTerrainBridge::Invalidate()
{
    if (m_frame_invalid) return;
    m_frame_invalid = true;
    ++m_counts.invalid_frames;
}

int WgpuTerrainBridge::Fail(const char* reason)
{
    if (m_failed) return KFX_GPOLY_DECLINED;
    if (reason != nullptr) std::snprintf(m_error.data(), m_error.size(), "%s", reason);
    // Fail owns whatever is still pending: it is replayed onto the target, or the frame is
    // invalidated so RendererSoftware redraws it. Dropping a run and staying valid is a hole.
    if (m_gpu_dirty || m_queue_active) {
        Invalidate();
        DiscardPending();
        if (m_queue_active) kfx_wgpu_draw_frame_abort(m_context, m_error.data(), m_error.size());
        m_queue_active = false;
    } else if (!m_pending.empty() || !m_triangles.empty()) {
        bool replayed = false;
        try { replayed = ReplayPending(); }
        catch (...) {
            DiscardPending();
            std::snprintf(m_error.data(), m_error.size(), "immutable terrain recovery failed; target unchanged");
        }
        if (!replayed) Invalidate();
    }
    m_failed = true;
    ++m_counts.failures;
    return KFX_GPOLY_DECLINED;
}

int WgpuTerrainBridge::Sink(void* context, const KfxGpolyTarget* target,
    const KfxGpolySpan* span, const uint8_t* texture, const uint8_t* fade)
{
    auto& bridge = *static_cast<WgpuTerrainBridge*>(context);
    int result;
    try {
        result = bridge.Draw(*target, *span, texture, fade);
    } catch (const std::exception& error) {
        result = bridge.Fail(error.what());
    } catch (...) {
        result = bridge.Fail("native drawing bridge exception");
    }
    if (result != KFX_GPOLY_CONSUMED) {
        if (!bridge.CpuBarrier()) return KFX_GPOLY_CONSUMED;
        ++bridge.m_counts.cpu_gpoly_spans;
    }
    return result;
}

/* Only storage registered as an immutable asset range may be keyed by pointer;
 * vec_map can also point at mutable scratch, which must keep content comparison. */
const void* WgpuTerrainBridge::StableKey(const void* bytes, size_t length)
{
    return kfx_render_asset_stable(bytes, length) ? bytes : nullptr;
}

uint64_t WgpuTerrainBridge::KeyedResource(uint32_t kind, const void* key, const void* tail_key,
    uint64_t generation, const uint8_t* bytes, const Extent& extent)
{
    if (key == nullptr) {
        const uint64_t handle = kfx_wgpu_draw_resource_create(m_context, bytes, extent.length,
            extent.width, extent.height, extent.pitch, m_error.data(), m_error.size());
        if (handle == 0) return 0;
        m_counts.resource_snapshot_bytes += extent.length;
        m_superseded.push_back(handle);
        return handle;
    }
    return KeyedResourceRaw(kind, static_cast<uint64_t>(reinterpret_cast<uintptr_t>(tail_key)),
        static_cast<uint64_t>(reinterpret_cast<uintptr_t>(key)), generation, bytes, extent);
}

uint64_t WgpuTerrainBridge::KeyedResourceRaw(uint32_t kind, uint64_t key_hi, uint64_t key_lo,
    uint64_t generation, const uint8_t* bytes, const Extent& extent)
{
    uint64_t previous = 0;
    const uint64_t handle = kfx_wgpu_draw_resource_create_keyed(m_context, kind, key_hi, key_lo,
        generation, bytes, extent.length, extent.width, extent.height, extent.pitch, &previous,
        m_error.data(), m_error.size());
    if (handle == 0) return 0;
    if (previous != handle) m_counts.resource_snapshot_bytes += extent.length;
    // The snapshot follows the handle, not the key: a run recorded before the bump can
    // still be replayed from the superseded bytes until CollectSuperseded releases them.
    if (previous != 0 && previous != handle) m_superseded.push_back(previous);
    return handle;
}

uint64_t WgpuTerrainBridge::TerrainResource(uint32_t kind, KeyMemo& memo, const void* key,
    const uint8_t* bytes, const Extent& extent)
{
    const uint64_t handle = KeyedResource(kind, key, nullptr, kfx_render_asset_generation, bytes,
        extent);
    if (handle == 0) return 0;
    m_replay_assets.try_emplace(handle, bytes, bytes + extent.length);
    memo = {key, nullptr, kfx_render_asset_generation, extent, key != nullptr ? handle : 0};
    return handle;
}

/* The 32x32 tile the gpoly kernels read lives at pitch 256; only a keyed miss pays for
 * the copy that squares it up. */
uint64_t WgpuTerrainBridge::TextureResource(const uint8_t* texture)
{
    const Extent extent = {KFX_GPOLY_TEXTURE_BYTES, 32, 32, 256};
    const void* key = StableKey(texture, TEXTURE_READ_BYTES);
    const uint64_t memoized = MemoHandle(m_texture_memo, key, nullptr, extent);
    if (memoized != 0) return memoized;
    std::array<uint8_t, KFX_GPOLY_TEXTURE_BYTES> bytes = {};
    for (size_t row = 0; row < 32; ++row)
        std::memcpy(bytes.data() + row * 256, texture + row * 256, 32);
    return TerrainResource(KFX_WGPU_DRAW_KEY_TERRAIN_TILE, m_texture_memo, key, bytes.data(),
        extent);
}

uint64_t WgpuTerrainBridge::FadeResource(const uint8_t* fade)
{
    const Extent extent = {KFX_GPOLY_FADE_BYTES, 256, 64, 256};
    const void* key = StableKey(fade, KFX_GPOLY_FADE_BYTES);
    const uint64_t memoized = MemoHandle(m_fade_memo, key, nullptr, extent);
    if (memoized != 0) return memoized;
    return TerrainResource(KFX_WGPU_DRAW_KEY_TERRAIN_FADE, m_fade_memo, key, fade, extent);
}

uint64_t WgpuTerrainBridge::MemoHandle(const KeyMemo& memo, const void* key,
    const void* tail_key, const Extent& extent) const
{
    return key != nullptr && memo.handle != 0 && memo.key == key && memo.tail_key == tail_key &&
        memo.extent == extent && memo.generation == kfx_render_asset_generation ? memo.handle : 0;
}

bool WgpuTerrainBridge::CollectSuperseded()
{
    char error[1024] = {};
    bool released = true;
    if (m_context != nullptr)
        for (const uint64_t handle : m_superseded) {
            released = kfx_wgpu_draw_resource_release(m_context, handle, error, sizeof(error)) == 1
                && released;
            m_replay_assets.erase(handle);
        }
    m_superseded.clear();
    return released;
}

const uint8_t* WgpuTerrainBridge::ReplayAsset(uint64_t handle) const
{
    const auto asset = m_replay_assets.find(handle);
    return asset == m_replay_assets.end() ? nullptr : asset->second.data();
}

/* Interns a lookup table by the caller's buffer identity. Tables built on the caller's
 * stack carry no identity the drawing context can key, so they still compare content. */
uint64_t WgpuTerrainBridge::TableResource(const KfxWgpuNativeResource& table, uint32_t kind,
    std::vector<Resource>& cache, TableMemos& memos, size_t limit)
{
    const size_t length = table.length + table.tail_length;
    const void* key = StableKey(table.bytes, table.length);
    const void* tail_key = table.tail == nullptr ? nullptr
                                                 : StableKey(table.tail, table.tail_length);
    const Extent extent = {length, table.width, table.height, table.pitch};
    if (key != nullptr && (table.tail == nullptr || tail_key != nullptr)) {
        // Before the concatenation: a resident key needs no bytes, and the tables are
        // 64 KiB and 80 KiB.
        for (const auto& memo : memos.slots) {
            const uint64_t memoized = MemoHandle(memo, key, tail_key, extent);
            if (memoized != 0) return memoized;
        }
        std::vector<uint8_t> bytes;
        bytes.reserve(length);
        bytes.insert(bytes.end(), table.bytes, table.bytes + table.length);
        if (table.tail_length != 0)
            bytes.insert(bytes.end(), table.tail, table.tail + table.tail_length);
        const uint64_t handle = KeyedResource(kind, key, tail_key, kfx_render_asset_generation,
            bytes.data(), extent);
        // No replay half: RasterizePending only replays terrain, so no snapshot is kept.
        if (handle != 0) {
            memos.slots[memos.next] = {key, tail_key, kfx_render_asset_generation, extent,
                handle};
            memos.next = (memos.next + 1) % kTableMemos;
        }
        return handle;
    }
    for (const auto& resource : cache) {
        if (resource.width != table.width || resource.height != table.height ||
            resource.pitch != table.pitch || resource.bytes.size() != length) continue;
        if (std::memcmp(resource.bytes.data(), table.bytes, table.length) != 0) continue;
        if (table.tail_length != 0 && std::memcmp(resource.bytes.data() + table.length,
                table.tail, table.tail_length) != 0) continue;
        return resource.handle;
    }
    if (cache.size() >= limit) {
        Flush();
        if (m_failed) return 0;
        if (kfx_wgpu_draw_resource_release(m_context, cache.front().handle,
                m_error.data(), m_error.size()) != 1) return 0;
        cache.erase(cache.begin());
    }
    Resource resource = {0, {}, table.width, table.height, table.pitch};
    resource.bytes.reserve(length);
    resource.bytes.insert(resource.bytes.end(), table.bytes, table.bytes + table.length);
    if (table.tail_length != 0)
        resource.bytes.insert(resource.bytes.end(), table.tail, table.tail + table.tail_length);
    resource.handle = kfx_wgpu_draw_resource_create(m_context, resource.bytes.data(), length,
        table.width, table.height, table.pitch, m_error.data(), m_error.size());
    if (resource.handle == 0) return 0;
    m_counts.resource_snapshot_bytes += length;
    cache.push_back(std::move(resource));
    return cache.back().handle;
}

int WgpuTerrainBridge::Draw(const KfxGpolyTarget& target, const KfxGpolySpan& span,
    const uint8_t* texture, const uint8_t* fade)
{
    if (m_failed || !m_allow_terrain) return KFX_GPOLY_DECLINED;
    if (m_verify && !m_triangles.empty()) Flush();
    if (m_failed) return KFX_GPOLY_DECLINED;
    if (target.pixels == nullptr || texture == nullptr || fade == nullptr || span.x < 0 ||
        span.y < 0 || span.count == 0 || target.pitch < target.width ||
        static_cast<uint32_t>(span.y) >= target.height ||
        static_cast<uint64_t>(span.x) + span.count > target.width ||
        static_cast<uint64_t>(target.pitch) * target.height > std::numeric_limits<size_t>::max())
        return Fail("invalid native terrain span");
    uint64_t position = (static_cast<uint64_t>(span.start_high) << 32) | span.start_low;
    const uint64_t step = (static_cast<uint64_t>(span.step_high) << 32) | span.step_low;
    for (uint32_t pixel = 0; pixel < span.count; ++pixel, position += step)
        if ((position & 0xff00) >= KFX_GPOLY_FADE_BYTES) return Fail("invalid terrain shade");
    if (m_context == nullptr) {
        if (m_fail_init) return Fail("injected GPU drawing initialization failure");
        m_context = kfx_wgpu_draw_create(m_error.data(), m_error.size());
        if (m_context == nullptr) return Fail(nullptr);
    }
    // Verification rasterizes the run itself, which needs the single-target invariant.
    if (m_pending.size() >= kPendingSpanLimit || (m_verify && PendingTargetChanged(target))) {
        Flush();
        if (m_failed) return KFX_GPOLY_DECLINED;
    }
    if (m_gpu_valid && !SameTarget(target)) {
        ++m_counts.target_alias_barriers;
        if (!Materialize()) return Fail(nullptr);
        m_gpu_valid = false;
        if (m_frame_active) EndFrame(false);
    }
    uint32_t view_x, view_y;
    if (m_frame_active && !FrameView(target, view_x, view_y)) {
        ++m_counts.bridge_target_flushes;
        if (!EndFrame(true)) return Fail(nullptr);
    }
    m_native_target = target;
    if (!ReadBarrier(texture, KFX_GPOLY_TEXTURE_BYTES) || !ReadBarrier(fade, KFX_GPOLY_FADE_BYTES))
        return KFX_GPOLY_DECLINED;
    const uint64_t texture_handle = TextureResource(texture);
    if (texture_handle == 0) return Fail(nullptr);
    const uint64_t fade_handle = FadeResource(fade);
    if (fade_handle == 0) return Fail(nullptr);
    KfxWgpuDrawCommand command = {};
    command.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
    command.kind = KFX_WGPU_DRAW_GPOLY_SPAN;
    command.x = span.x;
    command.y = span.y;
    command.width = span.count;
    command.height = 1;
    command.clip_width = target.width;
    command.clip_height = target.height;
    command.source = texture_handle;
    command.table = fade_handle;
    command.start_low = span.start_low;
    command.start_high = span.start_high;
    command.step_low = span.step_low;
    command.step_high = span.step_high;
    command.transparent = KFX_WGPU_DRAW_OPAQUE;
    AppendCommand(command, 0);
    return KFX_GPOLY_CONSUMED;
}

int WgpuTerrainBridge::TriangleSink(void* context, const KfxGpolyTarget* target,
    const KfxWgpuTriangle* triangle, const uint8_t* texture, const uint8_t* fade,
    KfxGpolyRasterizer rasterizer)
{
    auto& bridge = *static_cast<WgpuTerrainBridge*>(context);
    int result;
    try { result = bridge.DrawTriangle(*target, *triangle, texture, fade, rasterizer); }
    catch (const std::exception& error) { result = bridge.Fail(error.what()); }
    catch (...) { result = bridge.Fail("native triangle bridge exception"); }
    if (result != KFX_GPOLY_CONSUMED) {
        if (!bridge.CpuBarrier()) return KFX_GPOLY_CONSUMED;
        ++bridge.m_counts.cpu_triangles;
    }
    return result;
}

int WgpuTerrainBridge::DrawTriangle(const KfxGpolyTarget& target,
    const KfxWgpuTriangle& triangle, const uint8_t* texture, const uint8_t* fade,
    KfxGpolyRasterizer rasterizer)
{
    if (m_failed || !m_allow_terrain) return KFX_GPOLY_DECLINED;
    if (!target.pixels || !texture || !fade || !rasterizer || !target.width || !target.height ||
        target.width > 32767 || target.height > 32767 || target.pitch < target.width ||
        static_cast<uint64_t>(target.pitch) * target.height > std::numeric_limits<size_t>::max())
        return KFX_GPOLY_DECLINED;
    for (const auto& vertex : triangle.vertices)
        if (vertex.x < -32768 || vertex.x > 32767 || vertex.y < -32768 || vertex.y > 32767)
            return KFX_GPOLY_DECLINED;
    if ((m_verify && (!m_pending.empty() || PendingTargetChanged(target))) ||
        m_triangles.size() >= kPendingLimit || (m_rasterizer && m_rasterizer != rasterizer)) Flush();
    if (m_failed) return KFX_GPOLY_DECLINED;
    if (m_context == nullptr) {
        if (m_fail_init) return Fail("injected GPU drawing initialization failure");
        m_context = kfx_wgpu_draw_create(m_error.data(), m_error.size());
        if (m_context == nullptr) return Fail(nullptr);
    }
    if (m_gpu_valid && !SameTarget(target)) {
        ++m_counts.target_alias_barriers;
        if (!Materialize()) return Fail(nullptr);
        m_gpu_valid = false;
        if (m_frame_active) EndFrame(false);
    }
    uint32_t view_x, view_y;
    if (m_frame_active && !FrameView(target, view_x, view_y)) {
        ++m_counts.bridge_target_flushes;
        if (!EndFrame(true)) return Fail(nullptr);
    }
    m_native_target = target;
    m_rasterizer = rasterizer;
    if (!ReadBarrier(texture, KFX_GPOLY_TEXTURE_BYTES) || !ReadBarrier(fade, KFX_GPOLY_FADE_BYTES))
        return KFX_GPOLY_DECLINED;
    KfxWgpuTriangle owned = triangle;
    owned.source = TextureResource(texture);
    if (!owned.source) return Fail(nullptr);
    owned.table = FadeResource(fade);
    if (!owned.table) return Fail(nullptr);
    AppendTriangle(owned);
    return KFX_GPOLY_CONSUMED;
}

bool WgpuTerrainBridge::PendingIsReplayable() const
{
    // Generic commands keep their oracle in the caller, so a mixed run has no CPU replay.
    for (const auto& command : m_pending)
        if (command.kind != KFX_WGPU_DRAW_GPOLY_SPAN) return false;
    // RasterizePending writes one buffer its callers size from m_native_target, so a run
    // recorded against several views would place an earlier, larger view's spans outside it.
    for (const auto& run : m_order)
        if (!SameRun(run.target, m_order.front().target)) return false;
    return true;
}

bool WgpuTerrainBridge::RasterizePending(uint8_t* pixels, uint32_t pitch) const
{
    if (!PendingIsReplayable()) return false;
    size_t commands = 0, triangles = 0;
    for (const auto& run : m_order) {
        for (uint32_t index = 0; index < run.count; ++index) {
            const uint8_t* texture = nullptr;
            const uint8_t* fade = nullptr;
            if (run.kind == kRunTriangles) {
                const auto& triangle = m_triangles[triangles++];
                texture = ReplayAsset(triangle.source);
                fade = ReplayAsset(triangle.table);
                if (texture == nullptr || fade == nullptr) return false;
                KfxGpolyTarget target = {pixels, m_native_target.width, m_native_target.height, pitch};
                if (!m_rasterizer(&target, &triangle, texture, fade)) return false;
                continue;
            }
            const auto& command = m_pending[commands++];
            texture = ReplayAsset(command.source);
            fade = ReplayAsset(command.table);
            if (texture == nullptr || fade == nullptr) return false;
            uint64_t position = (static_cast<uint64_t>(command.start_high) << 32) | command.start_low;
            const uint64_t step = (static_cast<uint64_t>(command.step_high) << 32) | command.step_low;
            uint8_t* destination = pixels + static_cast<size_t>(command.y) * pitch + command.x;
            for (uint32_t pixel = 0; pixel < command.width; ++pixel, position += step) {
                const uint32_t high = static_cast<uint32_t>(position >> 32);
                const uint32_t uv = ((high << 8) | (high >> 24)) & 0x1f1f;
                destination[pixel] = fade[texture[uv] | (position & 0xff00)];
            }
        }
    }
    return true;
}

void WgpuTerrainBridge::AppendCommand(const KfxWgpuDrawCommand& command, uint64_t source)
{
    const RunKind kind = command.kind == KFX_WGPU_DRAW_SHADOW ? kRunShadow : kRunCommands;
    if (m_order.empty() || m_order.back().kind != kind || kind == kRunShadow ||
        !SameRun(m_order.back().target, m_native_target)) {
        if (!m_order.empty() && !SameRun(m_order.back().target, m_native_target))
            ++m_counts.bridge_target_runs;
        m_order.push_back({kind, 0, m_native_target});
    }
    ++m_order.back().count;
    m_pending.push_back(command);
    m_pending_sources.push_back(source);
}

void WgpuTerrainBridge::AppendTriangle(const KfxWgpuTriangle& triangle)
{
    if (m_order.empty() || m_order.back().kind != kRunTriangles ||
        !SameRun(m_order.back().target, m_native_target)) {
        if (!m_order.empty() && !SameRun(m_order.back().target, m_native_target))
            ++m_counts.bridge_target_runs;
        m_order.push_back({kRunTriangles, 0, m_native_target});
    }
    ++m_order.back().count;
    m_triangles.push_back(triangle);
}

void WgpuTerrainBridge::ClearPending()
{
    char error[1024] = {};
    for (const uint64_t source : m_pending_sources)
        if (source != 0 && m_context != nullptr)
            kfx_wgpu_draw_resource_release(m_context, source, error, sizeof(error));
    m_pending_sources.clear();
    m_pending.clear();
    m_triangles.clear();
    m_order.clear();
    // Memos and replay snapshots key on live handles, not on the run, so only the
    // per-call handles this run created are released here.
    CollectSuperseded();
}

void WgpuTerrainBridge::DiscardPending()
{
    for (const auto& command : m_pending) {
        if (command.kind == KFX_WGPU_DRAW_GPOLY_SPAN) ++m_counts.rejected_spans;
        else ++m_counts.rejected_commands;
    }
    m_counts.rejected_triangles += m_triangles.size();
    ClearPending();
}

bool WgpuTerrainBridge::ReplayPending()
{
    if (!PendingIsReplayable()) {
        std::snprintf(m_error.data(), m_error.size(),
            "batched generic commands have no CPU replay; pending run dropped for a full redraw");
        DiscardPending();
        return false;
    }
    std::vector<uint8_t> recovered(static_cast<size_t>(m_native_target.width) * m_native_target.height);
    for (uint32_t row = 0; row < m_native_target.height; ++row)
        std::memcpy(recovered.data() + static_cast<size_t>(row) * m_native_target.width,
            m_native_target.pixels + static_cast<size_t>(row) * m_native_target.pitch, m_native_target.width);
    if (!RasterizePending(recovered.data(), m_native_target.width)) {
        std::snprintf(m_error.data(), m_error.size(), "invalid triangle shade; immutable fallback batch rejected without target writes");
        DiscardPending();
        return false;
    }
    for (uint32_t row = 0; row < m_native_target.height; ++row)
        std::memcpy(m_native_target.pixels + static_cast<size_t>(row) * m_native_target.pitch,
            recovered.data() + static_cast<size_t>(row) * m_native_target.width, m_native_target.width);
    m_counts.cpu_replayed_spans += m_pending.size();
    m_counts.replayed_triangles += m_triangles.size();
    ClearPending();
    return true;
}

bool WgpuTerrainBridge::PrepareNativeTarget()
{
    const KfxGpolyTarget& owner = m_frame_active ? m_frame_target : m_native_target;
    if (m_width != owner.width || m_height != owner.height) {
        ReleaseViews();
        if (m_target != 0 && kfx_wgpu_draw_target_release(m_context, m_target,
                m_error.data(), m_error.size()) != 1) return false;
        m_target = 0;
        m_width = owner.width;
        m_height = owner.height;
        m_readback.resize(static_cast<size_t>(m_width) * m_height);
        m_target = kfx_wgpu_draw_target_create(m_context, m_width, m_height,
            m_error.data(), m_error.size());
        if (m_target == 0) return false;
        ++m_counts.target_creations;
    }
    if (m_frame_active) BeginResident();
    if (!m_resident_lease || !m_gpu_valid) {
        if (!m_discard_initial) {
            const size_t initial_length = static_cast<size_t>(owner.pitch) * (m_height - 1) + m_width;
            const uint64_t initial = kfx_wgpu_draw_resource_create(m_context, owner.pixels,
                initial_length, m_width, m_height, owner.pitch, m_error.data(), m_error.size());
            if (initial == 0) return false;
            KfxWgpuDrawCommand copy = {};
            copy.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
            copy.kind = KFX_WGPU_DRAW_IMAGE;
            copy.width = copy.clip_width = copy.source_width = m_width;
            copy.height = copy.clip_height = copy.source_height = m_height;
            copy.source = initial;
            copy.transparent = KFX_WGPU_DRAW_OPAQUE;
            bool success = kfx_wgpu_draw_submit(m_context, m_target, &copy, 1,
                m_error.data(), m_error.size()) == 1;
            m_counts.bridge_initial_index_bytes += initial_length;
            char release_error[1024] = {};
            if (kfx_wgpu_draw_resource_release(m_context, initial, release_error, sizeof(release_error)) != 1)
                success = false;
            if (!success) return false;
        }
        m_discard_initial = false;
        m_gpu_native_target = owner;
        m_gpu_valid = m_resident_lease;
        if (m_verify && m_resident_lease) {
            m_expected.resize(static_cast<size_t>(m_width) * m_height);
            for (uint32_t row = 0; row < m_height; ++row)
                std::memcpy(m_expected.data() + static_cast<size_t>(row) * m_width,
                    owner.pixels + static_cast<size_t>(row) * owner.pitch, m_width);
            m_cpu_checkpoint = m_expected;
        }
    }
    if (m_frame_active && !m_queue_active) {
        if (kfx_wgpu_draw_frame_begin(m_context, m_target, m_error.data(), m_error.size()) != 1) return false;
        m_queue_active = true;
    }
    return true;
}

bool WgpuTerrainBridge::ExecutePending(KfxWgpuNativeOracle oracle, void* oracle_context)
{
    if (!ValidateCpuLease()) return false;
    if (m_fail_after != 0 && m_counts.gpu_batches >= m_fail_after) {
        std::snprintf(m_error.data(), m_error.size(), "injected GPU drawing batch failure");
        return false;
    }
    if (!PrepareNativeTarget()) return false;
    const uint64_t destination = SubmissionTarget(m_native_target);
    if (!destination) return false;
    if (m_resident_lease) m_gpu_dirty = true;
    size_t routes = 0;
    bool shadow_route = false;
    if (m_pending.size() == 1 && m_triangles.empty() &&
            m_pending[0].kind == KFX_WGPU_DRAW_TRANSITION) {
        if (kfx_wgpu_draw_submit_target_images(m_context, destination, m_pending.data(), 1,
                m_error.data(), m_error.size()) != 1) return false;
        routes = 1;
    } else if (m_order.empty()) {
        if (kfx_wgpu_draw_submit(m_context, destination, m_pending.data(), 0,
                m_error.data(), m_error.size()) != 1) return false;
        routes = 1;
    } else {
        size_t commands = 0, triangles = 0;
        for (const auto& run : m_order) {
            // Each run carries the view it was recorded against; the stream keeps their order.
            const uint64_t into = SubmissionTarget(run.target);
            if (!into) return false;
            if (run.kind == kRunTriangles) {
                if (kfx_wgpu_draw_submit_triangles(m_context, into, m_triangles.data() + triangles,
                        run.count, m_error.data(), m_error.size()) != 1) return false;
                triangles += run.count;
            } else if (run.kind == kRunShadow) {
                // AppendCommand opens a fresh run per shadow, and the route accepts one command.
                if (run.count != 1) {
                    std::snprintf(m_error.data(), m_error.size(), "shadow run must hold one command");
                    return false;
                }
                if (kfx_wgpu_draw_submit_shadow(m_context, into, m_pending.data() + commands,
                        m_error.data(), m_error.size()) != 1) return false;
                commands += run.count;
                shadow_route = true;
                ++m_counts.gpu_shadow_commands;
            } else {
                if (kfx_wgpu_draw_submit(m_context, into, m_pending.data() + commands,
                        run.count, m_error.data(), m_error.size()) != 1) return false;
                commands += run.count;
            }
            ++routes;
        }
    }
    if (m_pending.size() == 1 && NeedsSoloBatch(m_pending[0])) ++m_counts.bridge_solo_batches;
    if (!m_resident_lease || m_verify) {
        if (kfx_wgpu_draw_readback(m_context, m_target, m_readback.data(), m_readback.size(),
                m_width, m_error.data(), m_error.size()) != 1) return false;
        ++m_counts.bridge_readbacks;
        m_counts.gpu_readback_bytes += static_cast<uint64_t>(m_width) * m_height * sizeof(uint32_t);
        if (m_resident_lease) ++m_counts.verification_readbacks;
    }
    if (m_verify) {
        std::vector<uint8_t> expected(static_cast<size_t>(m_width) * m_height);
        if (m_resident_lease) expected = m_expected;
        else for (uint32_t row = 0; row < m_height; ++row)
            std::memcpy(expected.data() + static_cast<size_t>(row) * m_width,
                m_native_target.pixels + static_cast<size_t>(row) * m_native_target.pitch, m_width);
        bool prior_diverged = false, flagged_shade = false;
        if (oracle != nullptr) {
            const bool compare_scratch = shadow_route && m_shadow_scratch != nullptr;
            // The oracle runs on the scratch the software path would have used, so a resident
            // chain that has drifted from it is measured rather than hidden by re-seeding.
            if (compare_scratch &&
                std::memcmp(m_shadow_scratch, m_shadow_prior.data(), 65536) != 0) {
                prior_diverged = true;
                ++m_counts.shadow_prior_divergence;
            }
            m_oracle_active = true;
            oracle(expected.data() + ExpectedOffset(), m_width, oracle_context);
            m_oracle_active = false;
            if (compare_scratch) {
                std::vector<uint8_t> resident(65536);
                if (kfx_wgpu_draw_shadow_scratch_read(m_context, resident.data(), resident.size(),
                        m_error.data(), m_error.size()) != 1) return false;
                m_counts.shadow_scratch_readback_bytes += 65536 * sizeof(uint32_t);
                if (!prior_diverged &&
                    std::memcmp(m_shadow_scratch, resident.data(), 65536) != 0) {
                    std::snprintf(m_error.data(), m_error.size(), "GPU shadow scratch index comparison failed");
                    return false;
                }
                // Resume from the resident prior so divergence counts events, not every
                // later shadow, and the next masks are verified on their own terms.
                if (prior_diverged) {
                    std::memcpy(m_shadow_scratch, resident.data(), 65536);
                    m_counts.shadow_scratch_copy_bytes += 65536;
                }
                m_shadow_prior = std::move(resident);
            }
            m_counts.verification_cpu_commands += m_pending.size();
        } else if (!RasterizePending(expected.data() + ExpectedOffset(), m_width)) {
            // The kernels flag and skip an out-of-range shade that the CPU oracle rejects
            // outright, so the two disagree by construction: count the batch rather than
            // compare it, and leave the GPU result authoritative as the frame flag reports it.
            flagged_shade = true;
            ++m_counts.verification_flagged_shades;
        } else {
            m_counts.verification_cpu_spans += m_pending.size();
        }
        // A diverged prior makes the two masks legitimately different, so the batch is counted
        // instead of compared; the GPU result stays authoritative for the resident checkpoint.
        if (!prior_diverged && !flagged_shade && expected != m_readback) {
            std::snprintf(m_error.data(), m_error.size(), "GPU terrain index comparison failed");
            return false;
        }
        if (m_resident_lease)
            m_expected = prior_diverged || flagged_shade ? m_readback : std::move(expected);
        if (!prior_diverged && !flagged_shade) {
            m_counts.verified_triangles += m_triangles.size();
            ++m_counts.verified_batches;
        }
    }
    if (!m_resident_lease) {
        for (uint32_t row = 0; row < m_height; ++row)
            std::memcpy(m_native_target.pixels + static_cast<size_t>(row) * m_native_target.pitch,
                m_readback.data() + static_cast<size_t>(row) * m_width, m_width);
        m_counts.native_copy_bytes += static_cast<uint64_t>(m_width) * m_height;
    } else ++m_counts.resident_batches;
    for (const auto& command : m_pending) {
        if (command.kind == KFX_WGPU_DRAW_GPOLY_SPAN) {
            m_counts.gpu_pixels += command.width;
            ++m_counts.gpu_spans;
        } else {
            ++m_counts.native_commands;
            if (command.kind == KFX_WGPU_DRAW_TRANSITION) ++m_counts.transition_commands;
            if (command.kind == KFX_WGPU_DRAW_SPRITE) {
                ++m_counts.gpu_sprite_commands;
                /* Bit 3 of source_x marks the serial row-copy ordered sprite path. */
                if (command.source_x & 8u) ++m_counts.gpu_ordered_sprites;
            }
        }
    }
    m_counts.gpu_triangles += m_triangles.size();
    m_counts.gpu_batches += routes;
    ClearPending();
    return true;
}

bool WgpuTerrainBridge::PacksInBatch(uint32_t kind)
{
    return kind <= KFX_WGPU_DRAW_TRIG || kind == KFX_WGPU_DRAW_MOVIE ||
        kind == KFX_WGPU_DRAW_MAP_VIEW || kind == KFX_WGPU_DRAW_BITMAP;
}

bool WgpuTerrainBridge::NeedsSoloBatch(const KfxWgpuDrawCommand& command)
{
    // The shadow route takes one command but keeps its place in the ordered record list.
    if (command.kind == KFX_WGPU_DRAW_SHADOW) return false;
    return !PacksInBatch(command.kind);
}

bool WgpuTerrainBridge::OrderedSprite(const KfxWgpuDrawCommand& command)
{
    /* Bit 3 of source_x marks the serial row-copy ordered sprite path. */
    return command.kind == KFX_WGPU_DRAW_SPRITE && (command.source_x & 8u) != 0;
}

bool WgpuTerrainBridge::SameRun(const KfxGpolyTarget& a, const KfxGpolyTarget& b)
{
    return a.pixels == b.pixels && a.width == b.width && a.height == b.height && a.pitch == b.pitch;
}

bool WgpuTerrainBridge::PendingTargetChanged(const KfxGpolyTarget& target) const
{
    return !SameRun(m_native_target, target);
}

void WgpuTerrainBridge::EmitterBoundary()
{
    if (m_resident_lease && !m_verify && !m_failed) return;
    Flush();
}

int WgpuTerrainBridge::SubmitNative(const KfxGpolyTarget& target,
    const KfxWgpuDrawCommand& command, const KfxWgpuNativeResource* source,
    const KfxWgpuNativeResource* table, KfxWgpuNativeOracle oracle, void* oracle_context,
    const KfxWgpuSpriteAssets* sprite, const KfxWgpuNativeKey* name,
    const KfxWgpuNativePart* parts, unsigned part_count)
{
    const KfxWgpuNativeResource* ranges = sprite != nullptr ? sprite->ranges : nullptr;
    const KfxWgpuNativeResource* remap = sprite != nullptr ? sprite->remap : nullptr;
    if (m_oracle_active) return 0;
    if (!m_pending.empty() || !m_triangles.empty()) {
        // A record carries the view it was issued against, so a target change opens a run
        // inside the pending list instead of closing it.
        if (m_verify || !m_resident_lease || NeedsSoloBatch(command) || OrderedSprite(command) ||
            m_pending.size() >= kPendingLimit) Flush();
    }
    m_allow_terrain = false;
    if (m_failed) return 0;
    if (m_verify && oracle == nullptr) {
        CpuBarrier();
        return 0;
    }
    if (target.pixels == nullptr || target.width == 0 || target.height == 0 ||
        target.pitch < target.width ||
        static_cast<uint64_t>(target.pitch) * target.height > std::numeric_limits<size_t>::max())
        return Fail("invalid native drawing target");
    uint64_t source_handle = 0, table_handle = 0;
    try {
        if (m_context == nullptr) {
            if (m_fail_init) return Fail("injected GPU drawing initialization failure");
            m_context = kfx_wgpu_draw_create(m_error.data(), m_error.size());
            if (m_context == nullptr) return Fail(nullptr);
        }
        if (m_gpu_valid && !SameTarget(target)) {
            ++m_counts.target_alias_barriers;
            if (!Materialize()) return Fail(nullptr);
            m_gpu_valid = false;
            if (m_frame_active) EndFrame(false);
        }
        uint32_t view_x, view_y;
        if (m_frame_active && !FrameView(target, view_x, view_y)) {
            ++m_counts.bridge_target_flushes;
            if (!EndFrame(true)) return Fail(nullptr);
        }
        m_native_target = target;
        auto aliases_range = [&](const uint8_t* bytes, size_t bytes_length) {
            if (!bytes || !bytes_length || !m_gpu_dirty) return false;
            const uintptr_t first = reinterpret_cast<uintptr_t>(bytes);
            const uintptr_t screen = reinterpret_cast<uintptr_t>(m_gpu_native_target.pixels);
            const size_t length = static_cast<size_t>(m_gpu_native_target.pitch) * (m_gpu_native_target.height - 1) + m_gpu_native_target.width;
            return first <= screen ? screen - first < bytes_length : first - screen < length;
        };
        auto aliases_target = [&](const KfxWgpuNativeResource* resource) {
            return resource && (aliases_range(resource->bytes, resource->length) ||
                aliases_range(resource->tail, resource->tail_length));
        };
        bool aliased = aliases_target(source) || aliases_target(ranges) ||
            aliases_target(remap) || aliases_target(table);
        for (unsigned i = 0; i < part_count; ++i)
            aliased = aliases_target(&parts[i].resource) || aliased;
        if (aliased) {
            ++m_counts.target_alias_barriers;
            for (const auto* resource : {source, ranges, remap, table}) {
                if (resource == nullptr) continue;
                if (!ReadBarrier(resource->bytes, resource->length)) return Fail(nullptr);
                if (resource->tail && !ReadBarrier(resource->tail, resource->tail_length))
                    return Fail(nullptr);
            }
            for (unsigned i = 0; i < part_count; ++i)
                if (!ReadBarrier(parts[i].resource.bytes, parts[i].resource.length))
                    return Fail(nullptr);
        }
        KfxWgpuDrawCommand owned = command;
        OwnedResource guard = {m_context, 0};
        if (source != nullptr) {
            if (sprite != nullptr && sprite->identity != nullptr) {
                /* Artwork the emitter can name stays resident under one handle, so the
                   arena keeps it across frames instead of re-uploading it per command. */
                source_handle = KeyedResourceRaw(KFX_WGPU_DRAW_KEY_SPRITE_ARTWORK,
                    (static_cast<uint64_t>(command.source_width) << 32) | command.source_height,
                    static_cast<uint64_t>(reinterpret_cast<uintptr_t>(sprite->identity)),
                    sprite->generation, source->bytes,
                    {source->length, source->width, source->height, source->pitch});
                if (source_handle == 0) return Fail(nullptr);
            } else if (name != nullptr && source->tail == nullptr) {
                // A keyed handle outlives the run that named it, so the record owns nothing.
                source_handle = KeyedResourceRaw(name->kind, name->hi, name->lo, name->generation,
                    source->bytes,
                    {source->length, source->width, source->height, source->pitch});
                if (source_handle == 0) return Fail(nullptr);
            } else {
                source_handle = kfx_wgpu_draw_resource_create(m_context, source->bytes,
                    source->length, source->width, source->height, source->pitch,
                    m_error.data(), m_error.size());
                if (source_handle == 0) return Fail(nullptr);
                m_counts.resource_snapshot_bytes += source->length;
                // The batch owns one handle; a second per-call one is released with the run.
                if (ranges == nullptr) guard.handle = source_handle;
                else m_superseded.push_back(source_handle);
            }
            if (source->cursor) kfx_wgpu_draw_resource_mark_cursor(m_context, source_handle);
        }
        uint64_t remap_handle = 0;
        if (ranges != nullptr) {
            guard.handle = kfx_wgpu_draw_resource_create(m_context, ranges->bytes, ranges->length,
                ranges->width, ranges->height, ranges->pitch, m_error.data(), m_error.size());
            if (guard.handle == 0) return Fail(nullptr);
            m_counts.resource_snapshot_bytes += ranges->length;
            remap_handle = TableResource(*remap, KFX_WGPU_DRAW_KEY_SPRITE_REMAP, m_remap_tables,
                m_remap_memos, 16);
            if (remap_handle == 0) return Fail(nullptr);
        }
        if (table != nullptr) {
            table_handle = TableResource(*table, KFX_WGPU_DRAW_KEY_NATIVE_TABLE, m_native_tables,
                m_table_memos, 16);
        }
        const bool resources_ready = table == nullptr || table_handle != 0;
        owned.source = command.kind == KFX_WGPU_DRAW_TRANSITION ? command.source : source_handle;
        owned.table = table_handle;
        if (ranges != nullptr) {
            owned.start_low = static_cast<uint32_t>(guard.handle);
            owned.start_high = static_cast<uint32_t>(guard.handle >> 32);
            owned.step_low = static_cast<uint32_t>(remap_handle);
            owned.step_high = static_cast<uint32_t>(remap_handle >> 32);
        }
        for (unsigned i = 0; i < part_count; ++i) {
            const KfxWgpuNativeResource& asset = parts[i].resource;
            const KfxWgpuNativeKey& key = parts[i].name;
            const uint64_t handle = KeyedResourceRaw(key.kind, key.hi, key.lo, key.generation,
                asset.bytes, {asset.length, asset.width, asset.height, asset.pitch});
            if (handle == 0) return Fail(nullptr);
            uint32_t* const words = i == 0 ? &owned.start_low : &owned.step_low;
            words[0] = static_cast<uint32_t>(handle);
            words[1] = static_cast<uint32_t>(handle >> 32);
        }
        bool success = false;
        if (resources_ready) {
            AppendCommand(owned, guard.Take());
            success = !NeedsSoloBatch(command) && !OrderedSprite(command) && !m_verify &&
                m_resident_lease && m_pending.size() < kPendingLimit;
            if (!success) success = ExecutePending(oracle, oracle_context);
        }
        if (!success) return Fail(nullptr);
        return 1;
    } catch (const std::exception& error) {
        return Fail(error.what());
    } catch (...) {
        return Fail("native command bridge exception");
    }
}

void WgpuTerrainBridge::Flush()
{
    if (m_pending.empty() && m_triangles.empty()) return;
    try {
        if (!ExecutePending()) Fail(nullptr);
    } catch (const std::exception& error) {
        Fail(error.what());
    } catch (...) {
        Fail("native terrain batch exception");
    }
}
#else
extern "C" uint64_t kfx_wgpu_native_snapshot(const KfxGpolyTarget*, uint32_t, uint32_t, uint32_t, uint8_t*) { return 0; }
extern "C" void kfx_wgpu_native_snapshot_release(uint64_t) {}
extern "C" int kfx_wgpu_native_shadow(const KfxGpolyTarget*, const KfxWgpuDrawCommand*,
    const KfxWgpuNativeResource*, const KfxWgpuNativeResource*, uint8_t*, KfxWgpuNativeOracle, void*) { return 0; }
extern "C" void* kfx_wgpu_native_context(void) { return nullptr; }
extern "C" void kfx_wgpu_native_context_cleanup(void (*)(void*)) {}
extern "C" uint64_t kfx_wgpu_native_target(const KfxGpolyTarget*) { return 0; }
extern "C" int kfx_wgpu_native_enabled(void) { return 0; }
extern "C" int kfx_wgpu_native_cpu_barrier(void) { return 1; }
extern "C" int kfx_wgpu_native_read_barrier(const void*, size_t) { return 1; }
extern "C" void kfx_wgpu_native_flush(void) {}
extern "C" void kfx_wgpu_native_invalidate_frame(void) {}
extern "C" void kfx_wgpu_terrain_boundary(int) {}
extern "C" int kfx_wgpu_native_draw(const KfxGpolyTarget*, const KfxWgpuDrawCommand*,
    const KfxWgpuNativeResource*, const KfxWgpuNativeResource*, KfxWgpuNativeOracle, void*) { return 0; }
extern "C" int kfx_wgpu_native_draw_named(const KfxGpolyTarget*, const KfxWgpuDrawCommand*,
    const KfxWgpuNativeResource*, const KfxWgpuNativeKey*, const KfxWgpuNativeResource*,
    KfxWgpuNativeOracle, void*) { return 0; }
extern "C" int kfx_wgpu_native_draw_sprite(const KfxGpolyTarget*, const KfxWgpuDrawCommand*,
    const KfxWgpuSpriteAssets*, KfxWgpuNativeOracle, void*) { return 0; }
extern "C" int kfx_wgpu_native_draw_parts(const KfxGpolyTarget*, const KfxWgpuDrawCommand*,
    const KfxWgpuNativeResource*, const KfxWgpuNativePart*, unsigned,
    const KfxWgpuNativeResource*, KfxWgpuNativeOracle, void*) { return 0; }
#endif
