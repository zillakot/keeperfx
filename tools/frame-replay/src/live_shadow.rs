use super::*;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_submit_shadow(
    drawing: *mut c_void,
    target: u64,
    command: *const crate::draw::Command,
    mirror: *mut u8,
    length: usize,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        boundary(error, capacity, || {
            ensure!(
                !drawing.is_null() && !command.is_null() && !mirror.is_null() && length == 65536,
                "invalid shadow arguments"
            );
            (&mut *drawing.cast::<crate::draw::DrawRenderer>()).submit_shadow(
                target,
                &*command,
                std::slice::from_raw_parts_mut(mirror, length),
            )?;
            Ok(Some(1))
        })
        .unwrap_or(-1)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn kfx_wgpu_draw_submit_target_triangles(
    drawing: *mut c_void,
    target: u64,
    commands: *const crate::draw::Command,
    count: usize,
    texture: u64,
    error: *mut c_char,
    capacity: usize,
) -> i32 {
    unsafe {
        boundary(error, capacity, || {
            ensure!(
                !drawing.is_null() && (!commands.is_null() || count == 0) && count <= 262144,
                "invalid snapshot triangle arguments"
            );
            let commands = if count == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(commands, count)
            };
            (&mut *drawing.cast::<crate::draw::DrawRenderer>())
                .submit_target_triangles(target, commands, texture)?;
            Ok(Some(1))
        })
        .unwrap_or(-1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shadow_ffi_rejects_null_arguments() {
        let mut error = [0; 128];
        unsafe {
            assert_eq!(
                kfx_wgpu_draw_submit_shadow(
                    std::ptr::null_mut(),
                    1,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    65536,
                    error.as_mut_ptr(),
                    error.len()
                ),
                -1
            );
            assert_eq!(
                kfx_wgpu_draw_submit_target_triangles(
                    std::ptr::null_mut(),
                    1,
                    std::ptr::null(),
                    0,
                    1,
                    error.as_mut_ptr(),
                    error.len()
                ),
                -1
            );
        }
    }
}
