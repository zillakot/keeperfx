use super::*;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_target_snapshot(
    drawing: *mut c_void,
    target: u64,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    pitch: u32,
    error: *mut c_char,
    capacity: usize,
) -> u64 {
    unsafe {
        boundary(error, capacity, || {
            ensure!(!drawing.is_null(), "null drawing context");
            (&mut *drawing.cast::<crate::draw::DrawRenderer>())
                .create_target_snapshot(target, x, y, width, height, pitch)
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_target_snapshot_release(
    drawing: *mut c_void,
    snapshot: u64,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        let result = boundary(error, capacity, || {
            ensure!(!drawing.is_null(), "null drawing context");
            (&mut *drawing.cast::<crate::draw::DrawRenderer>())
                .release_target_snapshot(snapshot)?;
            Ok(Some(1))
        });
        result.unwrap_or(-1)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_submit_target_images(
    drawing: *mut c_void,
    target: u64,
    commands: *const crate::draw::Command,
    count: usize,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        let result = boundary(error, capacity, || {
            ensure!(
                !drawing.is_null() && (!commands.is_null() || count == 0),
                "null drawing context or commands"
            );
            ensure!(count <= 262_144, "snapshot command count exceeds limit");
            let commands = if count == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(commands, count)
            };
            (&mut *drawing.cast::<crate::draw::DrawRenderer>())
                .submit_target_images(target, commands)?;
            Ok(Some(1))
        });
        result.unwrap_or(-1)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_target_resource_counters(
    drawing: *mut c_void,
    output: *mut crate::draw::TargetResourceCounters,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        let result = boundary(error, capacity, || {
            ensure!(
                !drawing.is_null() && !output.is_null(),
                "null drawing context or counters"
            );
            let drawing = &*drawing.cast::<crate::draw::DrawRenderer>();
            drawing.check_status()?;
            *output = drawing.target_resource_counters();
            Ok(Some(1))
        });
        result.unwrap_or(-1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_ffi_rejects_null_arguments() {
        let mut error = [0; 128];
        unsafe {
            assert_eq!(
                kfx_wgpu_draw_target_snapshot(
                    std::ptr::null_mut(),
                    1,
                    0,
                    0,
                    1,
                    1,
                    1,
                    error.as_mut_ptr(),
                    error.len()
                ),
                0
            );
            assert_ne!(error[0], 0);
            assert_eq!(
                kfx_wgpu_draw_target_snapshot_release(
                    std::ptr::null_mut(),
                    1,
                    error.as_mut_ptr(),
                    error.len()
                ),
                -1
            );
            assert_eq!(
                kfx_wgpu_draw_submit_target_images(
                    std::ptr::null_mut(),
                    1,
                    std::ptr::null(),
                    0,
                    error.as_mut_ptr(),
                    error.len()
                ),
                -1
            );
            assert_eq!(
                kfx_wgpu_draw_target_resource_counters(
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    error.as_mut_ptr(),
                    error.len()
                ),
                -1
            );
        }
    }
}
