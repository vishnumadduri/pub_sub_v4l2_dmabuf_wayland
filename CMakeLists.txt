cmake_minimum_required(VERSION 3.10)
project(dma_buf_pipeline LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
endif()

add_compile_options(-Wall -Wextra -Wno-unused-parameter)

find_package(PkgConfig REQUIRED)
pkg_check_modules(WAYLAND_CLIENT  REQUIRED wayland-client)
pkg_check_modules(WAYLAND_EGL     REQUIRED wayland-egl)
pkg_check_modules(WAYLAND_PROTOS  REQUIRED wayland-protocols>=1.12)
pkg_check_modules(WAYLAND_SCANNER REQUIRED wayland-scanner)
pkg_check_modules(EGL             REQUIRED egl)
pkg_check_modules(GLES2           REQUIRED glesv2)
pkg_check_modules(LIBV4L2         REQUIRED libv4l2)
pkg_check_modules(LIBDRM          REQUIRED libdrm)

# ---- Generate xdg-shell client glue from the protocol XML -----------------
# wayland-scanner produces a header (client-protocol.h) and a private code
# file (protocol.c) that we compile into a small static library both
# executables can link against.

pkg_get_variable(WAYLAND_PROTOCOLS_DIR wayland-protocols pkgdatadir)
pkg_get_variable(WAYLAND_SCANNER_BIN   wayland-scanner   wayland_scanner)

set(XDG_SHELL_XML "${WAYLAND_PROTOCOLS_DIR}/stable/xdg-shell/xdg-shell.xml")
set(XDG_SHELL_HDR "${CMAKE_CURRENT_BINARY_DIR}/xdg-shell-client-protocol.h")
set(XDG_SHELL_SRC "${CMAKE_CURRENT_BINARY_DIR}/xdg-shell-protocol.c")

add_custom_command(
    OUTPUT ${XDG_SHELL_HDR}
    COMMAND ${WAYLAND_SCANNER_BIN} client-header ${XDG_SHELL_XML} ${XDG_SHELL_HDR}
    DEPENDS ${XDG_SHELL_XML}
    VERBATIM)

add_custom_command(
    OUTPUT ${XDG_SHELL_SRC}
    COMMAND ${WAYLAND_SCANNER_BIN} private-code ${XDG_SHELL_XML} ${XDG_SHELL_SRC}
    DEPENDS ${XDG_SHELL_XML}
    VERBATIM)

add_library(xdg_shell_protocol STATIC ${XDG_SHELL_SRC} ${XDG_SHELL_HDR})
target_include_directories(xdg_shell_protocol PUBLIC ${CMAKE_CURRENT_BINARY_DIR})
target_include_directories(xdg_shell_protocol PRIVATE ${WAYLAND_CLIENT_INCLUDE_DIRS})
set_target_properties(xdg_shell_protocol PROPERTIES POSITION_INDEPENDENT_CODE ON)

# ---- Shared utility library ----------------------------------------------

add_library(dmabuf_utils STATIC
    src/socket_utils.cpp
    src/v4l2_utils.cpp
    src/wayland_utils.cpp
    src/egl_utils.cpp)

target_include_directories(dmabuf_utils PUBLIC
    ${CMAKE_CURRENT_SOURCE_DIR}/include
    ${CMAKE_CURRENT_BINARY_DIR}                # for xdg-shell-client-protocol.h
    ${WAYLAND_CLIENT_INCLUDE_DIRS}
    ${WAYLAND_EGL_INCLUDE_DIRS}
    ${EGL_INCLUDE_DIRS}
    ${GLES2_INCLUDE_DIRS}
    ${LIBV4L2_INCLUDE_DIRS}
    ${LIBDRM_INCLUDE_DIRS})

target_link_libraries(dmabuf_utils PUBLIC
    xdg_shell_protocol
    ${WAYLAND_CLIENT_LIBRARIES}
    ${WAYLAND_EGL_LIBRARIES}
    ${EGL_LIBRARIES}
    ${GLES2_LIBRARIES}
    ${LIBV4L2_LIBRARIES})

# ---- Executables ----------------------------------------------------------

add_executable(publisher  src/publisher.cpp)
target_link_libraries(publisher  PRIVATE dmabuf_utils)

add_executable(subscriber src/subscriber.cpp)
target_link_libraries(subscriber PRIVATE dmabuf_utils)
