# dma_buf_pipeline

Zero-copy DMA-BUF pipeline between two processes:

- **publisher**: captures frames from a V4L2 device, exports each MMAP buffer
  as a DMA-BUF (`VIDIOC_EXPBUF`), and forwards the file descriptor to the
  subscriber over a Unix domain socket using `SCM_RIGHTS`.
- **subscriber**: imports the received fd as `EGLImageKHR` via
  `EGL_LINUX_DMA_BUF_EXT`, binds it to a GL texture with
  `glEGLImageTargetTexture2DOES`, and renders a fullscreen quad on a
  Wayland xdg-toplevel surface.

A 1-byte ack on the same socket gates buffer re-queueing on the producer
side, so a buffer is never re-queued while the GPU is still sampling it.

## Layout

```
include/
  socket_utils.hpp   FrameMeta struct, send_frame / recv_frame (SCM_RIGHTS)
  v4l2_utils.hpp     V4l2Capture: open + REQBUFS + EXPBUF + DQBUF/QBUF loop
  wayland_utils.hpp  WaylandWindow: display + xdg-toplevel + wl_egl_window
  egl_utils.hpp      EglContext + QuadRenderer (DMA-BUF import + draw)
src/
  publisher.cpp      capture -> sendmsg(SCM_RIGHTS) -> wait ack -> requeue
  subscriber.cpp     recvmsg -> EGLImage -> draw -> swap -> ack
```

## Dependencies (Ubuntu/Debian)

```
sudo apt install build-essential cmake pkg-config \
    libwayland-dev wayland-protocols libwayland-egl1 \
    libegl1-mesa-dev libgles2-mesa-dev \
    libv4l-dev libdrm-dev
```

CMake `find_package(PkgConfig)` then locates: `wayland-client`,
`wayland-egl`, `wayland-protocols`, `wayland-scanner`, `egl`, `glesv2`,
`libv4l2`, `libdrm`. The xdg-shell client glue is generated from
`stable/xdg-shell/xdg-shell.xml` at configure time.

## Build

```
mkdir build && cd build
cmake ..
make -j
```

## Run

The subscriber needs a Wayland session (`WAYLAND_DISPLAY` set). On a vanilla
GNOME/KDE/Sway desktop that's already true.

```
# terminal 1
./publisher --device /dev/video0 --width 640 --height 480 --yuyv

# terminal 2
./subscriber
```

Both accept `--socket /path` to override the default `/tmp/dma_buf_socket`.

## Notes & limits

- **Format coverage.** Start with `--yuyv` (single plane, every UVC webcam
  supports it). The `--nv12` flag selects NV12 capture, but the subscriber
  only sets up a single-plane `EGLImage` import — to fully render NV12
  you'd need to send the second plane's offset/pitch in `FrameMeta`,
  pass `EGL_DMA_BUF_PLANE1_*` attributes during import, and add a sampler
  for the chroma plane. The translation table in
  [v4l2_utils.cpp](src/v4l2_utils.cpp#L185) already maps NV12 -> DRM.
- **YUYV shader.** When the driver imports YUYV as a single-plane RGBA
  texture (no native YUV sampling support), each texel covers two source
  pixels. The fragment shader picks the correct luma sample and converts
  with BT.601. On drivers that expose native YUYV sampling the shader
  output may look slightly different — adjust the channel order in
  [egl_utils.cpp](src/egl_utils.cpp#L173) if so.
- **fd lifetime.** The publisher exports each V4L2 buffer once at start
  and keeps the fd alive for the whole session. `SCM_RIGHTS` duplicates
  the fd into the subscriber, which closes its copy after EGL takes its
  own reference.
- **Flow control.** The 1-byte ack from subscriber back to publisher
  prevents the producer from re-queueing a buffer while it's still being
  sampled. Without it you'd race the GPU against `VIDIOC_QBUF`.

## Troubleshooting

- `EGL_EXT_image_dma_buf_import not advertised` — your GPU/driver doesn't
  support DMA-BUF import. Try Mesa on Intel/AMD or recent Nvidia.
- `VIDIOC_EXPBUF: Invalid argument` — the capture driver doesn't support
  exporting MMAP buffers as DMA-BUF. Most modern UVC drivers do; some
  embedded ISP drivers do not.
- `eglCreateImageKHR failed: 0x300c` (EGL_BAD_PARAMETER) — width/stride
  or DRM FOURCC don't match what the driver expects. Check that
  `meta.stride` is the V4L2 `bytesperline` for the selected format.
