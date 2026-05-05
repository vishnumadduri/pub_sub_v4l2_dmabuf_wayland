#include "wayland_utils.hpp"

#include <cstdio>
#include <cstring>

#include <wayland-client.h>
#include <wayland-egl.h>

#include "xdg-shell-client-protocol.h"

namespace dmabuf {

// ---- C-style listener trampolines -----------------------------------------
// These exist because the Wayland C API takes plain function pointers; each
// one just forwards to the matching C++ method on WaylandWindow.

static void registry_global(void* data,
                            wl_registry* registry,
                            uint32_t name,
                            const char* interface,
                            uint32_t version) {
    static_cast<WaylandWindow*>(data)->on_global(registry, name, interface, version);
}

static void registry_global_remove(void*, wl_registry*, uint32_t) {}

static const wl_registry_listener kRegistryListener = {
    registry_global,
    registry_global_remove,
};

static void wm_base_ping(void* data, xdg_wm_base* wm, uint32_t serial) {
    static_cast<WaylandWindow*>(data)->on_xdg_wm_base_ping(wm, serial);
}

static const xdg_wm_base_listener kWmBaseListener = {
    wm_base_ping,
};

static void xdg_surface_configure(void* data, xdg_surface* xs, uint32_t serial) {
    static_cast<WaylandWindow*>(data)->on_xdg_surface_configure(xs, serial);
}

static const xdg_surface_listener kXdgSurfaceListener = {
    xdg_surface_configure,
};

static void xdg_toplevel_configure(void*, xdg_toplevel*, int32_t, int32_t, wl_array*) {}

static void xdg_toplevel_close(void* data, xdg_toplevel*) {
    static_cast<WaylandWindow*>(data)->on_xdg_toplevel_close();
}

static const xdg_toplevel_listener kXdgToplevelListener = {
    xdg_toplevel_configure,
    xdg_toplevel_close,
    nullptr, // configure_bounds (since v4)
    nullptr, // wm_capabilities  (since v5)
};

// ---------------------------------------------------------------------------

WaylandWindow::~WaylandWindow() {
    destroy();
}

void WaylandWindow::on_global(wl_registry* registry,
                              uint32_t name,
                              const char* interface,
                              uint32_t version) {
    if (std::strcmp(interface, wl_compositor_interface.name) == 0) {
        compositor_ = static_cast<wl_compositor*>(
            wl_registry_bind(registry, name, &wl_compositor_interface,
                             version < 4 ? version : 4));
    } else if (std::strcmp(interface, xdg_wm_base_interface.name) == 0) {
        wm_base_ = static_cast<xdg_wm_base*>(
            wl_registry_bind(registry, name, &xdg_wm_base_interface, 1));
        xdg_wm_base_add_listener(wm_base_, &kWmBaseListener, this);
    }
}

void WaylandWindow::on_xdg_wm_base_ping(xdg_wm_base* wm, uint32_t serial) {
    xdg_wm_base_pong(wm, serial);
}

void WaylandWindow::on_xdg_surface_configure(xdg_surface* xs, uint32_t serial) {
    xdg_surface_ack_configure(xs, serial);
    configured_ = true;
}

void WaylandWindow::on_xdg_toplevel_close() {
    should_close_ = true;
}

bool WaylandWindow::create(uint32_t width, uint32_t height, const std::string& title) {
    width_ = width;
    height_ = height;

    display_ = wl_display_connect(nullptr);
    if (!display_) {
        std::fprintf(stderr, "wl_display_connect failed (is a Wayland compositor running?)\n");
        return false;
    }

    registry_ = wl_display_get_registry(display_);
    wl_registry_add_listener(registry_, &kRegistryListener, this);

    // First roundtrip: server enumerates globals into our registry. After
    // this we know whether wl_compositor and xdg_wm_base are available.
    wl_display_roundtrip(display_);

    if (!compositor_ || !wm_base_) {
        std::fprintf(stderr, "missing wl_compositor or xdg_wm_base\n");
        return false;
    }

    surface_ = wl_compositor_create_surface(compositor_);
    xdg_surface_ = xdg_wm_base_get_xdg_surface(wm_base_, surface_);
    xdg_surface_add_listener(xdg_surface_, &kXdgSurfaceListener, this);

    xdg_toplevel_ = xdg_surface_get_toplevel(xdg_surface_);
    xdg_toplevel_add_listener(xdg_toplevel_, &kXdgToplevelListener, this);
    xdg_toplevel_set_title(xdg_toplevel_, title.c_str());
    xdg_toplevel_set_app_id(xdg_toplevel_, "dmabuf.subscriber");

    // Initial commit kicks off the configure handshake. We must wait for the
    // first xdg_surface.configure before committing real content.
    wl_surface_commit(surface_);
    while (!configured_) {
        if (wl_display_dispatch(display_) < 0) {
            std::fprintf(stderr, "wl_display_dispatch failed during configure\n");
            return false;
        }
    }

    egl_window_ = wl_egl_window_create(surface_,
                                       static_cast<int>(width_),
                                       static_cast<int>(height_));
    if (!egl_window_) {
        std::fprintf(stderr, "wl_egl_window_create failed\n");
        return false;
    }
    return true;
}

void WaylandWindow::destroy() {
    if (egl_window_)   { wl_egl_window_destroy(egl_window_); egl_window_ = nullptr; }
    if (xdg_toplevel_) { xdg_toplevel_destroy(xdg_toplevel_); xdg_toplevel_ = nullptr; }
    if (xdg_surface_)  { xdg_surface_destroy(xdg_surface_); xdg_surface_ = nullptr; }
    if (surface_)      { wl_surface_destroy(surface_); surface_ = nullptr; }
    if (wm_base_)      { xdg_wm_base_destroy(wm_base_); wm_base_ = nullptr; }
    if (compositor_)   { wl_compositor_destroy(compositor_); compositor_ = nullptr; }
    if (registry_)     { wl_registry_destroy(registry_); registry_ = nullptr; }
    if (display_)      { wl_display_disconnect(display_); display_ = nullptr; }
}

bool WaylandWindow::dispatch_pending() {
    if (!display_) return false;
    // Flush any client-side queued requests, then process events the server
    // may have queued for us. Non-blocking: returns immediately if no events.
    wl_display_flush(display_);
    if (wl_display_dispatch_pending(display_) < 0) return false;
    return !should_close_;
}

} // namespace dmabuf
