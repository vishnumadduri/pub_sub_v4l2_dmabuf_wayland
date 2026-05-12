// Subscriber: opens a Wayland xdg-toplevel window, sets up EGL/GLES2 on it,
// then loops:
//   recvmsg(SCM_RIGHTS) → EGLImageKHR(LINUX_DMA_BUF_EXT) →
//   glEGLImageTargetTexture2DOES → draw quad → eglSwapBuffers →
//   eglDestroyImage → close(fd) → send 1-byte ack
//
// The ack closes the loop with the publisher so it knows the buffer is
// safe to re-queue.

mod wayland;
mod egl;

use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use common::socket::{connect_to_socket, recv_frame, send_ack, RecvFrame};
use common::{DEFAULT_SOCKET_PATH};
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

    // Peek the first frame to learn the image dimensions before creating
    // the window (we size the window to match the capture resolution).
    let (first_meta, first_fd) = match recv_frame(sock_raw) {
        RecvFrame::Frame(m, fd) => (m, fd),
        RecvFrame::PeerClosed   => return Err("publisher closed before first frame".into()),
        RecvFrame::Err(e)       => return Err(e.into()),
    };

    println!(
        "subscriber: first frame {}x{} format=0x{:x} stride={}",
        first_meta.width, first_meta.height, first_meta.format, first_meta.stride
    );

    let mut win = WaylandWindow::create(first_meta.width, first_meta.height, "dmabuf subscriber")?;
    let egl = EglContext::init(win.display_ptr(), win.egl_window_ptr())?;
    let mut renderer = QuadRenderer::init(first_meta.format)?;
    renderer.set_viewport(first_meta.width as i32, first_meta.height as i32);

    let image_target = egl.gl_image_target;

    // Draw a frame and send the ack back to the publisher.
    let draw_one = |meta: &common::FrameMeta, fd: i32| -> bool {
        let img = egl.import_dmabuf(fd, meta);
        if img == EGL_NO_IMAGE_KHR {
            return false;
        }
        renderer.draw(img, image_target);
        egl.swap_buffers();
        egl.destroy_image(img);
        // EGL took its own reference at import time; our fd copy is no longer needed.
        // The OwnedFd caller will close it.
        send_ack(sock_raw).is_ok()
    };

    // Draw the first frame.
    {
        let fd_raw = first_fd.as_raw_fd();
        if !draw_one(&first_meta, fd_raw) {
            return Ok(());
        }
    }
    win.dispatch_pending();

    let mut last_log       = Instant::now();
    let mut frames_since_log: u32 = 1;

    while RUNNING.load(Ordering::Relaxed) && !win.should_close() {
        let (meta, owned_fd) = match recv_frame(sock_raw) {
            RecvFrame::Frame(m, fd) => (m, fd),
            RecvFrame::PeerClosed   => { println!("subscriber: peer closed"); break; }
            RecvFrame::Err(e)       => { eprintln!("recv_frame: {e}"); break; }
        };

        if !draw_one(&meta, owned_fd.as_raw_fd()) {
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
