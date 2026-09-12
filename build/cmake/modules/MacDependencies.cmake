include(FetchContent)
if(POLICY CMP0135)
    cmake_policy(SET CMP0135 NEW)
endif()

FetchContent_Declare(astronomy
    URL https://github.com/cosinekitty/astronomy/archive/865d3da7d8112bbc7911238052c6af4aaf877181.tar.gz
    SOURCE_SUBDIR source/c)
FetchContent_Declare(centijson
    URL https://github.com/mity/centijson/archive/8c7a5fb42d9f55044d60592809288164adb4ca95.tar.gz
    SOURCE_SUBDIR src)
FetchContent_Declare(enet6
    URL https://github.com/SirLynix/enet6/archive/bf0003fb0004b12ff1d2b0c51c7c7e9a0d2d7732.tar.gz
    SOURCE_SUBDIR src)
FetchContent_MakeAvailable(astronomy centijson enet6)

add_library(astronomy_static STATIC ${astronomy_SOURCE_DIR}/source/c/astronomy.c)
target_include_directories(astronomy_static PUBLIC ${astronomy_SOURCE_DIR}/source/c)

add_library(centijson_static STATIC
    ${centijson_SOURCE_DIR}/src/json.c
    ${centijson_SOURCE_DIR}/src/json-dom.c
    ${centijson_SOURCE_DIR}/src/json-ptr.c
    ${centijson_SOURCE_DIR}/src/value.c)
target_include_directories(centijson_static PUBLIC ${centijson_SOURCE_DIR}/src)

add_library(enet6_static STATIC
    ${enet6_SOURCE_DIR}/src/address.c
    ${enet6_SOURCE_DIR}/src/callbacks.c
    ${enet6_SOURCE_DIR}/src/compress.c
    ${enet6_SOURCE_DIR}/src/host.c
    ${enet6_SOURCE_DIR}/src/list.c
    ${enet6_SOURCE_DIR}/src/packet.c
    ${enet6_SOURCE_DIR}/src/peer.c
    ${enet6_SOURCE_DIR}/src/protocol.c
    ${enet6_SOURCE_DIR}/src/unix.c)
target_include_directories(enet6_static PUBLIC ${enet6_SOURCE_DIR}/include)
target_compile_definitions(enet6_static PRIVATE HAS_SOCKLEN_T=1)

find_package(CURL REQUIRED)
find_package(Iconv REQUIRED)
add_library(curl_static INTERFACE)
target_link_libraries(curl_static INTERFACE CURL::libcurl)

find_library(KFX_MINIUPNPC_LIBRARY miniupnpc REQUIRED)
find_library(KFX_NATPMP_LIBRARY natpmp REQUIRED)
add_library(miniupnpc UNKNOWN IMPORTED GLOBAL)
set_target_properties(miniupnpc PROPERTIES IMPORTED_LOCATION "${KFX_MINIUPNPC_LIBRARY}")
add_library(natpmp UNKNOWN IMPORTED GLOBAL)
set_target_properties(natpmp PROPERTIES IMPORTED_LOCATION "${KFX_NATPMP_LIBRARY}")
