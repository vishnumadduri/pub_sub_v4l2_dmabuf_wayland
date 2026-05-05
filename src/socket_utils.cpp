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
    if (s < 0) {
        std::perror("socket");
        return -1;
    }

    // Stale socket files from a prior run will make bind() fail with EADDRINUSE.
    ::unlink(path.c_str());

    sockaddr_un addr{};
    if (!fill_sockaddr(addr, path)) {
        ::close(s);
        return -1;
    }

    if (::bind(s, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) < 0) {
        std::perror("bind");
        ::close(s);
        return -1;
    }

    if (::listen(s, 1) < 0) {
        std::perror("listen");
        ::close(s);
        return -1;
    }
    return s;
}

int connect_to_socket(const std::string& path) {
    int s = ::socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (s < 0) {
        std::perror("socket");
        return -1;
    }

    sockaddr_un addr{};
    if (!fill_sockaddr(addr, path)) {
        ::close(s);
        return -1;
    }

    if (::connect(s, reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) < 0) {
        std::perror("connect");
        ::close(s);
        return -1;
    }
    return s;
}

int send_frame(int sock_fd, const FrameMeta& meta, int dmabuf_fd) {
    // Pack the FrameMeta as the regular payload and the dmabuf fd as ancillary
    // data using SCM_RIGHTS. We must send at least one byte for the kernel to
    // deliver the cmsg, but here we send the whole struct in one go.
    iovec iov{};
    iov.iov_base = const_cast<FrameMeta*>(&meta);
    iov.iov_len = sizeof(meta);

    char cbuf[CMSG_SPACE(sizeof(int))];
    std::memset(cbuf, 0, sizeof(cbuf));

    msghdr msg{};
    msg.msg_iov = &iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cbuf;
    msg.msg_controllen = sizeof(cbuf);

    cmsghdr* cmsg = CMSG_FIRSTHDR(&msg);
    cmsg->cmsg_level = SOL_SOCKET;
    cmsg->cmsg_type = SCM_RIGHTS;
    cmsg->cmsg_len = CMSG_LEN(sizeof(int));
    std::memcpy(CMSG_DATA(cmsg), &dmabuf_fd, sizeof(int));

    ssize_t n = ::sendmsg(sock_fd, &msg, MSG_NOSIGNAL);
    if (n < 0) {
        std::perror("sendmsg");
        return -1;
    }
    if (static_cast<size_t>(n) != sizeof(meta)) {
        std::fprintf(stderr, "short sendmsg: %zd of %zu\n", n, sizeof(meta));
        return -1;
    }
    return 0;
}

int recv_frame(int sock_fd, FrameMeta& meta, int& dmabuf_fd) {
    dmabuf_fd = -1;

    iovec iov{};
    iov.iov_base = &meta;
    iov.iov_len = sizeof(meta);

    char cbuf[CMSG_SPACE(sizeof(int))];
    std::memset(cbuf, 0, sizeof(cbuf));

    msghdr msg{};
    msg.msg_iov = &iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cbuf;
    msg.msg_controllen = sizeof(cbuf);

    ssize_t n = ::recvmsg(sock_fd, &msg, MSG_CMSG_CLOEXEC);
    if (n == 0) return 1; // peer closed cleanly
    if (n < 0) {
        std::perror("recvmsg");
        return -1;
    }
    if (static_cast<size_t>(n) != sizeof(meta)) {
        std::fprintf(stderr, "short recvmsg: %zd of %zu\n", n, sizeof(meta));
        return -1;
    }
    if (msg.msg_flags & MSG_CTRUNC) {
        std::fprintf(stderr, "ancillary data was truncated\n");
        return -1;
    }

    for (cmsghdr* cmsg = CMSG_FIRSTHDR(&msg); cmsg; cmsg = CMSG_NXTHDR(&msg, cmsg)) {
        if (cmsg->cmsg_level == SOL_SOCKET && cmsg->cmsg_type == SCM_RIGHTS) {
            std::memcpy(&dmabuf_fd, CMSG_DATA(cmsg), sizeof(int));
            break;
        }
    }
    if (dmabuf_fd < 0) {
        std::fprintf(stderr, "recvmsg: missing SCM_RIGHTS\n");
        return -1;
    }
    return 0;
}

} // namespace dmabuf
