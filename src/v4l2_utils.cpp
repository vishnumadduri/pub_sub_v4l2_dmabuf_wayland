#include "v4l2_utils.hpp"

#include <cerrno>
#include <cstdio>
#include <cstring>
#include <fcntl.h>
#include <linux/videodev2.h>
#include <sys/ioctl.h>
#include <sys/select.h>
#include <unistd.h>

// drm_fourcc.h provides DRM_FORMAT_* constants. The few we actually use are
// guaranteed to exist in any recent kernel header, but we still guard each
// translation in v4l2_to_drm_fourcc().
#include <drm_fourcc.h>

namespace dmabuf {

// Retry an ioctl() on EINTR. Standard V4L2 idiom.
static int xioctl(int fd, unsigned long req, void* arg) {
    int r;
    do {
        r = ::ioctl(fd, req, arg);
    } while (r == -1 && errno == EINTR);
    return r;
}

V4l2Capture::~V4l2Capture() {
    stop();
    if (fd_ >= 0) ::close(fd_);
}

bool V4l2Capture::open(const std::string& device,
                       uint32_t width,
                       uint32_t height,
                       uint32_t pixel_format) {
    fd_ = ::open(device.c_str(), O_RDWR | O_NONBLOCK | O_CLOEXEC);
    if (fd_ < 0) {
        std::perror(("open " + device).c_str());
        return false;
    }

    v4l2_capability cap{};
    if (xioctl(fd_, VIDIOC_QUERYCAP, &cap) < 0) {
        std::perror("VIDIOC_QUERYCAP");
        return false;
    }
    if (!(cap.capabilities & V4L2_CAP_VIDEO_CAPTURE) ||
        !(cap.capabilities & V4L2_CAP_STREAMING)) {
        std::fprintf(stderr, "%s does not support video capture + streaming\n",
                     device.c_str());
        return false;
    }

    v4l2_format fmt{};
    fmt.type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    fmt.fmt.pix.width = width;
    fmt.fmt.pix.height = height;
    fmt.fmt.pix.pixelformat = pixel_format;
    fmt.fmt.pix.field = V4L2_FIELD_ANY;
    if (xioctl(fd_, VIDIOC_S_FMT, &fmt) < 0) {
        std::perror("VIDIOC_S_FMT");
        return false;
    }

    // The driver may have adjusted everything — pick up the actual values.
    width_ = fmt.fmt.pix.width;
    height_ = fmt.fmt.pix.height;
    stride_ = fmt.fmt.pix.bytesperline;
    size_image_ = fmt.fmt.pix.sizeimage;
    pixel_format_ = fmt.fmt.pix.pixelformat;

    char fourcc[5] = {0};
    std::memcpy(fourcc, &pixel_format_, 4);
    std::printf("V4L2: %ux%u %s stride=%u size=%u\n",
                width_, height_, fourcc, stride_, size_image_);
    return true;
}

bool V4l2Capture::start(uint32_t count) {
    v4l2_requestbuffers req{};
    req.count = count;
    req.type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    req.memory = V4L2_MEMORY_MMAP; // we'll export each MMAP buffer as a dma-buf
    if (xioctl(fd_, VIDIOC_REQBUFS, &req) < 0) {
        std::perror("VIDIOC_REQBUFS");
        return false;
    }
    if (req.count < 2) {
        std::fprintf(stderr, "Need at least 2 buffers, got %u\n", req.count);
        return false;
    }

    buffers_.clear();
    buffers_.resize(req.count);

    for (uint32_t i = 0; i < req.count; ++i) {
        v4l2_buffer buf{};
        buf.type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        buf.memory = V4L2_MEMORY_MMAP;
        buf.index = i;
        if (xioctl(fd_, VIDIOC_QUERYBUF, &buf) < 0) {
            std::perror("VIDIOC_QUERYBUF");
            return false;
        }

        // Export this buffer as a DMA-BUF file descriptor. The fd stays valid
        // until we close it; we only need to send it to the subscriber once
        // (per index), since the subscriber re-uses it across frames.
        v4l2_exportbuffer expbuf{};
        expbuf.type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        expbuf.index = i;
        expbuf.flags = O_CLOEXEC | O_RDWR;
        if (xioctl(fd_, VIDIOC_EXPBUF, &expbuf) < 0) {
            std::perror("VIDIOC_EXPBUF");
            return false;
        }

        buffers_[i].dmabuf_fd = expbuf.fd;
        buffers_[i].index = i;
        buffers_[i].length = buf.length;

        if (xioctl(fd_, VIDIOC_QBUF, &buf) < 0) {
            std::perror("VIDIOC_QBUF");
            return false;
        }
    }

    v4l2_buf_type type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    if (xioctl(fd_, VIDIOC_STREAMON, &type) < 0) {
        std::perror("VIDIOC_STREAMON");
        return false;
    }
    streaming_ = true;
    return true;
}

void V4l2Capture::stop() {
    if (streaming_ && fd_ >= 0) {
        v4l2_buf_type type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        xioctl(fd_, VIDIOC_STREAMOFF, &type);
        streaming_ = false;
    }
    for (auto& b : buffers_) {
        if (b.dmabuf_fd >= 0) ::close(b.dmabuf_fd);
        b.dmabuf_fd = -1;
    }
    buffers_.clear();

    if (fd_ >= 0) {
        // Releasing all buffers is good hygiene before closing.
        v4l2_requestbuffers req{};
        req.count = 0;
        req.type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
        req.memory = V4L2_MEMORY_MMAP;
        xioctl(fd_, VIDIOC_REQBUFS, &req);
    }
}

bool V4l2Capture::dequeue(uint32_t& index) {
    // Wait up to 2s for the driver to fill a buffer. Using select() keeps
    // this simple; an epoll-based loop is the natural extension.
    fd_set fds;
    FD_ZERO(&fds);
    FD_SET(fd_, &fds);
    timeval tv{};
    tv.tv_sec = 2;
    int r = ::select(fd_ + 1, &fds, nullptr, nullptr, &tv);
    if (r < 0) {
        if (errno == EINTR) return false;
        std::perror("select");
        return false;
    }
    if (r == 0) {
        std::fprintf(stderr, "v4l2 dequeue timeout\n");
        return false;
    }

    v4l2_buffer buf{};
    buf.type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    buf.memory = V4L2_MEMORY_MMAP;
    if (xioctl(fd_, VIDIOC_DQBUF, &buf) < 0) {
        std::perror("VIDIOC_DQBUF");
        return false;
    }
    index = buf.index;
    return true;
}

bool V4l2Capture::requeue(uint32_t index) {
    v4l2_buffer buf{};
    buf.type = V4L2_BUF_TYPE_VIDEO_CAPTURE;
    buf.memory = V4L2_MEMORY_MMAP;
    buf.index = index;
    if (xioctl(fd_, VIDIOC_QBUF, &buf) < 0) {
        std::perror("VIDIOC_QBUF");
        return false;
    }
    return true;
}

uint32_t v4l2_to_drm_fourcc(uint32_t v4l2_fourcc) {
    // V4L2 and DRM use compatible 4-byte FOURCC encodings, but the actual
    // codes for the same pixel layout sometimes differ. Translate explicitly.
    switch (v4l2_fourcc) {
        case V4L2_PIX_FMT_YUYV:  return DRM_FORMAT_YUYV;
        case V4L2_PIX_FMT_UYVY:  return DRM_FORMAT_UYVY;
        case V4L2_PIX_FMT_NV12:  return DRM_FORMAT_NV12;
        case V4L2_PIX_FMT_NV21:  return DRM_FORMAT_NV21;
        case V4L2_PIX_FMT_RGB24: return DRM_FORMAT_RGB888;
        case V4L2_PIX_FMT_BGR24: return DRM_FORMAT_BGR888;
        case V4L2_PIX_FMT_ABGR32: return DRM_FORMAT_ARGB8888;
        case V4L2_PIX_FMT_XBGR32: return DRM_FORMAT_XRGB8888;
        default: return 0;
    }
}

} // namespace dmabuf
