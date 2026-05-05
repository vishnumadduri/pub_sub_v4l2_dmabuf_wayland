#pragma once

#include <cstdint>
#include <string>

struct wl_display;
struct wl_registry;
struct wl_compositor;
struct wl_surface;
struct wl_egl_window;
struct xdg_wm_base;
struct xdg_surface;
struct xdg_toplevel;

namespace dmabuf {

// Owns the Wayland connection plus a single xdg_toplevel-backed wl_surface
// that we render into via a wl_egl_window. Keep this small and explicit:
// every member is a globally registered handle we have to release.
class WaylandWindow {
public:
    WaylandWindow() = default;
    ~WaylandWindow();

    WaylandWindow(const WaylandWindow&) = delete;
    WaylandWindow& operator=(const WaylandWindow&) = delete;

    bool create(uint32_t width, uint32_t height, const std::string& title);
    void destroy();

    // Pump the display once (non-blocking). Returns false if the compositor
    // closed the window or an error occurred.
    bool dispatch_pending();

    wl_display* display() const { return display_; }
    wl_surface* surface() const { return surface_; }
    wl_egl_window* egl_window() const { return egl_window_; }

    uint32_t width() const { return width_; }
    uint32_t height() const { return height_; }
    bool should_close() const { return should_close_; }

    // Called from the registry/xdg listeners. Public so the C-style listener
    // callbacks can reach them; do not call from user code.
    void on_global(wl_registry* registry,
                   uint32_t name,
                   const char* interface,
                   uint32_t version);
    void on_xdg_surface_configure(xdg_surface* xs, uint32_t serial);
    void on_xdg_toplevel_close();
    void on_xdg_wm_base_ping(xdg_wm_base* wm, uint32_t serial);

private:
    wl_display* display_ = nullptr;
    wl_registry* registry_ = nullptr;
    wl_compositor* compositor_ = nullptr;
    xdg_wm_base* wm_base_ = nullptr;
    wl_surface* surface_ = nullptr;
    xdg_surface* xdg_surface_ = nullptr;
    xdg_toplevel* xdg_toplevel_ = nullptr;
    wl_egl_window* egl_window_ = nullptr;

    uint32_t width_ = 0;
    uint32_t height_ = 0;
    bool configured_ = false;
    bool should_close_ = false;
};

} // namespace dmabuf
