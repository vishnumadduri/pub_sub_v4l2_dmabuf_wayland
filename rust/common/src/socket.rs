use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use nix::sys::socket::{
    accept4, bind, connect, listen, recvmsg, sendmsg, socket, Backlog, ControlMessage,
    ControlMessageOwned, MsgFlags, SockFlag, SockType, UnixAddr,
};
use nix::sys::socket::AddressFamily;

use crate::FrameMeta;

fn to_io(e: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(e as i32)
}

pub fn create_listening_socket(path: &str) -> io::Result<OwnedFd> {
    // Remove stale socket file from a prior run.
    let _ = std::fs::remove_file(path);

    let fd = socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(to_io)?;

    let addr = UnixAddr::new(path).map_err(to_io)?;
    bind(fd.as_raw_fd(), &addr).map_err(to_io)?;
    listen(&fd, Backlog::new(1).unwrap()).map_err(to_io)?;
    Ok(fd)
}

pub fn accept_connection(listen_fd: &OwnedFd) -> io::Result<OwnedFd> {
    let raw = accept4(
        listen_fd.as_raw_fd(),
        SockFlag::SOCK_CLOEXEC,
    )
    .map_err(to_io)?;
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

pub fn connect_to_socket(path: &str) -> io::Result<OwnedFd> {
    let fd = socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(to_io)?;

    let addr = UnixAddr::new(path).map_err(to_io)?;
    connect(fd.as_raw_fd(), &addr).map_err(to_io)?;
    Ok(fd)
}

/// Send FrameMeta + a DMA-BUF fd via a single sendmsg with SCM_RIGHTS.
pub fn send_frame(sock_fd: RawFd, meta: &FrameMeta, dmabuf_fd: RawFd) -> io::Result<()> {
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (meta as *const FrameMeta).cast::<u8>(),
            size_of::<FrameMeta>(),
        )
    };
    let iov = [io::IoSlice::new(bytes)];
    let fds = [dmabuf_fd];
    let cmsg = [ControlMessage::ScmRights(&fds)];

    let n = sendmsg::<()>(sock_fd, &iov, &cmsg, MsgFlags::MSG_NOSIGNAL, None)
        .map_err(to_io)?;

    if n != size_of::<FrameMeta>() {
        return Err(io::Error::new(io::ErrorKind::Other, "short sendmsg"));
    }
    Ok(())
}

pub enum RecvFrame {
    Frame(FrameMeta, OwnedFd),
    PeerClosed,
    Err(io::Error),
}

/// Receive a FrameMeta + a single DMA-BUF fd from SCM_RIGHTS ancillary data.
pub fn recv_frame(sock_fd: RawFd) -> RecvFrame {
    let mut meta = FrameMeta::default();
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(
            (&raw mut meta).cast::<u8>(),
            size_of::<FrameMeta>(),
        )
    };
    let mut iov = [io::IoSliceMut::new(bytes)];
    let mut cmsg_buf = nix::cmsg_space!(RawFd);

    let msg = match recvmsg::<()>(
        sock_fd,
        &mut iov,
        Some(&mut cmsg_buf),
        MsgFlags::MSG_CMSG_CLOEXEC,
    ) {
        Ok(m) => m,
        Err(e) => return RecvFrame::Err(to_io(e)),
    };

    if msg.bytes == 0 {
        return RecvFrame::PeerClosed;
    }
    if msg.bytes != size_of::<FrameMeta>() {
        return RecvFrame::Err(io::Error::new(io::ErrorKind::Other, "short recvmsg"));
    }

    let mut dmabuf_fd: Option<OwnedFd> = None;
    if let Ok(cmsgs) = msg.cmsgs() {
        for cmsg in cmsgs {
            if let ControlMessageOwned::ScmRights(fds) = cmsg {
                if let Some(&fd) = fds.first() {
                    dmabuf_fd = Some(unsafe { OwnedFd::from_raw_fd(fd) });
                }
            }
        }
    }

    match dmabuf_fd {
        Some(fd) => RecvFrame::Frame(meta, fd),
        None => RecvFrame::Err(io::Error::new(io::ErrorKind::Other, "missing SCM_RIGHTS fd")),
    }
}

/// Send a single-byte ACK.
pub fn send_ack(sock_fd: RawFd) -> io::Result<()> {
    let buf = [1u8];
    let iov = [io::IoSlice::new(&buf)];
    sendmsg::<()>(sock_fd, &iov, &[], MsgFlags::MSG_NOSIGNAL, None)
        .map_err(to_io)?;
    Ok(())
}

/// Block until a single ACK byte arrives.
pub fn wait_for_ack(sock_fd: RawFd) -> io::Result<()> {
    let mut buf = [0u8; 1];
    let n = unsafe { libc::recv(sock_fd, buf.as_mut_ptr().cast(), 1, 0) };
    if n == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "subscriber disconnected"));
    }
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
