pub mod socket;
pub mod v4l2;

pub const DEFAULT_SOCKET_PATH: &str = "/tmp/dma_buf_socket";

/// Sent once at connection time: buffer geometry + count of DMA-BUF fds
/// that follow in the same SCM_RIGHTS message.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct Handshake {
    pub buf_count: u32,
    pub width:     u32,
    pub height:    u32,
    pub format:    u32, // DRM FOURCC
    pub stride:    u32, // bytes per line, plane 0
    pub size:      u32, // total buffer size in bytes
}

/// Used internally by EglContext::import_dmabuf; not sent over the socket.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameMeta {
    pub width:  u32,
    pub height: u32,
    pub format: u32,
    pub stride: u32,
    pub size:   u32,
}
