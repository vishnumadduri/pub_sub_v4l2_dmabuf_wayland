// Subscriber: opens a Wayland xdg-toplevel window and renders DMA-BUF frames.
//
// Handshake (once):
//   recv_handshake → Handshake struct + N OwnedFds (held for the session)
//
// Per frame:
//   recv(u32 idx) → import fds[idx] → draw → eglDestroyImage → send 1-byte ack
//
// eglDestroyImageKHR must precede send_ack so EGL releases its DMA-BUF
// reference before the publisher calls QBUF.

mod wayland;
mod egl;

use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use common::socket::{connect_to_socket, recv_handshake, recv_idx, send_ack, RecvIdx};
use common::{FrameMeta, DEFAULT_SOCKET_PATH};
use egl::{EglContext, EGL_NO_IMAGE_KHR, QuadRenderer};
use wayland::WaylandWindow;

static RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn sighandler(_: libc::c_int) {
    RUNNING.store(false, Ordering::SeqCst);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut socket_path = DEFAULT_SOCKET_PATH.to_string();

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => socket_path = args.next().ok_or("missing value for --socket")?.into(),
            _ => {
                eprintln!("usage: subscriber [--socket PATH]");
                std::process::exit(2);
            }
        }
    }

    unsafe {
        libc::signal(libc::SIGINT,  sighandler as libc::sighandler_t);
        libc::signal(libc::SIGTERM, sighandler as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let sock = connect_to_socket(&socket_path)?;
    let sock_raw = sock.as_raw_fd();

    // Receive geometry and all DMA-BUF fds in one message.
    let (hs, buf_fds) = recv_handshake(sock_raw)?;
    println!(
        "subscriber: {}x{} format=0x{:x} stride={} bufs={}",
        hs.width, hs.height, hs.format, hs.stride, hs.buf_count
    );

    // FrameMeta is constant for the session; reconstructed from the handshake.
    let frame_meta = FrameMeta {
        width:  hs.width,
        height: hs.height,
        format: hs.format,
        stride: hs.stride,
        size:   hs.size,
    };

    let mut win = WaylandWindow::create(hs.width, hs.height, "dmabuf subscriber")?;
    let egl = EglContext::init(win.display_ptr(), win.egl_window_ptr())?;
    let mut renderer = QuadRenderer::init(hs.format)?;
    renderer.set_viewport(hs.width as i32, hs.height as i32);

    let image_target = egl.gl_image_target;

    // Import buffer[idx], draw, destroy the EGLImage, then ack.
    // buf_fds are kept open for the session; no per-frame close needed.
    let draw_one = |idx: u32| -> bool {
        let fd = buf_fds[idx as usize].as_raw_fd();
        let img = egl.import_dmabuf(fd, &frame_meta);
        if img == EGL_NO_IMAGE_KHR {
            return false;
        }
        renderer.draw(img, image_target);
        egl.swap_buffers();
        egl.destroy_image(img);
        send_ack(sock_raw).is_ok()
    };

    let mut last_log         = Instant::now();
    let mut frames_since_log: u32 = 0;

    while RUNNING.load(Ordering::Relaxed) && !win.should_close() {
        let idx = match recv_idx(sock_raw) {
            RecvIdx::Idx(i)  => i,
            RecvIdx::PeerClosed => { println!("subscriber: peer closed"); break; }
            RecvIdx::Err(e)     => { eprintln!("recv_idx: {e}"); break; }
        };

        if !draw_one(idx) {
            break;
        }

        if !win.dispatch_pending() {
            break;
        }

        frames_since_log += 1;
        let elapsed = last_log.elapsed();
        if elapsed.as_millis() >= 1000 {
            let fps = frames_since_log as f64 * 1000.0 / elapsed.as_millis() as f64;
            println!("subscriber: {fps:.1} fps");
            frames_since_log = 0;
            last_log = Instant::now();
        }
    }

    println!("subscriber: shutting down");
    Ok(())
}
