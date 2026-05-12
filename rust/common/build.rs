use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir   = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let probe_c   = out_dir.join("v4l2_probe.c");
    let probe_bin = out_dir.join("v4l2_probe");
    let consts_rs = out_dir.join("v4l2_consts.rs");

    std::fs::write(&probe_c, CONST_PROBE_C).unwrap();

    let status = Command::new("cc")
        .arg(&probe_c)
        .arg("-o")
        .arg(&probe_bin)
        .status()
        .expect("failed to compile V4L2/DRM constant probe");
    assert!(
        status.success(),
        "V4L2/DRM constant probe failed — ensure linux-libc-dev and libdrm-dev are installed"
    );

    let out = Command::new(&probe_bin)
        .output()
        .expect("failed to run V4L2/DRM constant probe");
    assert!(out.status.success());

    std::fs::write(&consts_rs, out.stdout).unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}

const CONST_PROBE_C: &str = r#"
#include <stdio.h>
#include <linux/videodev2.h>
#include <drm/drm_fourcc.h>

/* private u32 constant */
#define EU(n)   printf("const " #n ": u32 = 0x%08X;\n", (unsigned int)(n))
/* public u32 constant (exported from the crate) */
#define EPU(n)  printf("pub const " #n ": u32 = 0x%08X;\n", (unsigned int)(n))
/* ioctl request number — unsigned long = u64 on 64-bit Linux */
#define EL(n)   printf("const " #n ": u64 = 0x%016lX;\n", (unsigned long)(n))

int main(void) {
    /* ----- buffer type / memory / field / capability ----- */
    EU(V4L2_BUF_TYPE_VIDEO_CAPTURE);
    EU(V4L2_MEMORY_MMAP);
    EU(V4L2_FIELD_ANY);
    EU(V4L2_CAP_VIDEO_CAPTURE);
    EU(V4L2_CAP_STREAMING);

    /* ----- VIDIOC ioctl numbers (encoded with struct sizes via _IOR/_IOW/_IOWR) ----- */
    EL(VIDIOC_QUERYCAP);
    EL(VIDIOC_S_FMT);
    EL(VIDIOC_REQBUFS);
    EL(VIDIOC_QUERYBUF);
    EL(VIDIOC_EXPBUF);
    EL(VIDIOC_QBUF);
    EL(VIDIOC_DQBUF);
    EL(VIDIOC_STREAMON);
    EL(VIDIOC_STREAMOFF);

    /* ----- V4L2 pixel format FOURCCs ----- */
    EPU(V4L2_PIX_FMT_YUYV);
    EPU(V4L2_PIX_FMT_UYVY);
    EPU(V4L2_PIX_FMT_NV12);
    EPU(V4L2_PIX_FMT_NV21);
    EPU(V4L2_PIX_FMT_RGB24);
    EPU(V4L2_PIX_FMT_BGR24);
    EPU(V4L2_PIX_FMT_ABGR32);
    EPU(V4L2_PIX_FMT_XBGR32);

    /* ----- DRM pixel format FOURCCs ----- */
    EPU(DRM_FORMAT_YUYV);
    EPU(DRM_FORMAT_UYVY);
    EPU(DRM_FORMAT_NV12);
    EPU(DRM_FORMAT_NV21);
    EPU(DRM_FORMAT_RGB888);
    EPU(DRM_FORMAT_BGR888);
    EPU(DRM_FORMAT_ARGB8888);
    EPU(DRM_FORMAT_XRGB8888);

    return 0;
}
"#;
