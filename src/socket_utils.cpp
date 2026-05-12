#include "socket_utils.hpp"

#include <cerrno>
#include <cstdio>
#include <cstring>
#include <unistd.h>
#include <sys/socket.h>
#include <sys/un.h>

namespace dmabuf {

ScopedFd::~ScopedFd() {
    if (fd_ >= 0) ::close(fd_);
}

ScopedFd& ScopedFd::operator=(ScopedFd&& other) noexcept {
    if (this != &other) {
        if (fd_ >= 0) ::close(fd_);
        fd_ = other.fd_;
        other.fd_ = -1;
    }
    return *this;
}

int ScopedFd::release() {
    int fd = fd_;
    fd_ = -1;
    return fd;
}

void ScopedFd::reset(int fd) {
    if (fd_ >= 0) ::close(fd_);
    fd_ = fd;
}

static bool fill_sockaddr(sockaddr_un& addr, const std::string& path) {
    if (path.size() + 1 > sizeof(addr.sun_path)) {
        std::fprintf(stderr, "socket path too long: %s\n", path.c_str());
        return false;
    }
    std::memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    std::memcpy(addr.sun_path, path.c_str(), path.size());
    return true;
}

int create_listening_socket(const std::string& path) {
    int s = ::socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (s < 0) { std::perror("socket"); return -1; }
    ::unlink(path.c_str());
    sockaddr_un addr{};
    if (!fill_sockaddr(addr, path)) { ::close(s); return -1; }
    if (::bind(s, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) < 0) {
        std::perror("bind"); ::close(s); return -1;
    }
    if (::listen(s, 1) < 0) {
        std::perror("listen"); ::close(s); return -1;
    }
    return s;
}

int connect_to_socket(const std::string& path) {
    int s = ::socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (s < 0) { std::perror("socket"); return -1; }
    sockaddr_un addr{};
    if (!fill_sockaddr(addr, path)) { ::close(s); return -1; }
    if (::connect(s, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) < 0) {
        std::perror("connect"); ::close(s); return -1;
    }
    return s;
}

int send_handshake(int sock_fd, const HandshakeMsg& msg,
                   const int* fds, int nfds) {
    iovec iov{};
    iov.iov_base = const_cast<HandshakeMsg*>(&msg);
    iov.iov_len  = sizeof(msg);

    const size_t cbuf_size = CMSG_SPACE(static_cast<size_t>(nfds) * sizeof(int));
    std::vector<char> cbuf(cbuf_size, 0);

    msghdr hdr{};
    hdr.msg_iov        = &iov;
    hdr.msg_iovlen     = 1;
    hdr.msg_control    = cbuf.data();
    hdr.msg_controllen = cbuf_size;

    cmsghdr* cmsg    = CMSG_FIRSTHDR(&hdr);
    cmsg->cmsg_level = SOL_SOCKET;
    cmsg->cmsg_type  = SCM_RIGHTS;
    cmsg->cmsg_len   = CMSG_LEN(static_cast<size_t>(nfds) * sizeof(int));
    std::memcpy(CMSG_DATA(cmsg), fds, static_cast<size_t>(nfds) * sizeof(int));

    ssize_t n = ::sendmsg(sock_fd, &hdr, MSG_NOSIGNAL);
    if (n < 0) { std::perror("sendmsg handshake"); return -1; }
    if (static_cast<size_t>(n) != sizeof(msg)) {
        std::fprintf(stderr, "short sendmsg handshake\n"); return -1;
    }
    return 0;
}

int recv_handshake(int sock_fd, HandshakeMsg& msg, std::vector<int>& fds_out) {
    iovec iov{};
    iov.iov_base = &msg;
    iov.iov_len  = sizeof(msg);

    constexpr int kMaxFds = 32;
    char cbuf[CMSG_SPACE(kMaxFds * sizeof(int))];
    std::memset(cbuf, 0, sizeof(cbuf));

    msghdr hdr{};
    hdr.msg_iov        = &iov;
    hdr.msg_iovlen     = 1;
    hdr.msg_control    = cbuf;
    hdr.msg_controllen = sizeof(cbuf);

    ssize_t n = ::recvmsg(sock_fd, &hdr, MSG_CMSG_CLOEXEC);
    if (n == 0) return 1;
    if (n < 0)  { std::perror("recvmsg handshake"); return -1; }
    if (static_cast<size_t>(n) != sizeof(msg)) {
        std::fprintf(stderr, "short recvmsg handshake\n"); return -1;
    }
    if (hdr.msg_flags & MSG_CTRUNC) {
        std::fprintf(stderr, "handshake: ancillary data truncated\n"); return -1;
    }

    for (cmsghdr* cm = CMSG_FIRSTHDR(&hdr); cm; cm = CMSG_NXTHDR(&hdr, cm)) {
        if (cm->cmsg_level == SOL_SOCKET && cm->cmsg_type == SCM_RIGHTS) {
            int count = static_cast<int>((cm->cmsg_len - CMSG_LEN(0)) / sizeof(int));
            fds_out.resize(count);
            std::memcpy(fds_out.data(), CMSG_DATA(cm), count * sizeof(int));
        }
    }

    if (static_cast<uint32_t>(fds_out.size()) != msg.buf_count) {
        std::fprintf(stderr, "handshake: expected %u fds, got %zu\n",
                     msg.buf_count, fds_out.size());
        for (int fd : fds_out) ::close(fd);
        return -1;
    }
    return 0;
}

int send_frame_idx(int sock_fd, uint32_t idx) {
    ssize_t n = ::send(sock_fd, &idx, sizeof(idx), MSG_NOSIGNAL);
    if (n != static_cast<ssize_t>(sizeof(idx))) { std::perror("send idx"); return -1; }
    return 0;
}

int recv_frame_idx(int sock_fd, uint32_t& idx) {
    ssize_t n = ::recv(sock_fd, &idx, sizeof(idx), MSG_WAITALL);
    if (n == 0) return 1;
    if (n < 0)  { std::perror("recv idx"); return -1; }
    if (static_cast<size_t>(n) != sizeof(idx)) return -1;
    return 0;
}

} // namespace dmabuf
