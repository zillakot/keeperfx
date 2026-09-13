cmake_minimum_required(VERSION 3.20)
project(terrain_bridge_tests C CXX)
set(CMAKE_CXX_STANDARD 20)
enable_testing()
get_filename_component(KFX_ROOT "../.." ABSOLUTE)
add_executable(terrain_bridge_test bridge_test.cpp
    "${KFX_ROOT}/src/kfx/renderer/WgpuTerrainBridge.cpp"
    "${KFX_ROOT}/src/kfx/renderer/GpolyCapture.c")
target_include_directories(terrain_bridge_test PRIVATE "${KFX_ROOT}/src")
target_compile_definitions(terrain_bridge_test PRIVATE KFX_RUST_PRESENTER=1)
target_compile_options(terrain_bridge_test PRIVATE -Wall -Wextra -Werror)
set(KFX_GPU_LIBRARY "" CACHE FILEPATH "Optional real wgpu static library")
if(KFX_GPU_LIBRARY)
    target_compile_definitions(terrain_bridge_test PRIVATE KFX_BRIDGE_REAL_GPU=1)
    target_link_libraries(terrain_bridge_test PRIVATE "${KFX_GPU_LIBRARY}" "-framework Metal" "-framework QuartzCore" "-framework Foundation" "-framework AppKit" "-framework IOKit" "-framework CoreGraphics")
else()
    target_compile_options(terrain_bridge_test PRIVATE -fsanitize=address -fno-omit-frame-pointer)
    target_link_options(terrain_bridge_test PRIVATE -fsanitize=address)
endif()
add_test(NAME terrain_bridge COMMAND terrain_bridge_test)

if(KFX_GPU_LIBRARY)
    add_executable(helper_lease_test helper_test.cpp
        "${KFX_ROOT}/src/kfx/renderer/WgpuTerrainBridge.cpp"
        "${KFX_ROOT}/src/kfx/renderer/GpolyCapture.c"
        "${KFX_ROOT}/src/kfx/renderer/software/WgpuRawImage.c"
        "${KFX_ROOT}/src/kfx/renderer/software/MovieFrame.cpp"
        "${KFX_ROOT}/src/kfx/lense/WgpuLens.cpp")
    target_include_directories(helper_lease_test PRIVATE "${KFX_ROOT}/src")
    target_compile_definitions(helper_lease_test PRIVATE KFX_RUST_PRESENTER=1)
    target_compile_options(helper_lease_test PRIVATE -Wall -Wextra -Werror -fwrapv)
    target_link_libraries(helper_lease_test PRIVATE "${KFX_GPU_LIBRARY}" "-framework Metal" "-framework QuartzCore" "-framework Foundation" "-framework AppKit" "-framework IOKit" "-framework CoreGraphics")
    add_test(NAME helper_leases COMMAND helper_lease_test)
endif()
