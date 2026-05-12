use std::io;
use std::mem::size_of;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use nix::sys::socket::{
    accept4, bind, connect, listen, recvmsg, sendmsg, socket, Backlog, ControlMessage,
    ControlMessageOwned, MsgFlags, SockFlag, SockType, UnixAddr,
};
use nix::sys::socket::AddressFamily;

use crate::Handshake;

fn to_io(e: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(e as i32)
}

pub fn create_listening_socket(path: &str) -> io::Result<OwnedFd> {
    let _ = std::fs::remove_file(path);
    let fd = socket(AddressFamily::Unix, SockType::Stream, SockFlag::SOCK_CLOEXEC, None)
        .map_err(to_io)?;
    let addr = UnixAddr::new(path).map_err(to_io)?;
    bind(fd.as_raw_fd(), &addr).map_err(to_io)?;
    listen(&fd, Backlog::new(1).unwrap()).map_err(to_io)?;
    Ok(fd)
}

pub fn accept_connection(listen_fd: &OwnedFd) -> io::Result<OwnedFd> {
    let raw = accept4(listen_fd.as_raw_fd(), SockFlag::SOCK_CLOEXEC).map_err(to_io)?;
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

pub fn connect_to_socket(path: &str) -> io::Result<OwnedFd> {
    let fd = socket(AddressFamily::Unix, SockType::Stream, SockFlag::SOCK_CLOEXEC, None)
        .map_err(to_io)?;
    let addr = UnixAddr::new(path).map_err(to_io)?;
    connect(fd.as_raw_fd(), &addr).map_err(to_io)?;
    Ok(fd)
}

/// Send the handshake message + all DMA-BUF fds in one sendmsg call.
pub fn send_handshake(sock_fd: RawFd, hs: &Handshake, fds: &[RawFd]) -> io::Result<()> {
    let bytes = unsafe {
        std::slice::from_raw_parts((hs as *const Handshake).cast::<u8>(), size_of::<Handshake>())
    };
    let iov  = [io::IoSlice::new(bytes)];
    let cmsg = [ControlMessage::ScmRights(fds)];
    let n = sendmsg::<()>(sock_fd, &iov, &cmsg, MsgFlags::MSG_NOSIGNAL, None).map_err(to_io)?;
    if n != size_of::<Handshake>() {
        return Err(io::Error::new(io::ErrorKind::Other, "short sendmsg handshake"));
    }
    Ok(())
}

/// Receive the handshake message + all DMA-BUF fds.
pub fn recv_handshake(sock_fd: RawFd) -> io::Result<(Handshake, Vec<OwnedFd>)> {
    let mut hs = Handshake::default();
    let bytes = unsafe {
        std::slice::from_raw_parts_mut((&raw mut hs).cast::<u8>(), size_of::<Handshake>())
    };
    let mut iov = [io::IoSliceMut::new(bytes)];
    // Space for up to 32 fds in the ancillary buffer.
    let cmsg_size = unsafe {
        libc::CMSG_SPACE((32 * size_of::<RawFd>()) as u32) as usize
    };
    let mut cmsg_buf = vec![0u8; cmsg_size];

    let msg = recvmsg::<()>(sock_fd, &mut iov, Some(&mut cmsg_buf), MsgFlags::MSG_CMSG_CLOEXEC)
        .map_err(to_io)?;

    if msg.bytes != size_of::<Handshake>() {
        return Err(io::Error::new(io::ErrorKind::Other, "short recvmsg handshake"));
    }

    let mut owned_fds: Vec<OwnedFd> = Vec::new();
    if let Ok(cmsgs) = msg.cmsgs() {
        for cmsg in cmsgs {
            if let ControlMessageOwned::ScmRights(fds) = cmsg {
                for fd in fds {
                    owned_fds.push(unsafe { OwnedFd::from_raw_fd(fd) });
                }
            }
        }
    }

    if owned_fds.len() != hs.buf_count as usize {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("handshake: expected {} fds, got {}", hs.buf_count, owned_fds.len()),
        ));
    }
    Ok((hs, owned_fds))
}

/// Send a buffer index (plain write, no SCM_RIGHTS).
pub fn send_idx(sock_fd: RawFd, idx: u32) -> io::Result<()> {
    let bytes = idx.to_ne_bytes();
    let iov = [io::IoSlice::new(&bytes)];
    let n = sendmsg::<()>(sock_fd, &iov, &[], MsgFlags::MSG_NOSIGNAL, None).map_err(to_io)?;
    if n != 4 {
        return Err(io::Error::new(io::ErrorKind::Other, "short send idx"));
    }
    Ok(())
}

pub enum RecvIdx {
    Idx(u32),
    PeerClosed,
    Err(io::Error),
}

/// Receive a buffer index.
pub fn recv_idx(sock_fd: RawFd) -> RecvIdx {
    let mut buf = [0u8; 4];
    let n = unsafe { libc::recv(sock_fd, buf.as_mut_ptr().cast(), 4, libc::MSG_WAITALL) };
    if n == 0  { return RecvIdx::PeerClosed; }
    if n < 0   { return RecvIdx::Err(io::Error::last_os_error()); }
    if n != 4  { return RecvIdx::Err(io::Error::new(io::ErrorKind::Other, "short recv idx")); }
    RecvIdx::Idx(u32::from_ne_bytes(buf))
}

/// Send a single-byte ACK.
pub fn send_ack(sock_fd: RawFd) -> io::Result<()> {
    let buf = [1u8];
    let iov = [io::IoSlice::new(&buf)];
    sendmsg::<()>(sock_fd, &iov, &[], MsgFlags::MSG_NOSIGNAL, None).map_err(to_io)?;
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
