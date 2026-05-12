// EGL context + DMA-BUF import + GLES2 quad renderer.
//
// Raw FFI against libEGL and libGLESv2 (linked by build.rs via pkg-config).
// Extension entry points are loaded lazily via eglGetProcAddress, matching
// the C++ implementation exactly.

use std::ffi::CStr;
use common::v4l2::{DRM_FORMAT_NV12, DRM_FORMAT_NV21, DRM_FORMAT_YUYV};
use common::FrameMeta;

// ---- EGL types and constants -----------------------------------------------

type EGLDisplay  = *mut std::ffi::c_void;
type EGLContext  = *mut std::ffi::c_void;
type EGLSurface  = *mut std::ffi::c_void;
type EGLConfig   = *mut std::ffi::c_void;
pub type EGLImageKHR = *mut std::ffi::c_void;
type EGLBoolean  = u32;
type EGLint      = i32;
type EGLenum     = u32;

const EGL_NO_DISPLAY:  EGLDisplay  = std::ptr::null_mut();
const EGL_NO_CONTEXT:  EGLContext  = std::ptr::null_mut();
const EGL_NO_SURFACE:  EGLSurface  = std::ptr::null_mut();
pub const EGL_NO_IMAGE_KHR: EGLImageKHR = std::ptr::null_mut();

// ---- GLES2 types and constants ---------------------------------------------

type GLuint   = u32;
type GLint    = i32;
type GLsizei  = i32;
type GLenum   = u32;
type GLfloat  = f32;
type GLchar   = libc::c_char;
type GlImageTargetTexFn = unsafe extern "C" fn(GLenum, EGLImageKHR);

include!(concat!(env!("OUT_DIR"), "/egl_consts.rs"));
const GL_FALSE: u8 = 0;

// ---- Raw EGL/GLES2 extern bindings -----------------------------------------

#[link(name = "EGL")]
extern "C" {
    fn eglQueryString(display: EGLDisplay, name: EGLint) -> *const libc::c_char;
    fn eglGetProcAddress(name: *const libc::c_char) -> *mut std::ffi::c_void;
    fn eglGetDisplay(native_display: *mut std::ffi::c_void) -> EGLDisplay;
    fn eglInitialize(dpy: EGLDisplay, major: *mut EGLint, minor: *mut EGLint) -> EGLBoolean;
    fn eglBindAPI(api: EGLenum) -> EGLBoolean;
    fn eglChooseConfig(
        dpy: EGLDisplay, attrib_list: *const EGLint,
        configs: *mut EGLConfig, config_size: EGLint, num_config: *mut EGLint,
    ) -> EGLBoolean;
    fn eglCreateContext(
        dpy: EGLDisplay, config: EGLConfig,
        share_context: EGLContext, attrib_list: *const EGLint,
    ) -> EGLContext;
    fn eglCreateWindowSurface(
        dpy: EGLDisplay, config: EGLConfig,
        native_window: *mut std::ffi::c_void, attrib_list: *const EGLint,
    ) -> EGLSurface;
    fn eglMakeCurrent(
        dpy: EGLDisplay, draw: EGLSurface, read: EGLSurface, ctx: EGLContext,
    ) -> EGLBoolean;
    fn eglSwapBuffers(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
    fn eglDestroyContext(dpy: EGLDisplay, ctx: EGLContext) -> EGLBoolean;
    fn eglDestroySurface(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
    fn eglTerminate(dpy: EGLDisplay) -> EGLBoolean;
    fn eglGetError() -> EGLint;
}

#[link(name = "GLESv2")]
extern "C" {
    fn glCreateShader(type_: GLenum) -> GLuint;
    fn glShaderSource(
        shader: GLuint, count: GLsizei,
        string: *const *const GLchar, length: *const GLint,
    );
    fn glCompileShader(shader: GLuint);
    fn glGetShaderiv(shader: GLuint, pname: GLenum, params: *mut GLint);
    fn glGetShaderInfoLog(shader: GLuint, max_len: GLsizei, length: *mut GLsizei, info: *mut GLchar);
    fn glDeleteShader(shader: GLuint);
    fn glCreateProgram() -> GLuint;
    fn glAttachShader(program: GLuint, shader: GLuint);
    fn glLinkProgram(program: GLuint);
    fn glGetProgramiv(program: GLuint, pname: GLenum, params: *mut GLint);
    fn glGetProgramInfoLog(program: GLuint, max_len: GLsizei, length: *mut GLsizei, info: *mut GLchar);
    fn glDeleteProgram(program: GLuint);
    fn glGetAttribLocation(program: GLuint, name: *const GLchar) -> GLint;
    fn glGetUniformLocation(program: GLuint, name: *const GLchar) -> GLint;
    fn glGenBuffers(n: GLsizei, buffers: *mut GLuint);
    fn glBindBuffer(target: GLenum, buffer: GLuint);
    fn glBufferData(target: GLenum, size: isize, data: *const std::ffi::c_void, usage: GLenum);
    fn glDeleteBuffers(n: GLsizei, buffers: *const GLuint);
    fn glGenTextures(n: GLsizei, textures: *mut GLuint);
    fn glBindTexture(target: GLenum, texture: GLuint);
    fn glTexParameteri(target: GLenum, pname: GLenum, param: GLint);
    fn glDeleteTextures(n: GLsizei, textures: *const GLuint);
    fn glViewport(x: GLint, y: GLint, width: GLsizei, height: GLsizei);
    fn glClearColor(r: GLfloat, g: GLfloat, b: GLfloat, a: GLfloat);
    fn glClear(mask: GLenum);
    fn glUseProgram(program: GLuint);
    fn glActiveTexture(texture: GLenum);
    fn glUniform1i(location: GLint, v0: GLint);
    fn glEnableVertexAttribArray(index: GLuint);
    fn glDisableVertexAttribArray(index: GLuint);
    fn glVertexAttribPointer(
        index: GLuint, size: GLint, type_: GLenum,
        normalized: u8, stride: GLsizei, pointer: *const std::ffi::c_void,
    );
    fn glDrawArrays(mode: GLenum, first: GLint, count: GLsizei);
    fn glGetString(name: GLenum) -> *const u8;
}

// ---- Helper: load EGL extension proc ---------------------------------------

fn get_proc(name: &[u8]) -> *mut std::ffi::c_void {
    unsafe { eglGetProcAddress(name.as_ptr() as *const libc::c_char) }
}

type EglGetPlatformDisplayFn =
    unsafe extern "C" fn(EGLenum, *mut std::ffi::c_void, *const EGLint) -> EGLDisplay;
type EglCreateImageKHRFn =
    unsafe extern "C" fn(EGLDisplay, EGLContext, EGLenum, *mut std::ffi::c_void, *const EGLint) -> EGLImageKHR;
type EglDestroyImageKHRFn =
    unsafe extern "C" fn(EGLDisplay, EGLImageKHR) -> EGLBoolean;

// ---- GLSL shaders ----------------------------------------------------------

const VS: &str = concat!(
    "attribute vec2 a_pos;\n",
    "attribute vec2 a_uv;\n",
    "varying vec2 v_uv;\n",
    "void main() {\n",
    "    v_uv = a_uv;\n",
    "    gl_Position = vec4(a_pos, 0.0, 1.0);\n",
    "}\n\0"
);

const FS_RGB: &str = concat!(
    "precision mediump float;\n",
    "varying vec2 v_uv;\n",
    "uniform sampler2D u_tex;\n",
    "void main() { gl_FragColor = texture2D(u_tex, v_uv); }\n\0"
);

// GL_OES_EGL_image_external provides hardware YUV→RGB conversion at sample time.
const FS_YUV: &str = concat!(
    "#extension GL_OES_EGL_image_external : require\n",
    "precision mediump float;\n",
    "varying vec2 v_uv;\n",
    "uniform samplerExternalOES u_tex;\n",
    "void main() { gl_FragColor = texture2D(u_tex, v_uv); }\n\0"
);

unsafe fn compile_shader(kind: GLenum, src: &str) -> Result<GLuint, String> {
    let sh = glCreateShader(kind);
    let ptr = src.as_ptr() as *const GLchar;
    let len = (src.len() - 1) as GLint; // exclude null terminator
    glShaderSource(sh, 1, &ptr, &len);
    glCompileShader(sh);
    let mut ok: GLint = 0;
    glGetShaderiv(sh, GL_COMPILE_STATUS, &mut ok);
    if ok == 0 {
        let mut buf = vec![0u8; 1024];
        glGetShaderInfoLog(sh, 1024, std::ptr::null_mut(), buf.as_mut_ptr() as *mut GLchar);
        glDeleteShader(sh);
        return Err(format!("shader compile: {}", String::from_utf8_lossy(&buf)));
    }
    Ok(sh)
}

unsafe fn link_program(vs_src: &str, fs_src: &str) -> Result<GLuint, String> {
    let vs = compile_shader(GL_VERTEX_SHADER,   vs_src)?;
    let fs = compile_shader(GL_FRAGMENT_SHADER, fs_src)?;
    let prog = glCreateProgram();
    glAttachShader(prog, vs);
    glAttachShader(prog, fs);
    glLinkProgram(prog);
    glDeleteShader(vs);
    glDeleteShader(fs);
    let mut ok: GLint = 0;
    glGetProgramiv(prog, GL_LINK_STATUS, &mut ok);
    if ok == 0 {
        let mut buf = vec![0u8; 1024];
        glGetProgramInfoLog(prog, 1024, std::ptr::null_mut(), buf.as_mut_ptr() as *mut GLchar);
        glDeleteProgram(prog);
        return Err(format!("program link: {}", String::from_utf8_lossy(&buf)));
    }
    Ok(prog)
}

// ---- EglContext -------------------------------------------------------------

pub struct EglContext {
    display:              EGLDisplay,
    context:              EGLContext,
    surface:              EGLSurface,
    has_modifiers:        bool,
    egl_create_image:     EglCreateImageKHRFn,
    egl_destroy_image:    EglDestroyImageKHRFn,
    pub gl_image_target:  GlImageTargetTexFn,
}

impl EglContext {
    pub fn init(
        wl_display: *mut std::ffi::c_void,
        egl_window: *mut std::ffi::c_void,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        unsafe {
            // Prefer the platform API so Mesa picks the right backend.
            let display = {
                let client_exts = eglQueryString(EGL_NO_DISPLAY, EGL_EXTENSIONS);
                let has_platform = !client_exts.is_null() && {
                    let s = CStr::from_ptr(client_exts).to_string_lossy();
                    s.contains("EGL_EXT_platform_wayland") || s.contains("EGL_KHR_platform_wayland")
                };
                if has_platform {
                    let get_plat: Option<EglGetPlatformDisplayFn> =
                        std::mem::transmute(get_proc(b"eglGetPlatformDisplayEXT\0"));
                    get_plat
                        .map(|f| f(EGL_PLATFORM_WAYLAND_EXT, wl_display, std::ptr::null()))
                        .unwrap_or(EGL_NO_DISPLAY)
                } else {
                    EGL_NO_DISPLAY
                }
            };
            let display = if display == EGL_NO_DISPLAY {
                eglGetDisplay(wl_display)
            } else {
                display
            };
            if display == EGL_NO_DISPLAY {
                return Err("eglGetDisplay failed".into());
            }

            let (mut major, mut minor) = (0, 0);
            if eglInitialize(display, &mut major, &mut minor) == 0 {
                return Err(format!("eglInitialize failed: 0x{:x}", eglGetError()).into());
            }
            println!("EGL {major}.{minor}");

            if eglBindAPI(EGL_OPENGL_ES_API) == 0 {
                return Err("eglBindAPI(GLES) failed".into());
            }

            let config_attribs = [
                EGL_SURFACE_TYPE,    EGL_WINDOW_BIT,
                EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
                EGL_RED_SIZE,   8,
                EGL_GREEN_SIZE, 8,
                EGL_BLUE_SIZE,  8,
                EGL_ALPHA_SIZE, 8,
                EGL_NONE,
            ];
            let mut config: EGLConfig = std::ptr::null_mut();
            let mut num_configs = 0;
            if eglChooseConfig(display, config_attribs.as_ptr(), &mut config, 1, &mut num_configs) == 0
                || num_configs < 1
            {
                return Err("eglChooseConfig failed".into());
            }

            let ctx_attribs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
            let context = eglCreateContext(display, config, EGL_NO_CONTEXT, ctx_attribs.as_ptr());
            if context == EGL_NO_CONTEXT {
                return Err(format!("eglCreateContext failed: 0x{:x}", eglGetError()).into());
            }

            let surface = eglCreateWindowSurface(display, config, egl_window, std::ptr::null());
            if surface == EGL_NO_SURFACE {
                return Err(format!("eglCreateWindowSurface failed: 0x{:x}", eglGetError()).into());
            }

            if eglMakeCurrent(display, surface, surface, context) == 0 {
                return Err(format!("eglMakeCurrent failed: 0x{:x}", eglGetError()).into());
            }

            let exts = CStr::from_ptr(eglQueryString(display, EGL_EXTENSIONS))
                .to_string_lossy()
                .to_string();
            if !exts.contains("EGL_EXT_image_dma_buf_import") {
                eprintln!("warning: EGL_EXT_image_dma_buf_import not advertised");
            }
            let has_modifiers = exts.contains("EGL_EXT_image_dma_buf_import_modifiers");

            let egl_create_image: EglCreateImageKHRFn =
                std::mem::transmute(get_proc(b"eglCreateImageKHR\0"));
            let egl_destroy_image: EglDestroyImageKHRFn =
                std::mem::transmute(get_proc(b"eglDestroyImageKHR\0"));
            let gl_image_target: GlImageTargetTexFn =
                std::mem::transmute(get_proc(b"glEGLImageTargetTexture2DOES\0"));

            if (egl_create_image as usize == 0)
                || (egl_destroy_image as usize == 0)
                || (gl_image_target as usize == 0)
            {
                return Err("missing EGL/GLES extension entry points".into());
            }

            let renderer = CStr::from_ptr(glGetString(GL_RENDERER) as *const libc::c_char);
            println!("GL_RENDERER: {}", renderer.to_string_lossy());

            Ok(Self {
                display,
                context,
                surface,
                has_modifiers,
                egl_create_image,
                egl_destroy_image,
                gl_image_target,
            })
        }
    }

    pub fn import_dmabuf(&self, dmabuf_fd: i32, meta: &FrameMeta) -> EGLImageKHR {
        let mut attrs: Vec<EGLint> = vec![
            EGL_WIDTH,                  meta.width as EGLint,
            EGL_HEIGHT,                 meta.height as EGLint,
            EGL_LINUX_DRM_FOURCC_EXT,   meta.format as EGLint,
            EGL_DMA_BUF_PLANE0_FD_EXT,     dmabuf_fd,
            EGL_DMA_BUF_PLANE0_OFFSET_EXT, 0,
            EGL_DMA_BUF_PLANE0_PITCH_EXT,  meta.stride as EGLint,
        ];
        if self.has_modifiers {
            attrs.extend_from_slice(&[
                EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT, 0,
                EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT, 0,
            ]);
        }

        // NV12/NV21: semi-planar — Y at offset 0, UV at stride*height,
        // both planes in the same fd.
        if meta.format == DRM_FORMAT_NV12 || meta.format == DRM_FORMAT_NV21 {
            let uv_off = (meta.stride * meta.height) as EGLint;
            attrs.extend_from_slice(&[
                EGL_DMA_BUF_PLANE1_FD_EXT,     dmabuf_fd,
                EGL_DMA_BUF_PLANE1_OFFSET_EXT, uv_off,
                EGL_DMA_BUF_PLANE1_PITCH_EXT,  meta.stride as EGLint,
            ]);
            if self.has_modifiers {
                attrs.extend_from_slice(&[
                    EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT, 0,
                    EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT, 0,
                ]);
            }
        }
        attrs.push(EGL_NONE);

        let img = unsafe {
            (self.egl_create_image)(
                self.display,
                EGL_NO_CONTEXT,
                EGL_LINUX_DMA_BUF_EXT,
                std::ptr::null_mut(),
                attrs.as_ptr(),
            )
        };
        if img == EGL_NO_IMAGE_KHR {
            eprintln!("eglCreateImageKHR failed: 0x{:x}", unsafe { eglGetError() });
        }
        img
    }

    pub fn destroy_image(&self, img: EGLImageKHR) {
        if img != EGL_NO_IMAGE_KHR {
            unsafe { (self.egl_destroy_image)(self.display, img) };
        }
    }

    pub fn swap_buffers(&self) {
        unsafe { eglSwapBuffers(self.display, self.surface) };
    }
}

impl Drop for EglContext {
    fn drop(&mut self) {
        unsafe {
            eglMakeCurrent(self.display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
            if self.surface != EGL_NO_SURFACE { eglDestroySurface(self.display, self.surface); }
            if self.context != EGL_NO_CONTEXT { eglDestroyContext(self.display, self.context); }
            eglTerminate(self.display);
        }
    }
}

// ---- QuadRenderer ----------------------------------------------------------

pub struct QuadRenderer {
    program:    GLuint,
    texture:    GLuint,
    vbo:        GLuint,
    attr_pos:   GLint,
    attr_uv:    GLint,
    uni_tex:    GLint,
    tex_target: GLenum,
    viewport_w: i32,
    viewport_h: i32,
}

impl QuadRenderer {
    pub fn init(drm_fourcc: u32) -> Result<Self, String> {
        let yuv = matches!(drm_fourcc, x if x == DRM_FORMAT_YUYV || x == DRM_FORMAT_NV12 || x == DRM_FORMAT_NV21);
        let tex_target = if yuv { GL_TEXTURE_EXTERNAL_OES } else { GL_TEXTURE_2D };

        let program = unsafe { link_program(VS, if yuv { FS_YUV } else { FS_RGB }) }?;

        let (attr_pos, attr_uv, uni_tex) = unsafe {
            let ap = glGetAttribLocation(program, c"a_pos".as_ptr());
            let au = glGetAttribLocation(program, c"a_uv".as_ptr());
            let ut = glGetUniformLocation(program, c"u_tex".as_ptr());
            (ap, au, ut)
        };

        // Screen bottom = texture bottom (v=1): camera row 0 is at texture top.
        // We swap v so the image appears right-side up on screen.
        #[rustfmt::skip]
        let verts: [f32; 16] = [
            -1.0, -1.0,  0.0, 1.0,
             1.0, -1.0,  1.0, 1.0,
            -1.0,  1.0,  0.0, 0.0,
             1.0,  1.0,  1.0, 0.0,
        ];

        let (mut vbo, mut texture) = (0u32, 0u32);
        unsafe {
            glGenBuffers(1, &mut vbo);
            glBindBuffer(GL_ARRAY_BUFFER, vbo);
            glBufferData(
                GL_ARRAY_BUFFER,
                (verts.len() * 4) as isize,
                verts.as_ptr().cast(),
                GL_STATIC_DRAW,
            );

            glGenTextures(1, &mut texture);
            glBindTexture(tex_target, texture);
            glTexParameteri(tex_target, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
            glTexParameteri(tex_target, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
            glTexParameteri(tex_target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            glTexParameteri(tex_target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        }

        Ok(Self {
            program,
            texture,
            vbo,
            attr_pos,
            attr_uv,
            uni_tex,
            tex_target,
            viewport_w: 0,
            viewport_h: 0,
        })
    }

    pub fn set_viewport(&mut self, w: i32, h: i32) {
        self.viewport_w = w;
        self.viewport_h = h;
    }

    pub fn draw(&self, image: EGLImageKHR, image_target_fn: GlImageTargetTexFn) {
        unsafe {
            glViewport(0, 0, self.viewport_w, self.viewport_h);
            glClearColor(0.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);

            glBindTexture(self.tex_target, self.texture);
            // Re-attach the EGLImage as backing every frame — no copy, just
            // re-pointing the texture at the DMA-BUF pages.
            image_target_fn(self.tex_target, image);

            glUseProgram(self.program);
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(self.tex_target, self.texture);
            glUniform1i(self.uni_tex, 0);

            glBindBuffer(GL_ARRAY_BUFFER, self.vbo);
            glEnableVertexAttribArray(self.attr_pos as GLuint);
            glVertexAttribPointer(
                self.attr_pos as GLuint, 2, GL_FLOAT, GL_FALSE,
                (4 * 4) as GLsizei, std::ptr::null(),
            );
            glEnableVertexAttribArray(self.attr_uv as GLuint);
            glVertexAttribPointer(
                self.attr_uv as GLuint, 2, GL_FLOAT, GL_FALSE,
                (4 * 4) as GLsizei, (2 * 4) as *const std::ffi::c_void,
            );

            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);

            glDisableVertexAttribArray(self.attr_pos as GLuint);
            glDisableVertexAttribArray(self.attr_uv as GLuint);
        }
    }
}

impl Drop for QuadRenderer {
    fn drop(&mut self) {
        unsafe {
            if self.texture != 0 { glDeleteTextures(1, &self.texture); }
            if self.vbo     != 0 { glDeleteBuffers(1, &self.vbo); }
            if self.program != 0 { glDeleteProgram(self.program); }
        }
    }
}
