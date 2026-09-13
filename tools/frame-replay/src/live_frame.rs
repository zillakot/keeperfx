use super::*;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_target_view(
    handle: *mut c_void,
    parent: u64,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    error: *mut c_char,
    capacity: usize,
) -> u64 {
    unsafe {
        boundary(error, capacity, || {
            ensure!(!handle.is_null(), "null drawing context");
            (*handle.cast::<crate::draw::DrawRenderer>())
                .create_target_view(parent, x, y, width, height)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_frame_begin(
    handle: *mut c_void,
    root: u64,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        let result: Option<i32> = boundary(error, capacity, || {
            ensure!(!handle.is_null(), "null drawing context");
            (*handle.cast::<crate::draw::DrawRenderer>()).frame_begin(root)?;
            Ok(Some(1))
        });
        result.unwrap_or(-1)
    }
}

macro_rules! frame_call {
    ($name:ident, $method:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            handle: *mut c_void,
            error: *mut c_char,
            capacity: usize,
        ) -> i32 {
            unsafe {
                let result: Option<i32> = boundary(error, capacity, || {
                    ensure!(!handle.is_null(), "null drawing context");
                    (*handle.cast::<crate::draw::DrawRenderer>()).$method()?;
                    Ok(Some(1))
                });
                result.unwrap_or(-1)
            }
        }
    };
}
frame_call!(kfx_wgpu_draw_frame_flush, frame_flush);
frame_call!(kfx_wgpu_draw_frame_end, frame_end);
frame_call!(kfx_wgpu_draw_frame_abort, frame_abort);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_frame_counters(
    handle: *mut c_void,
    output: *mut crate::draw::FrameCounters,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        let result: Option<i32> = boundary(error, capacity, || {
            ensure!(
                !handle.is_null() && !output.is_null(),
                "null frame counters"
            );
            *output = (*handle.cast::<crate::draw::DrawRenderer>()).frame_counters();
            Ok(Some(1))
        });
        result.unwrap_or(-1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{CLEAR, Command, FrameCounters, IMAGE, OPAQUE, RECT, SPRITE};
    use crate::live::target_resources::*;

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn gpu_c_abi_frame_offscreen_cursor_and_alias_checkpoints() {
        unsafe {
            let mut error = [0i8; 1024];
            let context = kfx_wgpu_draw_create(error.as_mut_ptr(), error.len());
            assert!(!context.is_null());
            macro_rules! call {
                ($function:ident $(, $argument:expr)*) => {
                    $function(context $(, $argument)*, error.as_mut_ptr(), error.len())
                };
            }
            macro_rules! accepted {
                ($expression:expr) => {
                    assert_eq!(
                        $expression,
                        1,
                        "{:?}",
                        std::ffi::CStr::from_ptr(error.as_ptr())
                    );
                };
            }
            let root = call!(kfx_wgpu_draw_target_create, 12, 8);
            let view = call!(kfx_wgpu_draw_target_view, root, 3, 2, 2, 2);
            let raster = call!(kfx_wgpu_draw_target_create, 2, 2);
            let background = call!(kfx_wgpu_draw_target_create, 2, 2);
            assert!(root != 0 && view != 0 && raster != 0 && background != 0);
            accepted!(call!(kfx_wgpu_draw_frame_begin, root));
            let clear = Command {
                kind: CLEAR,
                colour: 7,
                ..Default::default()
            };
            let world = Command {
                kind: RECT,
                colour: 19,
                width: 2,
                height: 2,
                ..Default::default()
            };
            accepted!(call!(kfx_wgpu_draw_submit, root, &clear, 1));
            accepted!(call!(kfx_wgpu_draw_submit, view, &world, 1));
            let mut bytes = vec![77, 1];
            for value in [0u32, 2, 0, 2] {
                bytes.extend(value.to_le_bytes());
            }
            bytes.extend(0..=255);
            let source = call!(
                kfx_wgpu_draw_resource_create,
                bytes.as_ptr(),
                bytes.len(),
                1,
                1,
                1
            );
            assert_ne!(source, 0);
            let sprite = Command {
                kind: SPRITE,
                source,
                source_width: 1,
                source_height: 1,
                width: 2,
                height: 2,
                clip_width: 2,
                clip_height: 2,
                ..Default::default()
            };
            let cursor = [
                Command {
                    colour: 255,
                    ..clear
                },
                sprite,
            ];
            accepted!(call!(
                kfx_wgpu_draw_submit,
                raster,
                cursor.as_ptr(),
                cursor.len()
            ));
            bytes.fill(0);
            let sprite_snapshot = call!(kfx_wgpu_draw_target_snapshot, raster, 0, 0, 2, 2, 2);
            assert_ne!(sprite_snapshot, 0);
            let mut counters = FrameCounters::default();
            accepted!(call!(kfx_wgpu_draw_frame_counters, &mut counters));
            assert_eq!(counters.checkpoints, 0);
            assert_eq!(counters.queued_commands, 2);
            let saved = call!(kfx_wgpu_draw_target_snapshot, view, 0, 0, 2, 2, 2);
            assert_ne!(saved, 0);
            let image = Command {
                kind: IMAGE,
                source: saved,
                width: 2,
                height: 2,
                source_width: 2,
                source_height: 2,
                transparent: OPAQUE,
                ..Default::default()
            };
            accepted!(call!(
                kfx_wgpu_draw_submit_target_images,
                background,
                &image,
                1
            ));
            let backup = call!(kfx_wgpu_draw_target_snapshot, background, 0, 0, 2, 2, 2);
            assert_ne!(backup, 0);
            let compose = Command {
                source: sprite_snapshot,
                ..image
            };
            accepted!(call!(kfx_wgpu_draw_submit_target_images, view, &compose, 1));
            let mut pixels = [0u8; 4];
            accepted!(call!(
                kfx_wgpu_draw_readback,
                view,
                pixels.as_mut_ptr(),
                pixels.len(),
                2
            ));
            assert_eq!(pixels, [77; 4]);
            accepted!(call!(kfx_wgpu_draw_frame_counters, &mut counters));
            assert_eq!(counters.checkpoints, 1);
            let interleaved = Command {
                colour: 90,
                width: 1,
                height: 1,
                ..world
            };
            accepted!(call!(kfx_wgpu_draw_submit, view, &interleaved, 1));
            let restore = Command {
                source: backup,
                ..image
            };
            accepted!(call!(kfx_wgpu_draw_submit_target_images, view, &restore, 1));
            let tail = Command {
                colour: 99,
                x: 1,
                y: 1,
                ..interleaved
            };
            accepted!(call!(kfx_wgpu_draw_submit, view, &tail, 1));
            let malformed = [
                Command { colour: 0, ..clear },
                Command {
                    kind: 0xffff,
                    ..world
                },
            ];
            assert_eq!(
                call!(
                    kfx_wgpu_draw_submit,
                    raster,
                    malformed.as_ptr(),
                    malformed.len()
                ),
                -1
            );
            accepted!(call!(
                kfx_wgpu_draw_readback,
                raster,
                pixels.as_mut_ptr(),
                pixels.len(),
                2
            ));
            assert_eq!(pixels, [77; 4]);
            accepted!(call!(kfx_wgpu_draw_resource_release, source));
            assert_eq!(call!(kfx_wgpu_draw_submit, raster, &sprite, 1), -1);
            let dead = call!(kfx_wgpu_draw_target_view, raster, 0, 0, 1, 1);
            accepted!(call!(kfx_wgpu_draw_target_release, dead));
            assert_eq!(call!(kfx_wgpu_draw_target_snapshot, dead, 0, 0, 1, 1, 1), 0);
            accepted!(call!(kfx_wgpu_draw_frame_counters, &mut counters));
            assert_eq!(counters.checkpoints, 2);
            assert_eq!(counters.validation_waits, 0);
            accepted!(call!(kfx_wgpu_draw_frame_end));
            accepted!(call!(kfx_wgpu_draw_frame_counters, &mut counters));
            assert_eq!(counters.checkpoints, 3);
            assert_eq!(counters.queued_commands, 4);
            let mut final_pixels = [0u8; 12 * 8];
            accepted!(call!(
                kfx_wgpu_draw_readback,
                root,
                final_pixels.as_mut_ptr(),
                final_pixels.len(),
                12
            ));
            let mut expected = [7u8; 12 * 8];
            expected[2 * 12 + 3] = 19;
            expected[2 * 12 + 4] = 19;
            expected[3 * 12 + 3] = 19;
            expected[3 * 12 + 4] = 99;
            assert_eq!(final_pixels, expected);
            for snapshot in [sprite_snapshot, saved, backup] {
                accepted!(call!(kfx_wgpu_draw_target_snapshot_release, snapshot));
            }
            for target in [view, raster, background, root] {
                accepted!(call!(kfx_wgpu_draw_target_release, target));
            }
            kfx_wgpu_draw_destroy(context);
        }
    }
}
