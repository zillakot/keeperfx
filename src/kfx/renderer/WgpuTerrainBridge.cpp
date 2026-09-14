#include "kfx/renderer/WgpuTerrainBridge.h"
#include "kfx/renderer/WgpuShadow.h"
#include "kfx/renderer/WgpuTargetResource.h"
#include "kfx/renderer/KfxWgpuFrame.h"
#ifdef KFX_RUST_PRESENTER
#include "kfx/renderer/WgpuDraw.h"
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
    if (active_bridge == nullptr || active_bridge->IsOracleActive() || target == nullptr || command == nullptr) return 0;
    return active_bridge->SubmitNative(*target, *command, source, table, oracle, oracle_context);
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
        if (m_frame_active && !FrameView(target, view_x, view_y) && !EndFrame(true)) return Fail(nullptr);
        m_native_target = target;
        if (!ValidateCpuLease() || !PrepareNativeTarget()) { Fail(nullptr); return 0; }
        uint64_t snapshot = kfx_wgpu_draw_target_snapshot(m_context, SubmissionTarget(), 0, 0,
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
    // The shadow route accepts only its own command, so close any pending run first.
    Flush();
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

uint64_t WgpuTerrainBridge::SubmissionTarget()
{
    uint32_t x, y;
    if (!FrameView(m_native_target, x, y)) return m_target;
    if (!x && !y && m_native_target.width == m_width && m_native_target.height == m_height)
        return m_target;
    for (const auto& view : m_views)
        if (view.x == x && view.y == y && view.width == m_native_target.width && view.height == m_native_target.height)
            return view.handle;
    const uint64_t handle = kfx_wgpu_draw_target_view(m_context, m_target, x, y,
        m_native_target.width, m_native_target.height, m_error.data(), m_error.size());
    if (handle) m_views.push_back({x, y, m_native_target.width, m_native_target.height, handle});
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

void WgpuTerrainBridge::FullRedraw()
{
    if (m_queue_active) kfx_wgpu_draw_frame_abort(m_context, m_error.data(), m_error.size());
    m_queue_active = false;
    m_frame_active = false;
    ClearPending();
    m_resident_lease = false;
    m_allow_terrain = false;
    m_frame_invalid = false;
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
    for (const auto& resource : m_textures)
        kfx_wgpu_draw_resource_release(m_context, resource.handle, m_error.data(), m_error.size());
    for (const auto& resource : m_fades)
        kfx_wgpu_draw_resource_release(m_context, resource.handle, m_error.data(), m_error.size());
    if (m_target) kfx_wgpu_draw_target_release(m_context, m_target, m_error.data(), m_error.size());
    m_textures.clear();
    m_fades.clear();
    m_context = nullptr;
    m_target = 0;
    m_width = m_height = 0;
    m_borrowed_context = false;
}

uint64_t WgpuTerrainBridge::ResidentTarget(const KfxGpolyTarget& target)
{
    Flush();
    if (m_queue_active && kfx_wgpu_draw_frame_flush(m_context, m_error.data(), m_error.size()) != 1) {
        Fail(nullptr);
        return 0;
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

uint64_t WgpuTerrainBridge::ResourceFor(std::vector<Resource>& cache, const void* key,
    uint64_t generation, const uint8_t* bytes, size_t length, uint32_t width, uint32_t height,
    uint32_t pitch, size_t limit)
{
    for (const auto& resource : cache) {
        if (resource.key != nullptr && resource.generation != generation) continue;
        if (resource.width != width || resource.height != height || resource.pitch != pitch ||
            resource.bytes.size() != length) continue;
        if (key != nullptr ? resource.key == key && resource.generation == generation
                           : resource.key == nullptr &&
                             std::memcmp(resource.bytes.data(), bytes, length) == 0)
            return resource.handle;
    }
    if (cache.size() >= limit) {
        Flush();
        if (m_failed) return 0;
        if (kfx_wgpu_draw_resource_release(m_context, cache.front().handle,
                m_error.data(), m_error.size()) != 1) return 0;
        cache.erase(cache.begin());
    }
    Resource resource = {0, std::vector<uint8_t>(bytes, bytes + length), width, height, pitch,
        key, generation};
    resource.handle = kfx_wgpu_draw_resource_create(m_context, resource.bytes.data(), length,
        width, height, pitch, m_error.data(), m_error.size());
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
    if (PendingTargetChanged(target) || m_pending.size() >= kPendingSpanLimit) {
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
    if (m_frame_active && !FrameView(target, view_x, view_y) && !EndFrame(true)) return Fail(nullptr);
    m_native_target = target;
    if (!ReadBarrier(texture, KFX_GPOLY_TEXTURE_BYTES) || !ReadBarrier(fade, KFX_GPOLY_FADE_BYTES))
        return KFX_GPOLY_DECLINED;
    std::array<uint8_t, KFX_GPOLY_TEXTURE_BYTES> texture_bytes = {};
    for (size_t row = 0; row < 32; ++row)
        std::memcpy(texture_bytes.data() + row * 256, texture + row * 256, 32);
    const uint64_t texture_handle = ResourceFor(m_textures, StableKey(texture, TEXTURE_READ_BYTES),
        kfx_render_asset_generation, texture_bytes.data(), texture_bytes.size(), 32, 32, 256, 64);
    if (texture_handle == 0) return Fail(nullptr);
    const uint64_t fade_handle = ResourceFor(m_fades, StableKey(fade, KFX_GPOLY_FADE_BYTES),
        kfx_render_asset_generation, fade, KFX_GPOLY_FADE_BYTES, 256, 64, 256, 4);
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
    if ((m_verify && !m_pending.empty()) || m_triangles.size() >= 128 ||
        PendingTargetChanged(target) || (m_rasterizer && m_rasterizer != rasterizer)) Flush();
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
    if (m_frame_active && !FrameView(target, view_x, view_y) && !EndFrame(true)) return Fail(nullptr);
    m_native_target = target;
    m_rasterizer = rasterizer;
    if (!ReadBarrier(texture, KFX_GPOLY_TEXTURE_BYTES) || !ReadBarrier(fade, KFX_GPOLY_FADE_BYTES))
        return KFX_GPOLY_DECLINED;
    std::array<uint8_t, KFX_GPOLY_TEXTURE_BYTES> texture_bytes = {};
    for (size_t row = 0; row < 32; ++row)
        std::memcpy(texture_bytes.data() + row * 256, texture + row * 256, 32);
    KfxWgpuTriangle owned = triangle;
    owned.source = ResourceFor(m_textures, StableKey(texture, TEXTURE_READ_BYTES),
        kfx_render_asset_generation, texture_bytes.data(), texture_bytes.size(), 32, 32, 256, 64);
    if (!owned.source) return Fail(nullptr);
    owned.table = ResourceFor(m_fades, StableKey(fade, KFX_GPOLY_FADE_BYTES),
        kfx_render_asset_generation, fade, KFX_GPOLY_FADE_BYTES, 256, 64, 256, 4);
    if (!owned.table) return Fail(nullptr);
    AppendTriangle(owned);
    return KFX_GPOLY_CONSUMED;
}

bool WgpuTerrainBridge::PendingIsReplayable() const
{
    // Generic commands keep their oracle in the caller, so a mixed run has no CPU replay.
    for (const auto& command : m_pending)
        if (command.kind != KFX_WGPU_DRAW_GPOLY_SPAN) return false;
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
            if (run.triangles) {
                const auto& triangle = m_triangles[triangles++];
                for (const auto& resource : m_textures)
                    if (resource.handle == triangle.source) texture = resource.bytes.data();
                for (const auto& resource : m_fades)
                    if (resource.handle == triangle.table) fade = resource.bytes.data();
                KfxGpolyTarget target = {pixels, m_native_target.width, m_native_target.height, pitch};
                if (!m_rasterizer(&target, &triangle, texture, fade)) return false;
                continue;
            }
            const auto& command = m_pending[commands++];
            for (const auto& resource : m_textures)
                if (resource.handle == command.source) texture = resource.bytes.data();
            for (const auto& resource : m_fades)
                if (resource.handle == command.table) fade = resource.bytes.data();
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
    if (m_order.empty() || m_order.back().triangles) m_order.push_back({false, 0});
    ++m_order.back().count;
    m_pending.push_back(command);
    m_pending_sources.push_back(source);
}

void WgpuTerrainBridge::AppendTriangle(const KfxWgpuTriangle& triangle)
{
    if (m_order.empty() || !m_order.back().triangles) m_order.push_back({true, 0});
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
    const uint64_t destination = SubmissionTarget();
    if (!destination) return false;
    if (m_resident_lease) m_gpu_dirty = true;
    std::vector<uint8_t> shadow_mirror;
    size_t routes = 0;
    if (m_shadow_scratch != nullptr) {
        if (m_pending.size() != 1 || !m_triangles.empty() ||
            m_pending[0].kind != KFX_WGPU_DRAW_SHADOW) {
            std::snprintf(m_error.data(), m_error.size(), "shadow submission requires a solo batch");
            return false;
        }
        shadow_mirror.resize(65536);
        if (kfx_wgpu_draw_submit_shadow(m_context, destination, m_pending.data(), shadow_mirror.data(),
                shadow_mirror.size(), m_error.data(), m_error.size()) != 1) return false;
        m_counts.shadow_scratch_upload_bytes += 65536;
        m_counts.shadow_scratch_readback_bytes += 65536 * sizeof(uint32_t);
        routes = 1;
    } else if (m_pending.size() == 1 && m_triangles.empty() &&
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
            if (run.triangles) {
                if (kfx_wgpu_draw_submit_triangles(m_context, destination, m_triangles.data() + triangles,
                        run.count, m_error.data(), m_error.size()) != 1) return false;
                triangles += run.count;
            } else if (kfx_wgpu_draw_submit(m_context, destination, m_pending.data() + commands,
                    run.count, m_error.data(), m_error.size()) != 1) return false;
            else commands += run.count;
            ++routes;
        }
    }
    if (m_shadow_scratch != nullptr ||
        (m_pending.size() == 1 && NeedsSoloBatch(m_pending[0]))) ++m_counts.bridge_solo_batches;
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
        if (oracle != nullptr) {
            std::vector<uint8_t> saved_scratch;
            if (m_shadow_scratch != nullptr) saved_scratch.assign(m_shadow_scratch, m_shadow_scratch + 65536);
            m_oracle_active = true;
            oracle(expected.data() + ExpectedOffset(), m_width, oracle_context);
            m_oracle_active = false;
            if (m_shadow_scratch != nullptr) {
                const bool same = std::memcmp(m_shadow_scratch, shadow_mirror.data(), 65536) == 0;
                std::memcpy(m_shadow_scratch, saved_scratch.data(), 65536);
                if (!same) {
                    std::snprintf(m_error.data(), m_error.size(), "GPU shadow scratch index comparison failed");
                    return false;
                }
            }
            m_counts.verification_cpu_commands += m_pending.size();
        } else {
            if (!RasterizePending(expected.data() + ExpectedOffset(), m_width)) {
                std::snprintf(m_error.data(), m_error.size(), "invalid triangle shade in native oracle");
                return false;
            }
            m_counts.verification_cpu_spans += m_pending.size();
        }
        if (expected != m_readback) {
            std::snprintf(m_error.data(), m_error.size(), "GPU terrain index comparison failed");
            return false;
        }
        if (m_resident_lease) m_expected = std::move(expected);
        m_counts.verified_triangles += m_triangles.size();
        ++m_counts.verified_batches;
    }
    if (m_shadow_scratch != nullptr) {
        std::memcpy(m_shadow_scratch, shadow_mirror.data(), 65536);
        ++m_counts.gpu_shadow_commands;
        m_counts.shadow_scratch_copy_bytes += 65536;
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

bool WgpuTerrainBridge::NeedsSoloBatch(const KfxWgpuDrawCommand& command)
{
    return !(command.kind <= KFX_WGPU_DRAW_TRIG || command.kind == KFX_WGPU_DRAW_MOVIE ||
        command.kind == KFX_WGPU_DRAW_MAP_VIEW || command.kind == KFX_WGPU_DRAW_BITMAP);
}

bool WgpuTerrainBridge::OrderedSprite(const KfxWgpuDrawCommand& command)
{
    /* Bit 3 of source_x marks the serial row-copy ordered sprite path. */
    return command.kind == KFX_WGPU_DRAW_SPRITE && (command.source_x & 8u) != 0;
}

bool WgpuTerrainBridge::PendingTargetChanged(const KfxGpolyTarget& target) const
{
    return m_native_target.pixels != target.pixels || m_native_target.width != target.width ||
        m_native_target.height != target.height || m_native_target.pitch != target.pitch;
}

void WgpuTerrainBridge::EmitterBoundary()
{
    if (m_resident_lease && !m_verify && !m_failed) return;
    Flush();
}

int WgpuTerrainBridge::SubmitNative(const KfxGpolyTarget& target,
    const KfxWgpuDrawCommand& command, const KfxWgpuNativeResource* source,
    const KfxWgpuNativeResource* table, KfxWgpuNativeOracle oracle, void* oracle_context)
{
    if (m_oracle_active) return 0;
    if (!m_pending.empty() || !m_triangles.empty()) {
        if (m_verify || !m_resident_lease || NeedsSoloBatch(command) || OrderedSprite(command) ||
            m_pending.size() >= kPendingLimit || PendingTargetChanged(target)) Flush();
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
        if (m_frame_active && !FrameView(target, view_x, view_y) && !EndFrame(true)) return Fail(nullptr);
        m_native_target = target;
        auto aliases_target = [&](const KfxWgpuNativeResource* resource) {
            if (!resource || !resource->bytes || !resource->length || !m_gpu_dirty) return false;
            const uintptr_t first = reinterpret_cast<uintptr_t>(resource->bytes);
            const uintptr_t screen = reinterpret_cast<uintptr_t>(m_gpu_native_target.pixels);
            const size_t length = static_cast<size_t>(m_gpu_native_target.pitch) * (m_gpu_native_target.height - 1) + m_gpu_native_target.width;
            return first <= screen ? screen - first < resource->length : first - screen < length;
        };
        if (aliases_target(source) || aliases_target(table)) {
            ++m_counts.target_alias_barriers;
            if ((source && !ReadBarrier(source->bytes, source->length)) ||
                (table && !ReadBarrier(table->bytes, table->length))) return Fail(nullptr);
        }
        KfxWgpuDrawCommand owned = command;
        OwnedResource guard = {m_context, 0};
        if (source != nullptr) {
            guard.handle = kfx_wgpu_draw_resource_create(m_context, source->bytes, source->length,
                source->width, source->height, source->pitch, m_error.data(), m_error.size());
            if (guard.handle == 0) return Fail(nullptr);
            m_counts.resource_snapshot_bytes += source->length;
        }
        source_handle = guard.handle;
        if (table != nullptr) {
            table_handle = ResourceFor(m_fades, nullptr, 0, table->bytes,
                table->length, table->width, table->height, table->pitch, 8);
        }
        const bool resources_ready = table == nullptr || table_handle != 0;
        owned.source = command.kind == KFX_WGPU_DRAW_TRANSITION ? command.source : source_handle;
        owned.table = table_handle;
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
#endif
