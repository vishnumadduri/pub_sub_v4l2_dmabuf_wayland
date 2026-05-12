# Zero-Copy V4L2 DMA-BUF Wayland Pipeline

A publisher/subscriber pipeline that streams camera frames to a Wayland window
**without any CPU copy**. The publisher captures frames from a V4L2 device and
passes each buffer's DMA-BUF file descriptor to the subscriber over a Unix
domain socket. The subscriber imports the fd directly into EGL and renders it
with GLES2.

Two independent implementations are provided: **C++17** and **Rust**.

Tested on **Raspberry Pi 5** (VideoCore VII / V3D 7.1.7.0), Debian 13 trixie,
kernel 6.12.75-rpt-rpi-2712. Achieves a steady 30 fps at 640×480 YUYV.

## How it works

```
Publisher                                   Subscriber
────────────────────────────────────────────────────────────────────
V4L2 DQBUF → DMA-BUF fd
  sendmsg(SCM_RIGHTS) ──────── fd ────────► recvmsg(SCM_RIGHTS)
                                             eglCreateImageKHR
                                               (EGL_LINUX_DMA_BUF_EXT)
                                             glEGLImageTargetTexture2DOES
                                             draw full-screen quad
                                             eglSwapBuffers
  wait ack → V4L2 QBUF ◄──── 1-byte ack ─── close(fd)
```

The 1-byte ack handshake keeps publisher and subscriber in lock-step: the
publisher does not re-queue a V4L2 buffer until the subscriber has finished
sampling from it, preventing buffer reuse races without shared memory or
locking.

`GL_OES_EGL_image_external` handles YUV→RGB conversion on the GPU at sample
time — no software colour-space conversion is needed.

## Supported formats

| Flag | V4L2 format | DRM FOURCC | Notes |
|------|-------------|------------|-------|
| `--yuyv` (default) | `V4L2_PIX_FMT_YUYV` | `YUYV` | Single-plane packed |
| `--nv12` | `V4L2_PIX_FMT_NV12` | `NV12` | Semi-planar Y + interleaved UV |

NV12 requires two EGL plane descriptors (`EGL_DMA_BUF_PLANE0_*` for Y,
`EGL_DMA_BUF_PLANE1_*` for UV) both pointing at the same fd with different
offsets.

## Repository layout

```
.
├── CMakeLists.txt          C++ build definition
├── run.sh                  C++ start/stop script
├── include/                C++ shared headers
│   ├── socket_utils.hpp    FrameMeta, send_frame / recv_frame (SCM_RIGHTS)
│   ├── v4l2_utils.hpp      V4l2Capture: REQBUFS / EXPBUF / DQBUF/QBUF
│   ├── wayland_utils.hpp   WaylandWindow: xdg-toplevel + wl_egl_window
│   └── egl_utils.hpp       EglContext + QuadRenderer (DMA-BUF import + draw)
├── src/                    C++ source
│   ├── publisher.cpp
│   └── subscriber.cpp
└── rust/
    ├── Cargo.toml          Workspace (common, publisher, subscriber)
    ├── run.sh              Rust start/stop script
    ├── common/             Shared types, V4L2 helpers, socket helpers
    ├── publisher/          V4L2 capture + DMA-BUF sender
    └── subscriber/
        ├── build.rs        C probe — extracts EGL/GL constants from headers
        └── src/
            ├── egl.rs      EGL context, DMA-BUF import, GLES2 quad renderer
            └── wayland.rs  Wayland window (xdg-toplevel + EGL surface)
```

## Dependencies

### System packages (Debian/Ubuntu)

```bash
sudo apt install \
    cmake pkg-config build-essential \
    libv4l-dev libdrm-dev \
    libegl-dev libgles-dev \
    libwayland-dev wayland-protocols
```

For Rust, install [rustup](https://rustup.rs) or the distro `rustc`/`cargo`
packages (edition 2021 / Rust 1.65+).

### Runtime requirements

- A running Wayland compositor (`WAYLAND_DISPLAY` / `XDG_RUNTIME_DIR` set).
- EGL implementation advertising `EGL_EXT_image_dma_buf_import` (Mesa V3D,
  Intel i965/iris, AMD radeonsi, recent Nvidia).
- A V4L2 capture device exporting MMAP buffers as DMA-BUF (`VIDIOC_EXPBUF`).
  Most modern UVC drivers support this.

## Building

### C++

```bash
cmake -B build
cmake --build build -j$(nproc)
# Produces: build/publisher  build/subscriber
```

The CMake build generates the xdg-shell Wayland protocol glue from
`stable/xdg-shell/xdg-shell.xml` at configure time via `wayland-scanner`.

### Rust

```bash
cd rust
cargo build --release
# Produces: rust/target/release/publisher  rust/target/release/subscriber
```

`subscriber/build.rs` compiles a small C probe against the system EGL/GLES2
headers to read every `EGL_*` and `GL_*` token value at build time, writing
them to `$OUT_DIR/egl_consts.rs`. This avoids hardcoded magic numbers and
catches token value mismatches early (e.g. plane1 vs plane2 attribute offsets).

## Running

### C++ — `run.sh`

```bash
bash run.sh
```

Starts the publisher in the background, waits 300 ms for it to bind the socket,
then runs the subscriber in the foreground. Press **Ctrl+C** to stop both and
clean up the socket file.

### Rust — `rust/run.sh`

```bash
bash rust/run.sh             # auto-builds release binaries if missing
bash rust/run.sh --build     # force rebuild first
```

Same start/stop behaviour as the C++ script.

### Running manually

```bash
# terminal 1
./build/publisher --device /dev/video0 --width 640 --height 480 --yuyv

# terminal 2
./build/subscriber --socket /tmp/dma_buf_socket
```

### CLI reference

**publisher**

```
publisher [--device PATH] [--width W] [--height H] [--socket PATH] [--yuyv|--nv12]
```

**subscriber**

```
subscriber [--socket PATH]
```

### Environment overrides (scripts)

| Variable | Default | Description |
|----------|---------|-------------|
| `DEVICE` | `/dev/video0` | V4L2 capture device |
| `WIDTH` | `640` | Capture width |
| `HEIGHT` | `480` | Capture height |
| `FORMAT` | `--yuyv` | Format flag passed to publisher |
| `SOCKET` | `/tmp/dma_buf_socket` | Unix domain socket path |
| `WAYLAND_DISPLAY` | `wayland-0` | Wayland compositor socket |
| `XDG_RUNTIME_DIR` | `/run/user/<uid>` | Runtime directory |

Example — 1280×720 NV12 from a second camera:

```bash
DEVICE=/dev/video2 WIDTH=1280 HEIGHT=720 FORMAT=--nv12 bash rust/run.sh
```

## EGL extensions used

| Extension | Purpose |
|-----------|---------|
| `EGL_EXT_image_dma_buf_import` | Import DMA-BUF fd as `EGLImageKHR` |
| `EGL_EXT_image_dma_buf_import_modifiers` | Pass DRM modifiers (optional; probed at runtime) |
| `EGL_EXT_platform_wayland` / `EGL_KHR_platform_wayland` | Wayland-native EGL display selection |
| `GL_OES_EGL_image_external` | Bind `EGLImageKHR` as `samplerExternalOES` for GPU-side YUV→RGB |

## Troubleshooting

**`EGL_EXT_image_dma_buf_import not advertised`**
The GPU driver doesn't support DMA-BUF import over EGL. Check that Mesa is
installed and the correct DRI driver is active (`glxinfo | grep renderer`).

**`VIDIOC_EXPBUF: Invalid argument`**
The V4L2 driver doesn't support exporting MMAP buffers as DMA-BUF. Most UVC
webcam drivers do; some embedded ISP drivers do not.

**`eglCreateImageKHR failed: 0x300c` (EGL_BAD_PARAMETER)**
Width, stride, or DRM FOURCC mismatch. Verify `meta.stride` equals
`bytesperline` from `VIDIOC_S_FMT`, and that the DRM FOURCC matches the
selected V4L2 format.

**`NoCompositor` error from subscriber**
`WAYLAND_DISPLAY` is not set or points to a non-existent socket. Run
`ls $XDG_RUNTIME_DIR/wayland-*` to find the correct socket name and set
`WAYLAND_DISPLAY` accordingly.

**fd lifetime**
The publisher exports each V4L2 buffer fd once at startup and keeps it open
for the session. `SCM_RIGHTS` duplicates the fd into the subscriber; the
subscriber closes its copy after EGL takes its own internal reference via
`eglCreateImageKHR`.
