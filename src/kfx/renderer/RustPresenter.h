#pragma once
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Main thread only. The caller owns the Metal layer until destroy returns.
 * Input buffers are borrowed for submit only; handles must not be reused after destroy.
 * Errors are NUL-terminated UTF-8. submit: 1 ready, 0 skipped, -1 terminal failure. */
void* kfx_wgpu_create(void* layer, uint32_t width, uint32_t height, int32_t vsync, char* error, size_t capacity);
int32_t kfx_wgpu_submit(void* handle, const uint8_t* indices, size_t length,
    uint32_t width, uint32_t height, uint32_t pitch, const uint8_t* palette, size_t palette_length,
    uint32_t output_width, uint32_t output_height, int32_t vsync, char* error, size_t capacity);
int32_t kfx_wgpu_present(void* handle, char* error, size_t capacity);
int32_t kfx_wgpu_details(void* handle, char* text, size_t capacity);
void kfx_wgpu_destroy(void* handle);
void kfx_wgpu_allocation_counts(uint64_t* allocations, uint64_t* requested_bytes);

#ifdef __cplusplus
}
#endif
