// Publisher: captures frames from a V4L2 device and streams them to a
// subscriber over a Unix domain socket.
//
// Handshake (once):
//   sendmsg(SCM_RIGHTS, all N dmabuf fds) + Handshake struct
//
// Per frame:
//   DQBUF → send(u32 idx) → wait 1-byte ack → QBUF

use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use common::socket::{accept_connection, create_listening_socket, send_handshake, send_idx, wait_for_ack};
use common::v4l2::{v4l2_to_drm_fourcc, V4l2Capture, V4L2_PIX_FMT_YUYV, V4L2_PIX_FMT_NV12};
use common::{Handshake, DEFAULT_SOCKET_PATH};

static RUNNING: AtomicBool = AtomicBool::new(true);

extern "C" fn sighandler(_: libc::c_int) {
    RUNNING.store(false, Ordering::SeqCst);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut device       = "/dev/video0".to_string();
    let mut width: u32   = 640;
    let mut height: u32  = 480;
    let mut pixel_fmt    = V4L2_PIX_FMT_YUYV;
    let mut socket_path  = DEFAULT_SOCKET_PATH.to_string();

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--device"  => device      = args.next().ok_or("missing value for --device")?.into(),
            "--width"   => width       = args.next().ok_or("missing value for --width")?.parse()?,
            "--height"  => height      = args.next().ok_or("missing value for --height")?.parse()?,
            "--socket"  => socket_path = args.next().ok_or("missing value for --socket")?.into(),
            "--yuyv"    => pixel_fmt   = V4L2_PIX_FMT_YUYV,
            "--nv12"    => pixel_fmt   = V4L2_PIX_FMT_NV12,
            _ => {
                eprintln!("usage: publisher [--device /dev/videoN] [--width W] [--height H] \
                           [--socket PATH] [--yuyv|--nv12]");
                std::process::exit(2);
            }
        }
    }

    unsafe {
        libc::signal(libc::SIGINT,  sighandler as libc::sighandler_t);
        libc::signal(libc::SIGTERM, sighandler as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    RUNNING.store(true, Ordering::SeqCst);

    let mut cap = V4l2Capture::open(&device, width, height, pixel_fmt)?;
    cap.start(4)?;

    let drm_fourcc = v4l2_to_drm_fourcc(cap.pixel_format)
        .ok_or_else(|| format!("no DRM FOURCC for V4L2 format 0x{:x}", cap.pixel_format))?;

    let listen_fd = create_listening_socket(&socket_path)?;
    println!("publisher: waiting for subscriber on {socket_path}");

    let client = accept_connection(&listen_fd)?;
    let client_raw = client.as_raw_fd();
    println!("publisher: subscriber connected");

    // Send all DMA-BUF fds once; the subscriber identifies buffers by index.
    let all_fds: Vec<i32> = cap.buffers.iter().map(|b| b.dmabuf_fd.as_raw_fd()).collect();
    let hs = Handshake {
        buf_count: all_fds.len() as u32,
        width:     cap.width,
        height:    cap.height,
        format:    drm_fourcc,
        stride:    cap.stride,
        size:      cap.size_image,
    };
    send_handshake(client_raw, &hs, &all_fds)?;

    let mut last_log = Instant::now();
    let mut frames_since_log: u32 = 0;
    let mut sequence: u32 = 0;

    while RUNNING.load(Ordering::Relaxed) {
        let idx = match cap.dequeue() {
            Ok(i)  => i,
            Err(e) => { eprintln!("dequeue: {e}"); break; }
        };

        if let Err(e) = send_idx(client_raw, idx) {
            eprintln!("send_idx: {e}");
            let _ = cap.requeue(idx);
            break;
        }
        if let Err(e) = wait_for_ack(client_raw) {
            eprintln!("wait_for_ack: {e}");
            let _ = cap.requeue(idx);
            break;
        }
        if let Err(e) = cap.requeue(idx) {
            eprintln!("requeue: {e}");
            break;
        }

        frames_since_log += 1;
        sequence += 1;
        let elapsed = last_log.elapsed();
        if elapsed.as_millis() >= 1000 {
            let fps = frames_since_log as f64 * 1000.0 / elapsed.as_millis() as f64;
            println!("publisher: {fps:.1} fps ({sequence} frames)");
            frames_since_log = 0;
            last_log = Instant::now();
        }
    }

    println!("publisher: shutting down");
    let _ = std::fs::remove_file(&socket_path);
    Ok(())
}
