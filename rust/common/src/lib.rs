pub mod socket;
pub mod v4l2;

pub const DEFAULT_SOCKET_PATH: &str = "/tmp/dma_buf_socket";

/// Metadata sent alongside each DMA-BUF fd. POD so it can travel directly
/// over a socket without serialisation.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameMeta {
    pub width: u32,
    pub height: u32,
    pub format: u32,   // DRM FOURCC
    pub stride: u32,   // bytes per line, plane 0
    pub size: u32,     // total buffer size in bytes
    pub sequence: u32, // frame counter
}
