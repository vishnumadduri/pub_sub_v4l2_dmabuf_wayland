#pragma once

#include <cstdint>

#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <GLES2/gl2ext.h>

#include "socket_utils.hpp"

struct wl_display;
struct wl_egl_window;

namespace dmabuf {

// Holds the EGL display/context/surface tied to a Wayland EGL window. Also
// keeps cached pointers to the EGL/GLES extension entry points we use for
// DMA-BUF import.
class EglContext {
public:
    EglContext() = default;
    ~EglContext();

    EglContext(const EglContext&) = delete;
    EglContext& operator=(const EglContext&) = delete;

    bool init(wl_display* wl_display, wl_egl_window* egl_window);
    void destroy();

    // Build an EGLImage from the given DMA-BUF file descriptor + metadata.
    // The caller may close the dmabuf_fd as soon as this returns successfully:
    // EGL takes its own reference on the buffer.
    EGLImageKHR import_dmabuf(int dmabuf_fd, const FrameMeta& meta);

    void destroy_image(EGLImageKHR image);

    void swap_buffers();

    EGLDisplay display() const { return display_; }
    EGLContext context() const { return context_; }
    EGLSurface surface() const { return surface_; }

private:
    EGLDisplay display_ = EGL_NO_DISPLAY;
    EGLContext context_ = EGL_NO_CONTEXT;
    EGLSurface surface_ = EGL_NO_SURFACE;
    EGLConfig config_ = nullptr;

    // Cached extension entry points — these must be looked up via
    // eglGetProcAddress because they're not part of the EGL/GLES core.
    PFNEGLCREATEIMAGEKHRPROC eglCreateImageKHR_ = nullptr;
    PFNEGLDESTROYIMAGEKHRPROC eglDestroyImageKHR_ = nullptr;
    PFNGLEGLIMAGETARGETTEXTURE2DOESPROC glEGLImageTargetTexture2DOES_ = nullptr;

    bool has_dma_buf_import_ = false;
    bool has_modifiers_ = false;
};

// Tiny GL renderer that draws a fullscreen textured quad, sampling either an
// RGB texture (when the DMA-BUF was already RGB) or a YUYV texture that we
// convert to RGB in the fragment shader.
class QuadRenderer {
public:
    QuadRenderer() = default;
    ~QuadRenderer();

    QuadRenderer(const QuadRenderer&) = delete;
    QuadRenderer& operator=(const QuadRenderer&) = delete;

    // drm_fourcc selects YUV vs RGB shader path and texture target.
    // YUV formats (YUYV, NV12, NV21) use GL_TEXTURE_EXTERNAL_OES so the driver
    // does YUV→RGB in hardware; RGB uses GL_TEXTURE_2D with a pass-through shader.
    bool init(uint32_t drm_fourcc);
    void destroy();

    void draw(EGLImageKHR image,
              uint32_t width,
              uint32_t height,
              PFNGLEGLIMAGETARGETTEXTURE2DOESPROC image_target_fn);

    void set_viewport(int width, int height);

private:
    GLuint program_ = 0;
    GLuint texture_ = 0;
    GLuint vbo_ = 0;
    GLint attr_pos_ = -1;
    GLint attr_uv_ = -1;
    GLint uniform_tex_ = -1;
    GLint uniform_tex_size_ = -1; // unused for YUYV/external path
    bool yuv_ = false;
    GLenum tex_target_ = GL_TEXTURE_2D; // GL_TEXTURE_EXTERNAL_OES for YUV formats
    int viewport_w_ = 0;
    int viewport_h_ = 0;
};

} // namespace dmabuf
