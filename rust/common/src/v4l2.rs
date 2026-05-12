use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use libc::{c_int, c_uint, c_ulong, ioctl, open, close, select, O_RDWR, O_NONBLOCK, O_CLOEXEC};

// ---- FOURCC helpers ---------------------------------------------------------

const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    a as u32 | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

pub const V4L2_PIX_FMT_YUYV:  u32 = fourcc(b'Y', b'U', b'Y', b'V');
pub const V4L2_PIX_FMT_UYVY:  u32 = fourcc(b'U', b'Y', b'V', b'Y');
pub const V4L2_PIX_FMT_NV12:  u32 = fourcc(b'N', b'V', b'1', b'2');
pub const V4L2_PIX_FMT_NV21:  u32 = fourcc(b'N', b'V', b'2', b'1');
pub const V4L2_PIX_FMT_RGB24: u32 = fourcc(b'R', b'G', b'B', b'3');
pub const V4L2_PIX_FMT_BGR24: u32 = fourcc(b'B', b'G', b'R', b'3');
pub const V4L2_PIX_FMT_ABGR32: u32 = fourcc(b'A', b'R', b'2', b'4');
pub const V4L2_PIX_FMT_XBGR32: u32 = fourcc(b'X', b'R', b'2', b'4');

// DRM FOURCC constants (from drm_fourcc.h)
pub const DRM_FORMAT_YUYV:    u32 = fourcc(b'Y', b'U', b'Y', b'V');
pub const DRM_FORMAT_UYVY:    u32 = fourcc(b'U', b'Y', b'V', b'Y');
pub const DRM_FORMAT_NV12:    u32 = fourcc(b'N', b'V', b'1', b'2');
pub const DRM_FORMAT_NV21:    u32 = fourcc(b'N', b'V', b'2', b'1');
pub const DRM_FORMAT_RGB888:  u32 = fourcc(b'R', b'G', b'2', b'4'); // "RG24"
pub const DRM_FORMAT_BGR888:  u32 = fourcc(b'B', b'G', b'2', b'4'); // "BG24"
pub const DRM_FORMAT_ARGB8888: u32 = fourcc(b'A', b'R', b'2', b'4'); // "AR24"
pub const DRM_FORMAT_XRGB8888: u32 = fourcc(b'X', b'R', b'2', b'4'); // "XR24"

pub fn v4l2_to_drm_fourcc(v4l2: u32) -> Option<u32> {
    match v4l2 {
        V4L2_PIX_FMT_YUYV   => Some(DRM_FORMAT_YUYV),
        V4L2_PIX_FMT_UYVY   => Some(DRM_FORMAT_UYVY),
        V4L2_PIX_FMT_NV12   => Some(DRM_FORMAT_NV12),
        V4L2_PIX_FMT_NV21   => Some(DRM_FORMAT_NV21),
        V4L2_PIX_FMT_RGB24  => Some(DRM_FORMAT_RGB888),
        V4L2_PIX_FMT_BGR24  => Some(DRM_FORMAT_BGR888),
        V4L2_PIX_FMT_ABGR32 => Some(DRM_FORMAT_ARGB8888),
        V4L2_PIX_FMT_XBGR32 => Some(DRM_FORMAT_XRGB8888),
        _ => None,
    }
}

// ---- V4L2 constants --------------------------------------------------------

const V4L2_BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const V4L2_MEMORY_MMAP: u32 = 1;
const V4L2_FIELD_ANY: u32 = 0;
const V4L2_CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
const V4L2_CAP_STREAMING:     u32 = 0x0400_0000;

// Ioctl numbers verified against <linux/videodev2.h> on aarch64 / x86-64.
const VIDIOC_QUERYCAP:  c_ulong = 0x8068_5600;
const VIDIOC_S_FMT:     c_ulong = 0xc0d0_5605;
const VIDIOC_REQBUFS:   c_ulong = 0xc014_5608;
const VIDIOC_QUERYBUF:  c_ulong = 0xc058_5609;
const VIDIOC_EXPBUF:    c_ulong = 0xc040_5610;
const VIDIOC_QBUF:      c_ulong = 0xc058_560f;
const VIDIOC_DQBUF:     c_ulong = 0xc058_5611;
const VIDIOC_STREAMON:  c_ulong = 0x4004_5612;
const VIDIOC_STREAMOFF: c_ulong = 0x4004_5613;

// ---- Kernel struct mirrors (verified offsets/sizes) ------------------------
// All layouts confirmed with offsetof/sizeof on aarch64 (Raspberry Pi 5).

#[repr(C)]
#[derive(Default)]
struct V4l2Capability {
    driver:       [u8; 16],
    card:         [u8; 32],
    bus_info:     [u8; 32],
    version:      u32,
    capabilities: u32,
    device_caps:  u32,
    _reserved:    [u32; 3],
} // 104 bytes

#[repr(C)]
#[derive(Default)]
struct V4l2PixFormat {
    width:        u32, // +0
    height:       u32, // +4
    pixelformat:  u32, // +8
    field:        u32, // +12
    bytesperline: u32, // +16
    sizeimage:    u32, // +20
    colorspace:   u32,
    priv_:        u32,
    flags:        u32,
    enc:          u32,
    quantization: u32,
    xfer_func:    u32,
} // 48 bytes

// v4l2_format: type at +0, padding at +4, union (200 bytes) at +8. Total 208.
#[repr(C)]
struct V4l2Format {
    type_: u32,           // +0
    _pad:  u32,           // +4  (union alignment on 64-bit)
    pix:   V4l2PixFormat, // +8  (48 bytes)
    _rest: [u8; 200 - size_of::<V4l2PixFormat>()], // pad union to 200 bytes
} // 8 + 200 = 208 bytes

// v4l2_requestbuffers: 20 bytes
#[repr(C)]
#[derive(Default)]
struct V4l2RequestBuffers {
    count:        u32,
    type_:        u32,
    memory:       u32,
    capabilities: u32,
    flags:        u8,
    _reserved:    [u8; 3],
}

// v4l2_buffer: 88 bytes. We only access a handful of fields; the rest are
// opaque payload (timestamp, timecode, union m).
#[repr(C)]
struct V4l2Buffer {
    index:     u32, // +0
    type_:     u32, // +4
    bytesused: u32, // +8
    flags:     u32, // +12
    field:     u32, // +16
    _pad1:     u32, // +20  (4-byte hole before 8-aligned timeval)
    _timestamp:[u8; 16], // +24
    _timecode: [u8; 16], // +40
    sequence:  u32, // +56
    memory:    u32, // +60
    _m:        u64, // +64  (union m; largest member is unsigned long = 8)
    length:    u32, // +72
    _tail:     [u8; 12], // +76 → total 88
}

// v4l2_exportbuffer: 64 bytes
#[repr(C)]
#[derive(Default)]
struct V4l2ExportBuffer {
    type_:     u32,
    index:     u32,
    plane:     u32,
    flags:     u32,
    fd:        i32,
    _reserved: [u32; 11], // 44 bytes → total 64
}

// ---- ioctl wrapper ---------------------------------------------------------

unsafe fn xioctl<T>(fd: RawFd, req: c_ulong, arg: *mut T) -> c_int {
    loop {
        let r = ioctl(fd, req, arg);
        if r == -1 && *libc::__errno_location() == libc::EINTR {
            continue;
        }
        return r;
    }
}

// ---- Public API ------------------------------------------------------------

pub struct V4l2BufInfo {
    pub dmabuf_fd: OwnedFd,
    pub index:     u32,
    pub length:    u32,
    pub bytesused: u32,
}

pub struct V4l2Capture {
    fd:               RawFd,
    pub width:        u32,
    pub height:       u32,
    pub stride:       u32,
    pub size_image:   u32,
    pub pixel_format: u32,
    streaming:        bool,
    pub buffers:      Vec<V4l2BufInfo>,
}

impl V4l2Capture {
    pub fn open(device: &str, width: u32, height: u32, pixel_format: u32) -> io::Result<Self> {
        let path = std::ffi::CString::new(device).unwrap();
        let fd = unsafe { open(path.as_ptr(), O_RDWR | O_NONBLOCK | O_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut cap = V4l2Capability::default();
        if unsafe { xioctl(fd, VIDIOC_QUERYCAP, &mut cap) } < 0 {
            unsafe { close(fd) };
            return Err(io::Error::last_os_error());
        }
        if cap.capabilities & V4L2_CAP_VIDEO_CAPTURE == 0
            || cap.capabilities & V4L2_CAP_STREAMING == 0
        {
            unsafe { close(fd) };
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("{device} does not support video capture + streaming"),
            ));
        }

        let mut fmt: V4l2Format = unsafe { std::mem::zeroed() };
        fmt.type_ = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        fmt.pix.width       = width;
        fmt.pix.height      = height;
        fmt.pix.pixelformat = pixel_format;
        fmt.pix.field       = V4L2_FIELD_ANY;
        if unsafe { xioctl(fd, VIDIOC_S_FMT, &mut fmt) } < 0 {
            unsafe { close(fd) };
            return Err(io::Error::last_os_error());
        }

        let actual_w   = fmt.pix.width;
        let actual_h   = fmt.pix.height;
        let stride     = fmt.pix.bytesperline;
        let size_image = fmt.pix.sizeimage;
        let actual_fmt = fmt.pix.pixelformat;

        let fcc = actual_fmt.to_le_bytes();
        println!(
            "V4L2: {}x{} {} stride={stride} size={size_image}",
            actual_w, actual_h,
            String::from_utf8_lossy(&fcc),
        );

        Ok(Self {
            fd,
            width: actual_w,
            height: actual_h,
            stride,
            size_image,
            pixel_format: actual_fmt,
            streaming: false,
            buffers: Vec::new(),
        })
    }

    pub fn start(&mut self, count: u32) -> io::Result<()> {
        let mut req = V4l2RequestBuffers::default();
        req.count  = count;
        req.type_  = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        req.memory = V4L2_MEMORY_MMAP;
        if unsafe { xioctl(self.fd, VIDIOC_REQBUFS, &mut req) } < 0 {
            return Err(io::Error::last_os_error());
        }
        if req.count < 2 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("need ≥2 buffers, got {}", req.count),
            ));
        }

        self.buffers = Vec::with_capacity(req.count as usize);

        for i in 0..req.count {
            let mut buf: V4l2Buffer = unsafe { std::mem::zeroed() };
            buf.type_  = V4L2_BUF_TYPE_VIDEO_CAPTURE;
            buf.memory = V4L2_MEMORY_MMAP;
            buf.index  = i;
            if unsafe { xioctl(self.fd, VIDIOC_QUERYBUF, &mut buf) } < 0 {
                return Err(io::Error::last_os_error());
            }

            let mut expbuf = V4l2ExportBuffer::default();
            expbuf.type_  = V4L2_BUF_TYPE_VIDEO_CAPTURE;
            expbuf.index  = i;
            expbuf.flags  = (libc::O_CLOEXEC | libc::O_RDWR) as u32;
            if unsafe { xioctl(self.fd, VIDIOC_EXPBUF, &mut expbuf) } < 0 {
                return Err(io::Error::last_os_error());
            }

            let length = buf.length;
            self.buffers.push(V4l2BufInfo {
                dmabuf_fd: unsafe { OwnedFd::from_raw_fd(expbuf.fd) },
                index: i,
                length,
                bytesused: 0,
            });

            // Reset and re-queue
            buf = unsafe { std::mem::zeroed() };
            buf.type_  = V4L2_BUF_TYPE_VIDEO_CAPTURE;
            buf.memory = V4L2_MEMORY_MMAP;
            buf.index  = i;
            if unsafe { xioctl(self.fd, VIDIOC_QBUF, &mut buf) } < 0 {
                return Err(io::Error::last_os_error());
            }
        }

        let mut buf_type: c_uint = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        if unsafe { xioctl(self.fd, VIDIOC_STREAMON, &mut buf_type) } < 0 {
            return Err(io::Error::last_os_error());
        }
        self.streaming = true;
        Ok(())
    }

    pub fn dequeue(&mut self) -> io::Result<u32> {
        let mut fds: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fds);
            libc::FD_SET(self.fd, &mut fds);
        }
        let mut tv = libc::timeval { tv_sec: 2, tv_usec: 0 };
        let r = unsafe {
            select(self.fd + 1, &mut fds, std::ptr::null_mut(), std::ptr::null_mut(), &mut tv)
        };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        if r == 0 {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "v4l2 dequeue timeout"));
        }

        let mut buf: V4l2Buffer = unsafe { std::mem::zeroed() };
        buf.type_  = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        buf.memory = V4L2_MEMORY_MMAP;
        if unsafe { xioctl(self.fd, VIDIOC_DQBUF, &mut buf) } < 0 {
            return Err(io::Error::last_os_error());
        }

        let idx = buf.index as usize;
        self.buffers[idx].bytesused = buf.bytesused;
        Ok(buf.index)
    }

    pub fn requeue(&self, index: u32) -> io::Result<()> {
        let mut buf: V4l2Buffer = unsafe { std::mem::zeroed() };
        buf.type_  = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        buf.memory = V4L2_MEMORY_MMAP;
        buf.index  = index;
        if unsafe { xioctl(self.fd, VIDIOC_QBUF, &mut buf) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn stop(&mut self) {
        if self.streaming {
            let mut buf_type: c_uint = V4L2_BUF_TYPE_VIDEO_CAPTURE;
            unsafe { xioctl(self.fd, VIDIOC_STREAMOFF, &mut buf_type) };
            self.streaming = false;
        }
        self.buffers.clear(); // OwnedFd drop closes each dmabuf fd

        if self.fd >= 0 {
            let mut req = V4l2RequestBuffers::default();
            req.count  = 0;
            req.type_  = V4L2_BUF_TYPE_VIDEO_CAPTURE;
            req.memory = V4L2_MEMORY_MMAP;
            unsafe { xioctl(self.fd, VIDIOC_REQBUFS, &mut req) };
        }
    }
}

impl Drop for V4l2Capture {
    fn drop(&mut self) {
        self.stop();
        if self.fd >= 0 {
            unsafe { close(self.fd) };
            self.fd = -1;
        }
    }
}
