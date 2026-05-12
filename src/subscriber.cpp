// Subscriber: opens a Wayland xdg-toplevel window and renders DMA-BUF frames.
//
// Handshake (once):
//   recv_handshake → HandshakeMsg + N fds (kept open for the session)
//
// Per frame:
//   recv(u32 idx) → import fds[idx] → draw → eglDestroyImage → send 1-byte ack
//
// eglDestroyImageKHR must precede send_ack so EGL releases its DMA-BUF
// reference before the publisher calls QBUF.

#include <atomic>
#include <chrono>
#include <csignal>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>
#include <unistd.h>

#include "egl_utils.hpp"
#include "socket_utils.hpp"
#include "wayland_utils.hpp"

using dmabuf::EglContext;
using dmabuf::FrameMeta;
using dmabuf::HandshakeMsg;
using dmabuf::QuadRenderer;
using dmabuf::ScopedFd;
using dmabuf::WaylandWindow;

namespace {

std::atomic<bool> g_running{true};
void on_sigint(int) { g_running.store(false); }

bool send_ack(int sock_fd) {
    char ack = 1;
    if (::send(sock_fd, &ack, 1, MSG_NOSIGNAL) != 1) {
        std::perror("send ack");
        return false;
    }
    return true;
}

} // namespace

int main(int argc, char** argv) {
    std::string socket_path = dmabuf::kDefaultSocketPath;
    for (int i = 1; i < argc; ++i) {
        std::string a = argv[i];
        if (a == "--socket" && i + 1 < argc) socket_path = argv[++i];
        else {
            std::fprintf(stderr, "usage: %s [--socket PATH]\n", argv[0]);
            return 2;
        }
    }

    std::signal(SIGINT,  on_sigint);
    std::signal(SIGTERM, on_sigint);
    std::signal(SIGPIPE, SIG_IGN);

    ScopedFd sock(dmabuf::connect_to_socket(socket_path));
    if (!sock) {
        std::fprintf(stderr, "subscriber: could not connect to %s\n", socket_path.c_str());
        return 1;
    }

    // Receive geometry and all DMA-BUF fds in one message.
    HandshakeMsg hs{};
    std::vector<int> raw_fds;
    if (dmabuf::recv_handshake(sock.get(), hs, raw_fds) != 0) {
        std::fprintf(stderr, "subscriber: handshake failed\n");
        return 1;
    }

    // Wrap the received fds in ScopedFd so they close on exit.
    std::vector<ScopedFd> session_fds;
    session_fds.reserve(raw_fds.size());
    for (int fd : raw_fds) session_fds.emplace_back(fd);

    std::printf("subscriber: %ux%u format=0x%x stride=%u bufs=%u\n",
                hs.width, hs.height, hs.format, hs.stride, hs.buf_count);

    // FrameMeta is constant for the session; reconstructed from the handshake.
    FrameMeta frame_meta{};
    frame_meta.width  = hs.width;
    frame_meta.height = hs.height;
    frame_meta.format = hs.format;
    frame_meta.stride = hs.stride;
    frame_meta.size   = hs.size;

    WaylandWindow win;
    if (!win.create(hs.width, hs.height, "dmabuf subscriber")) return 1;

    EglContext egl;
    if (!egl.init(win.display(), win.egl_window())) return 1;

    QuadRenderer renderer;
    if (!renderer.init(hs.format)) return 1;
    renderer.set_viewport(static_cast<int>(hs.width), static_cast<int>(hs.height));

    auto last_log = std::chrono::steady_clock::now();
    uint32_t frames_since_log = 0;

    // Import buffer[idx], draw, destroy EGLImage, then ack.
    // session_fds are kept open; no per-frame close needed.
    auto draw_one = [&](uint32_t idx) -> bool {
        EGLImageKHR img = egl.import_dmabuf(session_fds[idx].get(), frame_meta);
        if (img == EGL_NO_IMAGE_KHR) return false;

        renderer.draw(img,
                      frame_meta.width, frame_meta.height,
                      reinterpret_cast<PFNGLEGLIMAGETARGETTEXTURE2DOESPROC>(
                          eglGetProcAddress("glEGLImageTargetTexture2DOES")));
        egl.swap_buffers();
        egl.destroy_image(img);
        return send_ack(sock.get());
    };

    while (g_running.load() && !win.should_close()) {
        uint32_t idx = 0;
        int r = dmabuf::recv_frame_idx(sock.get(), idx);
        if (r == 1) { std::printf("subscriber: peer closed\n"); break; }
        if (r < 0)  break;

        if (!draw_one(idx)) break;
        if (!win.dispatch_pending()) break;

        ++frames_since_log;
        auto now     = std::chrono::steady_clock::now();
        auto elapsed = std::chrono::duration_cast<std::chrono::milliseconds>(now - last_log).count();
        if (elapsed >= 1000) {
            std::printf("subscriber: %.1f fps\n",
                        frames_since_log * 1000.0 / elapsed);
            std::fflush(stdout);
            frames_since_log = 0;
            last_log = now;
        }
    }

    std::printf("subscriber: shutting down\n");
    return 0;
}
