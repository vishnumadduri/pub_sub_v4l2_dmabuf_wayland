#pragma once

#include <cstdint>
#include <string>
#include <sys/socket.h>
#include <sys/un.h>

namespace dmabuf {

// Metadata sent alongside each DMA-BUF fd. Keep it POD/trivially-copyable so
// it can travel directly over a socket.
struct FrameMeta {
    uint32_t width;
    uint32_t height;
    uint32_t format;   // DRM FOURCC (e.g. DRM_FORMAT_YUYV)
    uint32_t stride;   // bytes per line for plane 0
    uint32_t size;     // total buffer size in bytes
    uint32_t sequence; // frame counter, useful for FPS / debugging
};

constexpr const char* kDefaultSocketPath = "/tmp/dma_buf_socket";

// RAII wrapper for a socket file descriptor.
class ScopedFd {
public:
    ScopedFd() = default;
    explicit ScopedFd(int fd) : fd_(fd) {}
    ~ScopedFd();

    ScopedFd(const ScopedFd&) = delete;
    ScopedFd& operator=(const ScopedFd&) = delete;

    ScopedFd(ScopedFd&& other) noexcept : fd_(other.fd_) { other.fd_ = -1; }
    ScopedFd& operator=(ScopedFd&& other) noexcept;

    int get() const { return fd_; }
    int release();
    void reset(int fd = -1);
    explicit operator bool() const { return fd_ >= 0; }

private:
    int fd_ = -1;
};

// Server side: create, bind, and listen on a Unix domain stream socket.
// Removes any stale socket file at path. Returns -1 on failure.
int create_listening_socket(const std::string& path);

// Client side: connect to a Unix domain stream socket.
int connect_to_socket(const std::string& path);

// Send the metadata struct followed by the DMA-BUF fd via SCM_RIGHTS
// in a single sendmsg() call. Returns 0 on success, -1 on failure.
int send_frame(int sock_fd, const FrameMeta& meta, int dmabuf_fd);

// Receive a metadata struct + a single DMA-BUF fd. The caller becomes the
// owner of the returned fd and is responsible for closing it.
// Returns 0 on success, -1 on failure, 1 on clean EOF.
int recv_frame(int sock_fd, FrameMeta& meta, int& dmabuf_fd);

} // namespace dmabuf
