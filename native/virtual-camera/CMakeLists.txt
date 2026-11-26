# PhoneCam Windows Media Foundation Virtual Camera DLL and Registrar

set(VCAM_SOURCES
    dllmain.cpp
    phonecam_virtual_camera.cpp
    ${CMAKE_CURRENT_SOURCE_DIR}/../shared/src/frame_shm.cpp
    ${CMAKE_CURRENT_SOURCE_DIR}/../shared/src/nv12.cpp
)

add_library(PhoneCamVirtualCamera SHARED ${VCAM_SOURCES} virtual_camera.def)

target_include_directories(PhoneCamVirtualCamera PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${CMAKE_CURRENT_SOURCE_DIR}/../shared/include
)

target_link_libraries(PhoneCamVirtualCamera PRIVATE
    mfplat.lib
    mfuuid.lib
    mf.lib
    mfsensorgroup.lib
    ole32.lib
    advapi32.lib
    shlwapi.lib
)

set_target_properties(PhoneCamVirtualCamera PROPERTIES
    OUTPUT_NAME "PhoneCamVirtualCamera"
    PREFIX ""
)

add_executable(phonecam-vcam-register
    phonecam_vcam_register.cpp
)

target_include_directories(phonecam-vcam-register PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(phonecam-vcam-register PRIVATE
    mfplat.lib
    mfsensorgroup.lib
    # MFEnumDeviceSources lives in mf.lib, not mfplat.lib. It is needed to answer
    # "is the camera actually enumerable", which is the only check that catches
    # a surviving CLSID key whose device entry was removed.
    mf.lib
    ole32.lib
    advapi32.lib
)

set_target_properties(phonecam-vcam-register PROPERTIES
    OUTPUT_NAME "phonecam-vcam-register"
)

# Verification tool: enumerates capture devices and pulls frames from the
# registered virtual camera. This is the only check that exercises the whole
# path — COM registration, Frame Server activation and sample delivery — the
# way a real application such as OBS or Chrome would.
add_executable(phonecam-vcam-probe
    probe_vcam_stream.cpp
)

target_include_directories(phonecam-vcam-probe PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(phonecam-vcam-probe PRIVATE
    mfplat.lib
    mf.lib
    mfreadwrite.lib
    mfuuid.lib
    ole32.lib
)

set_target_properties(phonecam-vcam-probe PROPERTIES
    OUTPUT_NAME "phonecam-vcam-probe"
)

# Verification tool: saves one frame as a PNG so the placeholder can be checked
# as an image rather than inferred from pixel values.
add_executable(phonecam-vcam-frame
    probe_frame_dump.cpp
)

target_include_directories(phonecam-vcam-frame PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
)

target_link_libraries(phonecam-vcam-frame PRIVATE
    mfplat.lib
    mf.lib
    mfreadwrite.lib
    mfuuid.lib
    ole32.lib
)

set_target_properties(phonecam-vcam-frame PROPERTIES
    OUTPUT_NAME "phonecam-vcam-frame"
)
