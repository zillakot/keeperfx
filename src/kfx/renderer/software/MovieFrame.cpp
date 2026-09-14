#include "kfx/renderer/software/MovieFrame.h"
#include "bflib_fmvids.h"
#include "kfx/renderer/WgpuTerrainBridge.h"
#include <algorithm>
#include <cstring>
using std::min;
using std::max;

namespace {
bool oracle_active;
struct MovieOracle { KfxMovieFrame frame; KfxMovieTarget target; int flags; bool scaled; };

void movie_oracle(uint8_t *pixels, uint32_t pitch, void *context)
{
    auto &oracle = *static_cast<MovieOracle *>(context);
    auto target = oracle.target;
    target.pixels = pixels;
    target.pitch = pitch;
    oracle_active = true;
    if (oracle.scaled) kfx_movie_copy_scaled(oracle.frame, target, oracle.flags);
    else kfx_movie_copy(oracle.frame, target, oracle.flags);
    oracle_active = false;
}

bool movie_submit(const KfxMovieFrame &frame, const KfxMovieTarget &target, int flags,
    bool scaled, int x, int y, int width, int height)
{
    if (oracle_active || !kfx_wgpu_native_enabled()) return false;
    kfx_wgpu_native_flush();
    if (!target.pixels || !frame.pixels || frame.width <= 0 || frame.height <= 0 ||
        frame.pitch < frame.width || frame.pitch > 16384 || frame.width > 8192 || frame.height > 8192 ||
        target.pitch <= 0 || target.pitch > 8192 || target.height <= 0 || target.height > 8192 ||
        width < 0 || height < 0 || width > 16384 || height > 16384 ||
        x < -16384 || x > 16384 || y < -16384 || y > 16384) return false;
    const size_t length = size_t(frame.pitch) * frame.height;
    const size_t capacity = size_t(target.pitch) * target.height;
    uintptr_t a = reinterpret_cast<uintptr_t>(frame.pixels), b = reinterpret_cast<uintptr_t>(target.pixels);
    if (length > 16 * 1024 * 1024 || (a < b ? length > b - a : capacity > a - b)) return false;
    unsigned mode = ((flags & SMK_PixelDoubleWidth) ? 1u : 0u) |
        ((flags & SMK_PixelDoubleLine) ? 2u : 0u) | ((flags & SMK_InterlaceLine) ? 4u : 0u);
    if (!scaled && (mode & 3) && frame.width < 4) return false;
    if (!scaled) {
        const int copied_width = ((mode & 3) ? (frame.width / 4) * 4 : frame.width) *
            ((mode & 1) ? 2 : 1);
        const int64_t begin = int64_t(y) * target.pitch + x;
        const int64_t end = begin + int64_t(frame.height - 1) * target.pitch *
            ((mode & 6) ? 2 : 1) + ((mode & 2) ? (target.pitch / 4) * 4 : 0) + copied_width;
        if (copied_width > target.pitch || begin < 0 || end > int64_t(capacity)) return false;
    }
    KfxGpolyTarget surface = {target.pixels, uint32_t(target.pitch), uint32_t(target.height), uint32_t(target.pitch)};
    KfxWgpuNativeResource source = {frame.pixels, length, uint32_t(frame.width), uint32_t(frame.height), uint32_t(frame.pitch), nullptr, 0, 0};
    KfxWgpuDrawCommand command = {};
    command.abi_version = KFX_WGPU_DRAW_ABI_VERSION;
    command.kind = scaled ? KFX_WGPU_DRAW_RAW_IMAGE : KFX_WGPU_DRAW_MOVIE;
    command.width = command.clip_width = target.pitch;
    command.height = command.clip_height = target.height;
    command.transparent = KFX_WGPU_DRAW_OPAQUE;
    command.source_width = frame.width;
    command.source_height = frame.height;
    command.start_low = x;
    command.start_high = y;
    command.step_low = width;
    command.step_high = height;
    if (!scaled) command.source_x = mode;
    if (scaled && (!width || !height)) command.kind = KFX_WGPU_DRAW_CLEAR;
    MovieOracle context = {frame, target, flags, scaled};
    return kfx_wgpu_native_draw(&surface, &command,
        command.kind == KFX_WGPU_DRAW_CLEAR ? nullptr : &source, nullptr, movie_oracle, &context) != 0;
}

void copy_to_screen_pxquad(unsigned char *srcbuf, unsigned char *dstbuf, long width, long dst_shift)
{
	const auto s = dst_shift >> 2;
	auto w = ((uint32_t)width) >> 2;
	auto * src = reinterpret_cast<uint32_t *>(srcbuf);
	auto * dst = reinterpret_cast<uint32_t *>(dstbuf);
	do {
		const auto c = *src++;
		const auto first_pixel_low_byte = c & 0xFF;
		const auto first_pixel_high_byte = (c >> 8) & 0xFF;
		const auto first_doubled_pixel = (first_pixel_high_byte << 24) + (first_pixel_high_byte << 16) + (first_pixel_low_byte << 8) + first_pixel_low_byte;
		dst[0] = first_doubled_pixel;
		dst[s] = first_doubled_pixel;
		const auto second_pixel_low_byte = (c >> 16) & 0xFF;
		const auto second_pixel_high_byte = (c >> 24) & 0xFF;
		const auto second_doubled_pixel = (second_pixel_high_byte << 24) + (second_pixel_high_byte << 16) + (second_pixel_low_byte << 8) + second_pixel_low_byte;
		dst[1] = second_doubled_pixel;
		dst[s+1] = second_doubled_pixel;
		dst += 2;
		w--;
	}
	while (w > 0);
}

void copy_to_screen_pxdblh(unsigned char *srcbuf, unsigned char *dstbuf, long width, long dst_shift)
{
	const auto s = dst_shift >> 2;
	auto w = ((unsigned long)width) >> 2;
	auto src = (uint32_t *)srcbuf;
	auto dst = (uint32_t *)dstbuf;
	do {
		const auto n = *src++;
		dst[0] = n;
		dst[s] = n;
		dst++;
		w--;
	}
	while (w > 0);
}

void copy_to_screen_pxdblw(unsigned char *srcbuf, unsigned char *dstbuf, long width)
{
	auto w = ((unsigned long)width) >> 2;
	auto src = (uint32_t *)srcbuf;
	auto dst = (uint32_t *)dstbuf;
	do {
		const auto c = *src++;
		const auto first_pixel_low_byte = c & 0xFF;
		const auto first_pixel_high_byte = (c >> 8) & 0xFF;
		dst[0] = (first_pixel_high_byte << 24) + (first_pixel_high_byte << 16) + (first_pixel_low_byte << 8) + first_pixel_low_byte;
		const auto second_pixel_low_byte = (c >> 16) & 0xFF;
		const auto second_pixel_high_byte = (c >> 24) & 0xFF;
		dst[1] = (second_pixel_high_byte << 24) + (second_pixel_high_byte << 16) + (second_pixel_low_byte << 8) + second_pixel_low_byte;
		dst += 2;
		w--;
	}
	while (w > 0);
}

}

void kfx_movie_copy(const KfxMovieFrame & frame, const KfxMovieTarget & target, const int flags)
{
	const auto src_pitch = frame.pitch;
	auto srcbuf = frame.pixels;
	long screen_buffer_center_offset;
	if (flags & (SMK_PixelDoubleLine | SMK_InterlaceLine)) {
		screen_buffer_center_offset = target.pitch * ((target.view_height - 2 * frame.height) >> 1);
	} else {
		screen_buffer_center_offset = target.pitch * ((target.view_height - frame.height) >> 1);
	}
	auto w = frame.width;
	if (flags & SMK_PixelDoubleWidth) {
		w = 2 * frame.width;
	}
	if (movie_submit(frame, target, flags, false, (target.view_width - w) >> 1,
        screen_buffer_center_offset / target.pitch, w, frame.height)) return;
    if (!oracle_active && !kfx_wgpu_native_cpu_barrier()) return;
	auto dstbuf = &target.pixels[screen_buffer_center_offset + ((target.view_width - w) >> 1)];
	if (flags & SMK_PixelDoubleLine) {
		if (flags & SMK_PixelDoubleWidth) {
			for (int h = frame.height; h > 0; h--) {
				copy_to_screen_pxquad(srcbuf, dstbuf, frame.width, target.pitch);
				dstbuf += 2 * target.pitch;
				srcbuf += src_pitch;
			}
		} else {
			for (int h = frame.height; h > 0; h--) {
				copy_to_screen_pxdblh(srcbuf, dstbuf, frame.width, target.pitch);
				dstbuf += 2 * target.pitch;
				srcbuf += src_pitch;
			}
		}
	} else {
		if (flags & SMK_PixelDoubleWidth) {
				if (flags & SMK_InterlaceLine) {
					for (int h = frame.height; h > 0; h--) {
						copy_to_screen_pxdblw(srcbuf, dstbuf, frame.width);
						dstbuf += 2 * target.pitch;
						srcbuf += src_pitch;
					}
				} else {
					for (int h = frame.height; h > 0; h--) {
						copy_to_screen_pxdblw(srcbuf, dstbuf, frame.width);
						dstbuf += target.pitch;
						srcbuf += src_pitch;
					}
				}
		} else if (flags & SMK_InterlaceLine) {
			for (int h = frame.height; h > 0; h--) {
				memcpy(dstbuf, srcbuf, frame.width);
				dstbuf += 2 * target.pitch;
				srcbuf += src_pitch;
			}
		} else {
			for (int h = frame.height; h > 0; h--) {
				memcpy(dstbuf, srcbuf, frame.width);
				dstbuf += target.pitch;
				srcbuf += src_pitch;
			}
		}
	}
}

void kfx_movie_copy_scaled(const KfxMovieFrame & frame, const KfxMovieTarget & target, const int flags)
{
	const auto src_pitch = frame.pitch;
	const auto src_buf = frame.pixels;
	const auto dst_buf = &target.pixels[0];
	const int scanline = target.pitch;
	const int nlines = target.height;
	int spw = 0;
	int sph = 0;
	int dst_width = 0;
	int dst_height = 0;

	if ((flags & SMK_FullscreenStretch) && !(flags & SMK_FullscreenFit)) {
		dst_width = scanline;
		dst_height = nlines;
	} else {
		int in_width = frame.width;
		int in_height = frame.height;
		float units_per_px = 0;
		const float relative_ar_difference = (in_width * 1.0 / in_height * 1.0) / (scanline * 1.0 / nlines * 1.0);
		float comparison_ratio = 1;
		if ((flags & SMK_FullscreenStretch) && (flags & SMK_FullscreenFit)) {
			if (frame.width == 320 && frame.height == 200) {
				in_height = (int)(in_height * 1.2);
			}
		}
		if ((flags & SMK_FullscreenCrop) && !(flags & SMK_FullscreenFit)) {
			comparison_ratio = relative_ar_difference;
		} else {
			comparison_ratio = 1.0 / relative_ar_difference;
		}
		if (comparison_ratio <= 1.0) {
			units_per_px = (scanline>nlines?scanline:nlines)/((in_width>in_height?in_width:in_height)/16.0);
		} else {
			units_per_px = (scanline>nlines?nlines:scanline)/((in_width>in_height?in_height:in_width)/16.0);
		}
		if ((flags & SMK_FullscreenCrop) && (flags & SMK_FullscreenFit)) {
			if (flags & SMK_FullscreenStretch) {
				if (frame.width == 320 && frame.height == 200) {
					units_per_px = (max(5, (int)(units_per_px / 16.0 / 5.0) * 5) * 16);
				}
			}
			units_per_px = ((int)(units_per_px / 16.0) * 16);
		}
		spw = (int)((scanline - in_width * units_per_px / 16.0) / 2.0);
		sph = (int)((nlines - in_height * units_per_px / 16.0) / 2.0);
		dst_width = (int)(in_width * units_per_px / 16.0);
		dst_height = (int)(in_height * units_per_px / 16.0);
	}

    if (movie_submit(frame, target, flags, true, spw, sph, dst_width, dst_height)) return;
    if (!oracle_active && !kfx_wgpu_native_cpu_barrier()) return;

	for (int sh = 0; sh < sph; sh++) {
		memset(&dst_buf[sh * scanline], 0, scanline);
	}
	for (int sh = sph + dst_height; sh < nlines; sh++) {
		memset(&dst_buf[sh * scanline], 0, scanline);
	}
	auto dhstart = sph;
	for (int sh = 0; sh < frame.height; sh++) {
		const auto dhend = sph + (dst_height * (sh + 1) / frame.height);
		const auto src = &src_buf[sh * src_pitch];
		const auto mhmin = max(0, -dhstart);
		const auto mhmax = min(dhend - dhstart, nlines - dhstart);
		for (int k = mhmin; k < mhmax; k++) {
			const auto dst = &dst_buf[(dhstart + k) * scanline];
			int dwstart = spw;
			if (dwstart > 0) {
				memset(dst, 0, dwstart);
			}
			for (int sw = 0; sw < frame.width; sw++) {
				const auto dwend = spw + (dst_width * (sw + 1) / frame.width);
				const auto mwmin = max(0, -dwstart);
				const auto mwmax = min(dwend - dwstart, scanline - dwstart);
				for (int i = mwmin; i < mwmax; i++) {
					dst[dwstart+i] = src[sw];
				}
				dwstart = dwend;
			}
			if (dwstart < scanline) {
				memset(dst+dwstart, 0, scanline-dwstart);
			}
		}
		dhstart = dhend;
	}
}
