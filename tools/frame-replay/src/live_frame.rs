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
