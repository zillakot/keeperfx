#pragma once

#include "kfx/renderer/GpolyCapture.h"
#include "kfx/renderer/WgpuDraw.h"
#include "kfx/renderer/WgpuTargetResource.h"

struct TbSprite;
struct TbRect;
struct SSurface;

struct KfxWgpuCursorCounters {
    uint64_t sprite_draws, backups, compositions, failures;
    uint64_t bridge_initial_index_bytes, checkpoint_index_bytes, native_commit_bytes;
    uint64_t cpu_sprite_draws, cpu_backups, cpu_compositions;
    KfxWgpuDrawCounters gpu;
    KfxWgpuTargetResourceCounters copies;
};
KfxWgpuCursorCounters kfx_wgpu_cursor_counters();
enum KfxWgpuCursorSoftware { CursorSoftwareSprite, CursorSoftwareBackup, CursorSoftwareCompose };
void kfx_wgpu_cursor_software(KfxWgpuCursorSoftware operation);
int kfx_wgpu_cursor_direct(const KfxGpolyTarget& target, const TbSprite* sprite,
    const int32_t* xsteps, const int32_t* ysteps);

class WgpuCursor {
public:
    explicit WgpuCursor(void* borrowed_context = nullptr);
    ~WgpuCursor();
    WgpuCursor(const WgpuCursor&) = delete;
    WgpuCursor& operator=(const WgpuCursor&) = delete;
    bool Initialise(SSurface& surface, const TbSprite* sprite,
        const int32_t* xsteps, const int32_t* ysteps);
    bool Backup(SSurface& checkpoint, int x, int y, const TbRect& rect);
    bool Compose(int x, int y, const TbRect& rect, bool restore);
    // The borrowed context and target must share a device and outlive this cursor.
    bool InitialiseTarget(uint32_t width, uint32_t height, const TbSprite* sprite,
        const int32_t* xsteps, const int32_t* ysteps);
    bool BackupTarget(uint64_t target, uint32_t width, uint32_t height,
        int x, int y, const TbRect& rect);
    bool ComposeTarget(uint64_t target, uint32_t width, uint32_t height,
        int x, int y, const TbRect& rect, bool restore);
private:
    struct State;
    State* state;
};
