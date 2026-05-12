#pragma once

#include <cstdint>
#include <string>
#include <vector>
#include <sys/socket.h>
#include <sys/un.h>

namespace dmabuf {

// Sent once at connection time: buffer geometry + count of DMA-BUF fds
// that follow in the same SCM_RIGHTS message.
struct HandshakeMsg {
    uint32_t buf_count;
    uint32_t width;
    uint32_t height;
    uint32_t format; // DRM FOURCC
    uint32_t stride; // bytes per line, plane 0
    uint32_t size;   // total buffer size in bytes
};

// Used internally by EglContext::import_dmabuf; not sent over the socket.
struct FrameMeta {
    uint32_t width;
    uint32_t height;
    uint32_t format;
    uint32_t stride;
    uint32_t size;
};

constexpr const char* kDefaultSocketPath = "/tmp/dma_buf_socket";

// RAII wrapper for a file descriptor.
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
int create_listening_socket(const std::string& path);

// Client side: connect to a Unix domain stream socket.
int connect_to_socket(const std::string& path);

// Send HandshakeMsg + all DMA-BUF fds via SCM_RIGHTS in one sendmsg().
// Returns 0 on success, -1 on failure.
int send_handshake(int sock_fd, const HandshakeMsg& msg,
                   const int* fds, int nfds);

// Receive HandshakeMsg + DMA-BUF fds. fds_out is populated with the received
// fds (caller owns them). Returns 0 on success, -1 on failure, 1 on EOF.
int recv_handshake(int sock_fd, HandshakeMsg& msg, std::vector<int>& fds_out);

// Send a buffer index (plain write, no SCM_RIGHTS).
// Returns 0 on success, -1 on failure.
int send_frame_idx(int sock_fd, uint32_t idx);

// Receive a buffer index.
// Returns 0 on success, -1 on failure, 1 on clean EOF.
int recv_frame_idx(int sock_fd, uint32_t& idx);

} // namespace dmabuf
