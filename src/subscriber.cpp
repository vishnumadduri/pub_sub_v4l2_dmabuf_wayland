// Subscriber: opens a Wayland xdg-toplevel window, sets up an EGL/GLES2
// context on it, then loops:
//   recvmsg(SCM_RIGHTS) -> EGLImageKHR(LINUX_DMA_BUF_EXT) ->
//   glEGLImageTargetTexture2DOES -> draw quad -> eglSwapBuffers ->
//   eglDestroyImage -> close(fd) -> send 1-byte ack
//
// The ack closes the loop with the publisher so it knows the buffer is
// safe to re-queue.

#include <atomic>
#include <chrono>
#include <csignal>
#include <cstdio>
#include <cstring>
#include <string>
#include <unistd.h>

#include "egl_utils.hpp"
#include "socket_utils.hpp"
#include "wayland_utils.hpp"

using dmabuf::EglContext;
using dmabuf::FrameMeta;
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

    // We need at least one frame's metadata to size the window. Peek the
    // first frame off the wire, then create the window with that size.
    FrameMeta first_meta{};
    int first_fd = -1;
    int r = dmabuf::recv_frame(sock.get(), first_meta, first_fd);
    if (r != 0) {
        std::fprintf(stderr, "subscriber: failed to receive first frame\n");
        return 1;
    }
    ScopedFd first_dmabuf(first_fd);

    std::printf("subscriber: first frame %ux%u format=0x%x stride=%u\n",
                first_meta.width, first_meta.height,
                first_meta.format, first_meta.stride);

    WaylandWindow win;
    if (!win.create(first_meta.width, first_meta.height, "dmabuf subscriber")) return 1;

    EglContext egl;
    if (!egl.init(win.display(), win.egl_window())) return 1;

    QuadRenderer renderer;
    if (!renderer.init(first_meta.format)) return 1;
    renderer.set_viewport(static_cast<int>(first_meta.width),
                          static_cast<int>(first_meta.height));

    auto last_log = std::chrono::steady_clock::now();
    uint32_t frames_since_log = 0;

    auto draw_one = [&](FrameMeta& meta, ScopedFd& dmabuf_fd) -> bool {
        EGLImageKHR img = egl.import_dmabuf(dmabuf_fd.get(), meta);
        if (img == EGL_NO_IMAGE_KHR) return false;

        renderer.draw(img,
                      meta.width, meta.height,
                      reinterpret_cast<PFNGLEGLIMAGETARGETTEXTURE2DOESPROC>(
                          eglGetProcAddress("glEGLImageTargetTexture2DOES")));
        egl.swap_buffers();
        egl.destroy_image(img);
        // Close our copy of the dmabuf fd: EGL took its own reference at
        // import time, the GPU read at draw time, and we no longer need it.
        dmabuf_fd.reset();

        return send_ack(sock.get());
    };

    if (!draw_one(first_meta, first_dmabuf)) return 1;
    win.dispatch_pending();
    ++frames_since_log;

    while (g_running.load() && !win.should_close()) {
        FrameMeta meta{};
        int fd = -1;
        int rr = dmabuf::recv_frame(sock.get(), meta, fd);
        if (rr == 1) { std::printf("subscriber: peer closed\n"); break; }
        if (rr < 0)  break;

        ScopedFd dmabuf_fd(fd);
        if (!draw_one(meta, dmabuf_fd)) break;

        if (!win.dispatch_pending()) break;

        ++frames_since_log;
        auto now = std::chrono::steady_clock::now();
        auto elapsed = std::chrono::duration_cast<std::chrono::milliseconds>(
                           now - last_log).count();
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
