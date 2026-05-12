// Publisher: captures frames from a V4L2 device and streams them to a
// subscriber over a Unix domain socket.
//
// Handshake (once):
//   sendmsg(SCM_RIGHTS, all N dmabuf fds) + HandshakeMsg
//
// Per frame:
//   DQBUF → send(u32 idx) → wait 1-byte ack → QBUF

#include <atomic>
#include <chrono>
#include <csignal>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>
#include <unistd.h>
#include <linux/videodev2.h>

#include "socket_utils.hpp"
#include "v4l2_utils.hpp"

using dmabuf::HandshakeMsg;
using dmabuf::ScopedFd;
using dmabuf::V4l2Capture;

namespace {

std::atomic<bool> g_running{true};
void on_sigint(int) { g_running.store(false); }

bool wait_for_ack(int sock_fd) {
    char ack = 0;
    ssize_t n = ::recv(sock_fd, &ack, 1, 0);
    if (n == 0) { std::fprintf(stderr, "wait_for_ack: subscriber disconnected\n"); return false; }
    if (n < 0)  { std::perror("wait_for_ack"); return false; }
    return true;
}

} // namespace

int main(int argc, char** argv) {
    std::string device    = "/dev/video0";
    uint32_t    width     = 640;
    uint32_t    height    = 480;
    uint32_t    v4l2_fmt  = V4L2_PIX_FMT_YUYV;
    std::string sock_path = dmabuf::kDefaultSocketPath;

    for (int i = 1; i < argc; ++i) {
        std::string a = argv[i];
        if      (a == "--device" && i+1 < argc) device   = argv[++i];
        else if (a == "--width"  && i+1 < argc) width    = static_cast<uint32_t>(std::stoi(argv[++i]));
        else if (a == "--height" && i+1 < argc) height   = static_cast<uint32_t>(std::stoi(argv[++i]));
        else if (a == "--socket" && i+1 < argc) sock_path= argv[++i];
        else if (a == "--yuyv")                 v4l2_fmt = V4L2_PIX_FMT_YUYV;
        else if (a == "--nv12")                 v4l2_fmt = V4L2_PIX_FMT_NV12;
        else {
            std::fprintf(stderr,
                "usage: %s [--device PATH] [--width W] [--height H] "
                "[--socket PATH] [--yuyv|--nv12]\n", argv[0]);
            return 2;
        }
    }

    std::signal(SIGINT,  on_sigint);
    std::signal(SIGTERM, on_sigint);
    std::signal(SIGPIPE, SIG_IGN);

    V4l2Capture cap;
    if (!cap.open(device, width, height, v4l2_fmt)) return 1;
    if (!cap.start(4)) return 1;

    uint32_t drm_fourcc = dmabuf::v4l2_to_drm_fourcc(cap.pixel_format());
    if (!drm_fourcc) {
        std::fprintf(stderr, "no DRM FOURCC for V4L2 format 0x%x\n", cap.pixel_format());
        return 1;
    }

    ScopedFd listen_fd(dmabuf::create_listening_socket(sock_path));
    if (!listen_fd) return 1;
    std::printf("publisher: waiting for subscriber on %s\n", sock_path.c_str());

    int client_raw = ::accept4(listen_fd.get(), nullptr, nullptr, SOCK_CLOEXEC);
    if (client_raw < 0) { std::perror("accept"); return 1; }
    ScopedFd client(client_raw);
    std::printf("publisher: subscriber connected\n");

    // Send geometry + all DMA-BUF fds once; per-frame messages carry only idx.
    const auto& bufs = cap.buffers();
    std::vector<int> all_fds;
    all_fds.reserve(bufs.size());
    for (const auto& b : bufs) all_fds.push_back(b.dmabuf_fd);

    HandshakeMsg hs{};
    hs.buf_count = static_cast<uint32_t>(all_fds.size());
    hs.width     = cap.width();
    hs.height    = cap.height();
    hs.format    = drm_fourcc;
    hs.stride    = cap.stride();
    hs.size      = cap.size_image();

    if (dmabuf::send_handshake(client.get(), hs, all_fds.data(),
                               static_cast<int>(all_fds.size())) < 0) return 1;

    auto last_log = std::chrono::steady_clock::now();
    uint32_t frames_since_log = 0;
    uint32_t sequence = 0;

    while (g_running.load()) {
        uint32_t idx = 0;
        if (!cap.dequeue(idx)) break;

        if (dmabuf::send_frame_idx(client.get(), idx) < 0) {
            cap.requeue(idx); break;
        }
        if (!wait_for_ack(client.get())) {
            cap.requeue(idx); break;
        }
        if (!cap.requeue(idx)) break;

        ++frames_since_log;
        ++sequence;
        auto now     = std::chrono::steady_clock::now();
        auto elapsed = std::chrono::duration_cast<std::chrono::milliseconds>(now - last_log).count();
        if (elapsed >= 1000) {
            std::printf("publisher: %.1f fps (%u frames)\n",
                        frames_since_log * 1000.0 / elapsed, sequence);
            std::fflush(stdout);
            frames_since_log = 0;
            last_log = now;
        }
    }

    std::printf("publisher: shutting down\n");
    ::unlink(sock_path.c_str());
    return 0;
}
