cmake_minimum_required(VERSION 3.20)

# PhoneCam Windows Media Foundation virtual camera native components.
# Windows-only: Media Foundation software camera sources require Windows 11+
# (MFCreateVirtualCamera, build 22621+).
project(PhoneCamVirtualCameraNative LANGUAGES CXX)

if(NOT WIN32)
    message(FATAL_ERROR "PhoneCam virtual camera native components currently target Windows only.")
endif()

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
    set(CMAKE_BUILD_TYPE Release CACHE STRING "" FORCE)
endif()

# Targeting 10.0.26100 keeps the Media Foundation virtual camera headers
# (mfvirtualcamera.h) and frame-server integration available.
add_compile_definitions(
    UNICODE
    _UNICODE
    WIN32_LEAN_AND_MEAN
    NOMINMAX
    _WIN32_WINNT=0x0A00
    NTDDI_VERSION=0x0A00000B
)

if(MSVC)
    add_compile_options(/W4 /permissive- /EHsc /utf-8 /Zc:__cplusplus)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>DLL")
endif()

set(CMAKE_RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)
set(CMAKE_LIBRARY_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)

add_subdirectory(virtual-camera)
