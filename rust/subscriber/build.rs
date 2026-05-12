use std::path::PathBuf;
use std::process::Command;

fn main() {
    let egl  = pkg_config::probe_library("egl").unwrap();
    let gles = pkg_config::probe_library("glesv2").unwrap();
    pkg_config::probe_library("wayland-egl").unwrap();

    let out_dir   = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let probe_c   = out_dir.join("const_probe.c");
    let probe_bin = out_dir.join("const_probe");
    let consts_rs = out_dir.join("egl_consts.rs");

    std::fs::write(&probe_c, CONST_PROBE_C).unwrap();

    let mut cmd = Command::new("cc");
    for p in egl.include_paths.iter().chain(gles.include_paths.iter()) {
        cmd.arg(format!("-I{}", p.display()));
    }
    cmd.arg(&probe_c).arg("-o").arg(&probe_bin);
    assert!(
        cmd.status().expect("failed to compile EGL/GL constant probe").success(),
        "EGL/GL constant probe compilation failed — ensure EGL and GLES2 headers are installed"
    );

    let out = Command::new(&probe_bin)
        .output()
        .expect("failed to run EGL/GL constant probe");
    assert!(out.status.success());

    std::fs::write(&consts_rs, out.stdout).unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}

const CONST_PROBE_C: &str = r#"
#include <stdio.h>
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <GLES2/gl2ext.h>

#define EI(n) printf("const " #n ": i32 = 0x%04X;\n", (unsigned)(n))
#define EU(n) printf("const " #n ": u32 = 0x%04X;\n", (unsigned)(n))

int main(void) {
    EI(EGL_EXTENSIONS);
    EU(EGL_OPENGL_ES_API);
    EI(EGL_SURFACE_TYPE);
    EI(EGL_WINDOW_BIT);
    EI(EGL_RENDERABLE_TYPE);
    EI(EGL_OPENGL_ES2_BIT);
    EI(EGL_RED_SIZE);
    EI(EGL_GREEN_SIZE);
    EI(EGL_BLUE_SIZE);
    EI(EGL_ALPHA_SIZE);
    EI(EGL_NONE);
    EI(EGL_CONTEXT_CLIENT_VERSION);
    EI(EGL_WIDTH);
    EI(EGL_HEIGHT);
    EU(EGL_LINUX_DMA_BUF_EXT);
    EI(EGL_LINUX_DRM_FOURCC_EXT);
    EI(EGL_DMA_BUF_PLANE0_FD_EXT);
    EI(EGL_DMA_BUF_PLANE0_OFFSET_EXT);
    EI(EGL_DMA_BUF_PLANE0_PITCH_EXT);
    EI(EGL_DMA_BUF_PLANE1_FD_EXT);
    EI(EGL_DMA_BUF_PLANE1_OFFSET_EXT);
    EI(EGL_DMA_BUF_PLANE1_PITCH_EXT);
    EI(EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT);
    EI(EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT);
    EI(EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT);
    EI(EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT);
    EU(EGL_PLATFORM_WAYLAND_EXT);
    EU(GL_FRAGMENT_SHADER);
    EU(GL_VERTEX_SHADER);
    EU(GL_COMPILE_STATUS);
    EU(GL_LINK_STATUS);
    EU(GL_TEXTURE_2D);
    EU(GL_TEXTURE_EXTERNAL_OES);
    EU(GL_TEXTURE_MIN_FILTER);
    EU(GL_TEXTURE_MAG_FILTER);
    EU(GL_TEXTURE_WRAP_S);
    EU(GL_TEXTURE_WRAP_T);
    EU(GL_LINEAR);
    EU(GL_CLAMP_TO_EDGE);
    EU(GL_ARRAY_BUFFER);
    EU(GL_STATIC_DRAW);
    EU(GL_COLOR_BUFFER_BIT);
    EU(GL_FLOAT);
    EU(GL_TRIANGLE_STRIP);
    EU(GL_RENDERER);
    EU(GL_TEXTURE0);
    return 0;
}
"#;
