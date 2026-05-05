#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace dmabuf {

struct V4l2Buffer {
    int dmabuf_fd = -1;    // exported via VIDIOC_EXPBUF
    uint32_t index = 0;
    uint32_t length = 0;
    uint32_t bytesused = 0; // filled by dequeue(); bytes written by the driver this frame
};

// Thin RAII wrapper around a V4L2 capture device configured for MMAP buffers
// that are exported as DMA-BUF file descriptors. Single-plane YUV formats
// (YUYV, NV12) are the primary targets; the fd is passed zero-copy to EGL.
class V4l2Capture {
public:
    V4l2Capture() = default;
    ~V4l2Capture();

    V4l2Capture(const V4l2Capture&) = delete;
    V4l2Capture& operator=(const V4l2Capture&) = delete;

    // Open the device and configure it. width/height are desired values; the
    // driver may adjust them and the actual values can be read from
    // width()/height() after this call. pixel_format is a V4L2 FOURCC.
    bool open(const std::string& device,
              uint32_t width,
              uint32_t height,
              uint32_t pixel_format);

    // Allocate count buffers via REQBUFS, export each one via EXPBUF, queue
    // them, and start streaming.
    bool start(uint32_t count = 4);

    // Stop streaming and free buffers.
    void stop();

    // Block until a buffer becomes available, then dequeue it. The returned
    // index refers to a buffer in buffers().
    bool dequeue(uint32_t& index);

    // Re-queue a previously dequeued buffer.
    bool requeue(uint32_t index);

    int fd() const { return fd_; }
    uint32_t width() const { return width_; }
    uint32_t height() const { return height_; }
    uint32_t stride() const { return stride_; }
    uint32_t size_image() const { return size_image_; }
    uint32_t pixel_format() const { return pixel_format_; }
    const std::vector<V4l2Buffer>& buffers() const { return buffers_; }

private:
    int fd_ = -1;
    uint32_t width_ = 0;
    uint32_t height_ = 0;
    uint32_t stride_ = 0;
    uint32_t size_image_ = 0;
    uint32_t pixel_format_ = 0;
    bool streaming_ = false;
    std::vector<V4l2Buffer> buffers_;
};

// Translate a V4L2 FOURCC to its DRM FOURCC equivalent. Returns 0 if there
// is no straightforward mapping.
uint32_t v4l2_to_drm_fourcc(uint32_t v4l2_fourcc);

} // namespace dmabuf
