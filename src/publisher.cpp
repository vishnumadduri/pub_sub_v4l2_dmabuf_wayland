// Publisher: captures frames from /dev/video0 via V4L2 (MMAP), exports each
// buffer as a DMA-BUF, and forwards the file descriptor to a single
// subscriber over a Unix domain socket.
//
// Flow per frame:
//   DQBUF -> sendmsg(SCM_RIGHTS, fd) -> wait for 1-byte ack -> QBUF
//
// The ack handshake is what keeps the producer/consumer synchronised: we
// only re-queue a buffer once the subscriber is done sampling from it.

#include <atomic>
#include <chrono>
#include <csignal>
#include <cstdio>
#include <cstring>
#include <string>
#include <unistd.h>
#include <linux/videodev2.h>

#include "socket_utils.hpp"
#include "v4l2_utils.hpp"

using dmabuf::FrameMeta;
using dmabuf::ScopedFd;
using dmabuf::V4l2Capture;

namespace {

std::atomic<bool> g_running{true};

void on_sigint(int) {
    g_running.store(false);
}

// Block until the subscriber has sent a single ACK byte back. This forms
// our flow control: we never re-queue a buffer that the GPU might still
// be sampling on the other side.
bool wait_for_ack(int sock_fd) {
    char ack = 0;
    ssize_t n = ::recv(sock_fd, &ack, 1, 0);
    if (n <= 0) {
        if (n == 0) std::fprintf(stderr, "subscriber disconnected\n");
        else        std::perror("recv ack");
        return false;
    }
    return true;
}

} // namespace

int main(int argc, char** argv) {
    // Default capture parameters; override via CLI as needed. Start with YUYV
    // because every USB UVC webcam supports it natively.
    std::string device = "/dev/video0";
    uint32_t width = 640;
    uint32_t height = 480;
    uint32_t v4l2_format = V4L2_PIX_FMT_YUYV;
    std::string socket_path = dmabuf::kDefaultSocketPath;

    for (int i = 1; i < argc; ++i) {
        std::string a = argv[i];
        auto next = [&](const std::string& flag) {
            if (i + 1 >= argc) {
                std::fprintf(stderr, "missing value for %s\n", flag.c_str());
                std::exit(2);
            }
            return std::string(argv[++i]);
        };
        if (a == "--device")        device = next(a);
        else if (a == "--width")    width = std::stoi(next(a));
        else if (a == "--height")   height = std::stoi(next(a));
        else if (a == "--socket")   socket_path = next(a);
        else if (a == "--nv12")     v4l2_format = V4L2_PIX_FMT_NV12;
        else if (a == "--yuyv")     v4l2_format = V4L2_PIX_FMT_YUYV;
        else {
            std::fprintf(stderr,
                "usage: %s [--device /dev/videoN] [--width W] [--height H]"
                " [--socket PATH] [--yuyv|--nv12]\n", argv[0]);
            return 2;
        }
    }

    std::signal(SIGINT,  on_sigint);
    std::signal(SIGTERM, on_sigint);
    std::signal(SIGPIPE, SIG_IGN); // we already pass MSG_NOSIGNAL but be safe

    V4l2Capture cap;
    if (!cap.open(device, width, height, v4l2_format)) return 1;
    if (!cap.start(4)) return 1;

    uint32_t drm_fourcc = dmabuf::v4l2_to_drm_fourcc(cap.pixel_format());
    if (!drm_fourcc) {
        std::fprintf(stderr, "no DRM FOURCC mapping for V4L2 format 0x%x\n",
                     cap.pixel_format());
        return 1;
    }

    ScopedFd listen_fd(dmabuf::create_listening_socket(socket_path));
    if (!listen_fd) return 1;

    std::printf("publisher: waiting for subscriber on %s\n", socket_path.c_str());
    int client_raw = ::accept4(listen_fd.get(), nullptr, nullptr, SOCK_CLOEXEC);
    if (client_raw < 0) {
        std::perror("accept");
        return 1;
    }
    ScopedFd client(client_raw);
    std::printf("publisher: subscriber connected\n");

    auto last_log = std::chrono::steady_clock::now();
    uint32_t frames_since_log = 0;
    uint32_t sequence = 0;

    while (g_running.load()) {
        uint32_t idx = 0;
        if (!cap.dequeue(idx)) break;

        FrameMeta meta{};
        meta.width = cap.width();
        meta.height = cap.height();
        meta.format = drm_fourcc;
        meta.stride = cap.stride();
        meta.size = cap.size_image();
        meta.sequence = sequence++;

        int dmabuf_fd = cap.buffers()[idx].dmabuf_fd;
        if (dmabuf::send_frame(client.get(), meta, dmabuf_fd) < 0) {
            cap.requeue(idx);
            break;
        }
        if (!wait_for_ack(client.get())) {
            cap.requeue(idx);
            break;
        }
        if (!cap.requeue(idx)) break;

        ++frames_since_log;
        auto now = std::chrono::steady_clock::now();
        auto elapsed = std::chrono::duration_cast<std::chrono::milliseconds>(
                           now - last_log).count();
        if (elapsed >= 1000) {
            std::printf("publisher: %.1f fps (%u frames)\n",
                        frames_since_log * 1000.0 / elapsed,
                        meta.sequence);
            std::fflush(stdout);
            frames_since_log = 0;
            last_log = now;
        }
    }

    std::printf("publisher: shutting down\n");
    ::unlink(socket_path.c_str());
    return 0;
}
