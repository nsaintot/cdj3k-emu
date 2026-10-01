//! `SCM_RIGHTS` over the Unix-socket monitor.

use std::io::{BufReader, Write};
use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::qmp::{read_line, read_reply, QmpError};

/// Hand QEMU a file descriptor through the Unix-socket monitor at `socket`,
/// and return the fdset it joined: open it in QEMU as `/dev/fdset/<id>`.
///
/// QEMU keeps a non-empty fdset after this connection closes, until
/// [`QmpClient::remove_fd`].
pub fn add_fd(socket: &Path, fd: BorrowedFd<'_>) -> Result<u64, QmpError> {
    let stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    read_line(&mut reader)?;
    (&stream).write_all(b"{\"execute\":\"qmp_capabilities\"}\n")?;
    read_reply(&mut reader)?;
    send_with_fd(&stream, b"{\"execute\":\"add-fd\"}\n", fd.as_raw_fd())?;
    let ret = read_reply(&mut reader)?;
    ret.get("fdset-id")
        .and_then(Value::as_u64)
        .ok_or_else(|| QmpError::QemuError(format!("add-fd returned no fdset-id: {ret}")))
}

/// Write `data` with `fd` attached as SCM_RIGHTS, in one message so QEMU sees
/// the descriptor with the command that claims it.
fn send_with_fd(stream: &UnixStream, data: &[u8], fd: RawFd) -> std::io::Result<()> {
    let mut iov = libc::iovec {
        iov_base: data.as_ptr() as *mut libc::c_void,
        iov_len: data.len(),
    };
    // u64 words keep the control buffer aligned for `cmsghdr`.
    let mut control = [0u64; 8];
    let fd_len = std::mem::size_of::<RawFd>() as u32;
    // SAFETY: msghdr is plain data; every pointer set below outlives sendmsg,
    // and the control buffer is aligned and larger than CMSG_SPACE(one fd).
    let sent = unsafe {
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = libc::CMSG_SPACE(fd_len) as _;
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(fd_len) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>(), fd);
        libc::sendmsg(stream.as_raw_fd(), &msg, 0)
    };
    if sent < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut stream = stream;
    stream.write_all(&data[sent as usize..])
}
