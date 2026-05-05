#include "egl_utils.hpp"

#include <cstdio>
#include <cstring>
#include <vector>

#include <drm_fourcc.h>
#include <wayland-client.h>
#include <wayland-egl.h>

namespace dmabuf {

// ---------- EglContext -----------------------------------------------------

EglContext::~EglContext() {
    destroy();
}

bool EglContext::init(wl_display* wl_display, wl_egl_window* egl_window) {
    // Use the platform display API so Mesa picks the right EGL backend when
    // multiple GPUs or display outputs are present.
    const char* client_exts = eglQueryString(EGL_NO_DISPLAY, EGL_EXTENSIONS);
    if (client_exts &&
        (std::strstr(client_exts, "EGL_EXT_platform_wayland") ||
         std::strstr(client_exts, "EGL_KHR_platform_wayland"))) {
        auto fn = reinterpret_cast<PFNEGLGETPLATFORMDISPLAYEXTPROC>(
            eglGetProcAddress("eglGetPlatformDisplayEXT"));
        if (fn)
            display_ = fn(EGL_PLATFORM_WAYLAND_EXT, wl_display, nullptr);
    }
    if (display_ == EGL_NO_DISPLAY)
        display_ = eglGetDisplay(reinterpret_cast<EGLNativeDisplayType>(wl_display));
    if (display_ == EGL_NO_DISPLAY) {
        std::fprintf(stderr, "eglGetDisplay failed\n");
        return false;
    }

    EGLint major, minor;
    if (!eglInitialize(display_, &major, &minor)) {
        std::fprintf(stderr, "eglInitialize failed: 0x%x\n", eglGetError());
        return false;
    }
    std::printf("EGL %d.%d\n", major, minor);

    if (!eglBindAPI(EGL_OPENGL_ES_API)) {
        std::fprintf(stderr, "eglBindAPI(GLES) failed\n");
        return false;
    }

    const EGLint config_attribs[] = {
        EGL_SURFACE_TYPE,     EGL_WINDOW_BIT,
        EGL_RENDERABLE_TYPE,  EGL_OPENGL_ES2_BIT,
        EGL_RED_SIZE,         8,
        EGL_GREEN_SIZE,       8,
        EGL_BLUE_SIZE,        8,
        EGL_ALPHA_SIZE,       8,
        EGL_NONE,
    };
    EGLint num_configs = 0;
    if (!eglChooseConfig(display_, config_attribs, &config_, 1, &num_configs)
        || num_configs < 1) {
        std::fprintf(stderr, "eglChooseConfig failed\n");
        return false;
    }

    const EGLint ctx_attribs[] = {
        EGL_CONTEXT_CLIENT_VERSION, 2,
        EGL_NONE,
    };
    context_ = eglCreateContext(display_, config_, EGL_NO_CONTEXT, ctx_attribs);
    if (context_ == EGL_NO_CONTEXT) {
        std::fprintf(stderr, "eglCreateContext failed: 0x%x\n", eglGetError());
        return false;
    }

    surface_ = eglCreateWindowSurface(
        display_, config_,
        reinterpret_cast<EGLNativeWindowType>(egl_window),
        nullptr);
    if (surface_ == EGL_NO_SURFACE) {
        std::fprintf(stderr, "eglCreateWindowSurface failed: 0x%x\n", eglGetError());
        return false;
    }

    if (!eglMakeCurrent(display_, surface_, surface_, context_)) {
        std::fprintf(stderr, "eglMakeCurrent failed: 0x%x\n", eglGetError());
        return false;
    }

    // The DMA-BUF import path is provided by EGL_EXT_image_dma_buf_import,
    // which exposes new pname tokens for eglCreateImageKHR. The function
    // pointers themselves come from EGL_KHR_image_base + GL_OES_EGL_image.
    const char* exts = eglQueryString(display_, EGL_EXTENSIONS);
    if (!exts) exts = "";
    has_dma_buf_import_ = std::strstr(exts, "EGL_EXT_image_dma_buf_import") != nullptr;
    if (!has_dma_buf_import_) {
        std::fprintf(stderr, "warning: EGL_EXT_image_dma_buf_import not advertised\n");
    }
    has_modifiers_ = std::strstr(exts, "EGL_EXT_image_dma_buf_import_modifiers") != nullptr;

    eglCreateImageKHR_ = reinterpret_cast<PFNEGLCREATEIMAGEKHRPROC>(
        eglGetProcAddress("eglCreateImageKHR"));
    eglDestroyImageKHR_ = reinterpret_cast<PFNEGLDESTROYIMAGEKHRPROC>(
        eglGetProcAddress("eglDestroyImageKHR"));
    glEGLImageTargetTexture2DOES_ = reinterpret_cast<PFNGLEGLIMAGETARGETTEXTURE2DOESPROC>(
        eglGetProcAddress("glEGLImageTargetTexture2DOES"));

    if (!eglCreateImageKHR_ || !eglDestroyImageKHR_ || !glEGLImageTargetTexture2DOES_) {
        std::fprintf(stderr, "missing EGL/GLES extension entry points\n");
        return false;
    }

    std::printf("GL_RENDERER: %s\n", glGetString(GL_RENDERER));
    return true;
}

void EglContext::destroy() {
    if (display_ != EGL_NO_DISPLAY) {
        eglMakeCurrent(display_, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
        if (surface_ != EGL_NO_SURFACE) eglDestroySurface(display_, surface_);
        if (context_ != EGL_NO_CONTEXT) eglDestroyContext(display_, context_);
        eglTerminate(display_);
    }
    surface_ = EGL_NO_SURFACE;
    context_ = EGL_NO_CONTEXT;
    display_ = EGL_NO_DISPLAY;
}

EGLImageKHR EglContext::import_dmabuf(int dmabuf_fd, const FrameMeta& meta) {
    // Mesa (all platforms, kernels ≥ 5.10) requires explicit DRM format
    // modifiers when the extension is advertised. DRM_FORMAT_MOD_LINEAR (= 0)
    // tells the driver this is a plain linear buffer — what V4L2 MMAP gives us.
    std::vector<EGLint> attrs = {
        EGL_WIDTH,                     static_cast<EGLint>(meta.width),
        EGL_HEIGHT,                    static_cast<EGLint>(meta.height),
        EGL_LINUX_DRM_FOURCC_EXT,      static_cast<EGLint>(meta.format),
        EGL_DMA_BUF_PLANE0_FD_EXT,     dmabuf_fd,
        EGL_DMA_BUF_PLANE0_OFFSET_EXT, 0,
        EGL_DMA_BUF_PLANE0_PITCH_EXT,  static_cast<EGLint>(meta.stride),
    };
    if (has_modifiers_) {
        attrs.insert(attrs.end(), {
            EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT, 0,
            EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT, 0,
        });
    }

    // NV12/NV21 is semi-planar: Y at offset 0, interleaved UV at stride*height.
    // Both planes live in the same DMA-BUF fd and share the luma stride.
    if (meta.format == DRM_FORMAT_NV12 || meta.format == DRM_FORMAT_NV21) {
        EGLint uv_offset = static_cast<EGLint>(meta.stride * meta.height);
        attrs.insert(attrs.end(), {
            EGL_DMA_BUF_PLANE1_FD_EXT,     dmabuf_fd,
            EGL_DMA_BUF_PLANE1_OFFSET_EXT, uv_offset,
            EGL_DMA_BUF_PLANE1_PITCH_EXT,  static_cast<EGLint>(meta.stride),
        });
        if (has_modifiers_) {
            attrs.insert(attrs.end(), {
                EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT, 0,
                EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT, 0,
            });
        }
    }

    attrs.push_back(EGL_NONE);

    EGLImageKHR img = eglCreateImageKHR_(
        display_, EGL_NO_CONTEXT,
        EGL_LINUX_DMA_BUF_EXT,
        nullptr, // dmabuf imports use attribs only
        attrs.data());
    if (img == EGL_NO_IMAGE_KHR) {
        std::fprintf(stderr, "eglCreateImageKHR failed: 0x%x\n", eglGetError());
    }
    return img;
}

void EglContext::destroy_image(EGLImageKHR image) {
    if (image != EGL_NO_IMAGE_KHR) eglDestroyImageKHR_(display_, image);
}

void EglContext::swap_buffers() {
    eglSwapBuffers(display_, surface_);
}

// ---------- QuadRenderer ---------------------------------------------------

namespace {

const char* kVertexShader = R"(
attribute vec2 a_pos;
attribute vec2 a_uv;
varying vec2 v_uv;
void main() {
    v_uv = a_uv;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
)";

// Plain RGB sampler. Used when the source DMA-BUF was already RGB-ish.
const char* kFragmentShaderRGB = R"(
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_tex;
void main() {
    gl_FragColor = texture2D(u_tex, v_uv);
}
)";

// For YUYV we use GL_TEXTURE_EXTERNAL_OES so the Intel/Mesa driver does
// hardware YUV→RGB conversion at sample time. The fragment shader just
// passes the result straight through — no manual BT.601 needed.
const char* kFragmentShaderYUYV = R"(
#extension GL_OES_EGL_image_external : require
precision mediump float;
varying vec2 v_uv;
uniform samplerExternalOES u_tex;
void main() {
    gl_FragColor = texture2D(u_tex, v_uv);
}
)";

GLuint compile(GLenum type, const char* src) {
    GLuint sh = glCreateShader(type);
    glShaderSource(sh, 1, &src, nullptr);
    glCompileShader(sh);
    GLint ok = 0;
    glGetShaderiv(sh, GL_COMPILE_STATUS, &ok);
    if (!ok) {
        char log[1024];
        glGetShaderInfoLog(sh, sizeof(log), nullptr, log);
        std::fprintf(stderr, "shader compile failed: %s\n", log);
        glDeleteShader(sh);
        return 0;
    }
    return sh;
}

GLuint link_program(const char* vs_src, const char* fs_src) {
    GLuint vs = compile(GL_VERTEX_SHADER, vs_src);
    GLuint fs = compile(GL_FRAGMENT_SHADER, fs_src);
    if (!vs || !fs) {
        if (vs) glDeleteShader(vs);
        if (fs) glDeleteShader(fs);
        return 0;
    }
    GLuint p = glCreateProgram();
    glAttachShader(p, vs);
    glAttachShader(p, fs);
    glLinkProgram(p);
    glDeleteShader(vs);
    glDeleteShader(fs);
    GLint ok = 0;
    glGetProgramiv(p, GL_LINK_STATUS, &ok);
    if (!ok) {
        char log[1024];
        glGetProgramInfoLog(p, sizeof(log), nullptr, log);
        std::fprintf(stderr, "program link failed: %s\n", log);
        glDeleteProgram(p);
        return 0;
    }
    return p;
}

} // namespace

QuadRenderer::~QuadRenderer() {
    destroy();
}

bool QuadRenderer::init(uint32_t drm_fourcc) {
    // YUV formats (YUYV, NV12, NV21) use GL_TEXTURE_EXTERNAL_OES so the GPU
    // driver handles YUV→RGB at sample time. RGB formats use GL_TEXTURE_2D.
    yuv_ = (drm_fourcc == DRM_FORMAT_YUYV ||
             drm_fourcc == DRM_FORMAT_NV12  ||
             drm_fourcc == DRM_FORMAT_NV21);
    tex_target_ = yuv_ ? GL_TEXTURE_EXTERNAL_OES : GL_TEXTURE_2D;

    program_ = link_program(kVertexShader, yuv_ ? kFragmentShaderYUYV : kFragmentShaderRGB);
    if (!program_) return false;

    attr_pos_    = glGetAttribLocation(program_, "a_pos");
    attr_uv_     = glGetAttribLocation(program_, "a_uv");
    uniform_tex_ = glGetUniformLocation(program_, "u_tex");

    // Both GL_TEXTURE_2D and GL_TEXTURE_EXTERNAL_OES on Linux/Mesa place the
    // first byte of the DMA-BUF at v=0, which is the camera's top scanline.
    // Screen top (NDC y=+1) must sample v=0, so we swap v here.
    const float verts[] = {
    //   x,    y,   u,    v
        -1.f, -1.f, 0.f, 1.f,  // screen bottom-left  → texture bottom
         1.f, -1.f, 1.f, 1.f,  // screen bottom-right → texture bottom
        -1.f,  1.f, 0.f, 0.f,  // screen top-left     → texture top (camera row 0)
         1.f,  1.f, 1.f, 0.f,  // screen top-right    → texture top
    };
    glGenBuffers(1, &vbo_);
    glBindBuffer(GL_ARRAY_BUFFER, vbo_);
    glBufferData(GL_ARRAY_BUFFER, sizeof(verts), verts, GL_STATIC_DRAW);

    glGenTextures(1, &texture_);
    glBindTexture(tex_target_, texture_);
    glTexParameteri(tex_target_, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(tex_target_, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexParameteri(tex_target_, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(tex_target_, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    return true;
}

void QuadRenderer::destroy() {
    if (texture_) { glDeleteTextures(1, &texture_); texture_ = 0; }
    if (vbo_)     { glDeleteBuffers(1, &vbo_); vbo_ = 0; }
    if (program_) { glDeleteProgram(program_); program_ = 0; }
}

void QuadRenderer::set_viewport(int width, int height) {
    viewport_w_ = width;
    viewport_h_ = height;
}

void QuadRenderer::draw(EGLImageKHR image,
                        uint32_t width,
                        uint32_t height,
                        PFNGLEGLIMAGETARGETTEXTURE2DOESPROC image_target_fn) {
    glViewport(0, 0, viewport_w_, viewport_h_);
    glClearColor(0.f, 0.f, 0.f, 1.f);
    glClear(GL_COLOR_BUFFER_BIT);

    glBindTexture(tex_target_, texture_);
    // Re-attach the EGLImage as the backing store every frame. Cheap: the
    // storage is the DMA-BUF pages; no copy occurs.
    image_target_fn(tex_target_, image);

    glUseProgram(program_);
    glActiveTexture(GL_TEXTURE0);
    glBindTexture(tex_target_, texture_);
    glUniform1i(uniform_tex_, 0);

    glBindBuffer(GL_ARRAY_BUFFER, vbo_);
    glEnableVertexAttribArray(attr_pos_);
    glVertexAttribPointer(attr_pos_, 2, GL_FLOAT, GL_FALSE,
                          4 * sizeof(float),
                          reinterpret_cast<void*>(0));
    glEnableVertexAttribArray(attr_uv_);
    glVertexAttribPointer(attr_uv_, 2, GL_FLOAT, GL_FALSE,
                          4 * sizeof(float),
                          reinterpret_cast<void*>(2 * sizeof(float)));

    glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);

    glDisableVertexAttribArray(attr_pos_);
    glDisableVertexAttribArray(attr_uv_);
}

} // namespace dmabuf
