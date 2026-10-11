use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::Value;

mod fd_passing;
pub use fd_passing::add_fd;

pub struct QmpClient {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

#[derive(Debug)]
pub enum QmpError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Timeout,
    QemuError(String),
}

impl From<std::io::Error> for QmpError {
    fn from(e: std::io::Error) -> Self {
        QmpError::Io(e)
    }
}
impl From<serde_json::Error> for QmpError {
    fn from(e: serde_json::Error) -> Self {
        QmpError::Json(e)
    }
}

impl QmpClient {
    /// The monitor connection, for a holder that must reach QEMU when this
    /// client is not at hand.
    pub(crate) fn stream(&self) -> &TcpStream {
        &self.stream
    }

    /// Connect and perform the QMP capabilities negotiation.
    pub fn connect(port: u16) -> Result<Self, QmpError> {
        let addr = format!("127.0.0.1:{}", port);
        let stream = TcpStream::connect(&addr)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let reader = BufReader::new(stream.try_clone()?);
        let mut client = Self { stream, reader };

        // Read the QMP greeting banner.
        client.read_line()?;

        client.execute("qmp_capabilities", &serde_json::json!({}))?;

        Ok(client)
    }

    /// Retry the connection until QMP answers or `timeout` expires.
    pub fn connect_with_retry(port: u16, timeout: Duration) -> Result<Self, QmpError> {
        let deadline = Instant::now() + timeout;
        loop {
            match Self::connect(port) {
                Ok(c) => return Ok(c),
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Execute a QMP command and return its "return" field.
    pub fn execute(&mut self, cmd: &str, args: &Value) -> Result<Value, QmpError> {
        let msg = if args.as_object().map(|m| m.is_empty()).unwrap_or(true) {
            serde_json::json!({ "execute": cmd })
        } else {
            serde_json::json!({ "execute": cmd, "arguments": args })
        };

        let mut line = serde_json::to_string(&msg)?;
        line.push('\n');
        self.stream.write_all(line.as_bytes())?;

        read_reply(&mut self.reader)
    }

    /// Add a raw block device file as a named blockdev node while QEMU runs.
    pub fn blockdev_add(&mut self, node_name: &str, path: &str) -> Result<Value, QmpError> {
        self.execute(
            "blockdev-add",
            &serde_json::json!({
                "driver": "raw",
                "node-name": node_name,
                "file": { "driver": "file", "filename": path }
            }),
        )
    }

    /// Remove a device by its QMP ID while QEMU runs.
    pub fn device_del(&mut self, id: &str) -> Result<Value, QmpError> {
        self.execute("device_del", &serde_json::json!({ "id": id }))
    }

    /// Remove a blockdev node. Call this after device_del has completed.
    pub fn blockdev_del(&mut self, node_name: &str) -> Result<Value, QmpError> {
        self.execute(
            "blockdev-del",
            &serde_json::json!({ "node-name": node_name }),
        )
    }

    /// Swap the medium behind a named drive, the id= of a -drive if=none. In
    /// QEMU, virtio_blk_change_media calls virtio_notify_config, which makes
    /// the guest call virtblk_config_changed and then set_capacity_and_notify.
    ///
    /// `format: None` leaves the driver chain to the filename, for a `json:`
    /// filename that spells it out.
    pub fn blockdev_change_medium(
        &mut self,
        drive_id: &str,
        filename: &str,
        format: Option<&str>,
    ) -> Result<Value, QmpError> {
        let mut args = serde_json::json!({ "device": drive_id, "filename": filename });
        if let Some(format) = format {
            args["format"] = format.into();
        }
        self.execute("blockdev-change-medium", &args)
    }

    /// Return the current backing filename for a named drive.
    ///
    /// - `Ok(Some(path))`: the drive exists and its medium is `path`.
    /// - `Ok(None)`: the drive exists but has no medium.
    /// - `Err(())`: the QMP call failed or the drive does not exist; the caller
    ///   should keep its current state.
    ///
    /// It reads `inserted.image.filename` first, and falls back to
    /// `inserted.file` when that is a plain path string instead of a node
    /// object.
    #[allow(clippy::result_unit_err)]
    pub fn query_block_medium(&mut self, drive_id: &str) -> Result<Option<String>, ()> {
        let ret = self
            .execute("query-block", &serde_json::json!({}))
            .map_err(|_| ())?;
        let devs = ret.as_array().ok_or(())?;
        for dev in devs {
            if dev.get("device").and_then(|d| d.as_str()) != Some(drive_id) {
                continue;
            }
            let inserted = match dev.get("inserted") {
                Some(i) => i,
                None => return Ok(None),
            };
            let filename = inserted
                .pointer("/image/filename")
                .or_else(|| inserted.get("file"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            return Ok(filename);
        }
        // The drive is not in the list.
        Err(())
    }

    /// Release an fdset handed over with [`add_fd`], closing QEMU's copy of the
    /// descriptor. Media opened from it keep their own duplicates.
    pub fn remove_fd(&mut self, fdset_id: u64) -> Result<(), QmpError> {
        self.execute("remove-fd", &serde_json::json!({ "fdset-id": fdset_id }))?;
        Ok(())
    }

    /// Send an ACPI power-button event, so that systemd in the guest shuts
    /// down gracefully. Errors are ignored: if QMP is already gone, the
    /// caller's next `wait_or_kill` sends SIGKILL to the child anyway, and
    /// the error would only race with QEMU closing the socket during
    /// shutdown.
    pub fn system_powerdown(&mut self) -> Result<(), QmpError> {
        let _ = self.execute("system_powerdown", &serde_json::json!({}));
        Ok(())
    }

    /// Send QMP "quit", which shuts QEMU down cleanly.
    pub fn quit(&mut self) -> Result<(), QmpError> {
        // Ignore errors, because QEMU may close the socket before the reply is
        // read.
        let _ = self.execute("quit", &serde_json::json!({}));
        Ok(())
    }

    fn read_line(&mut self) -> Result<String, QmpError> {
        read_line(&mut self.reader)
    }
}

fn read_line(reader: &mut impl BufRead) -> Result<String, QmpError> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(QmpError::Io(std::io::ErrorKind::UnexpectedEof.into()));
    }
    Ok(line)
}

/// Read lines until a command's reply, skipping events, and return its
/// `return` field.
fn read_reply(reader: &mut impl BufRead) -> Result<Value, QmpError> {
    loop {
        let resp_str = read_line(reader)?;
        if resp_str.trim().is_empty() {
            continue;
        }
        let resp: Value = serde_json::from_str(&resp_str)?;
        if let Some(ret) = resp.get("return") {
            return Ok(ret.clone());
        }
        if let Some(err) = resp.get("error") {
            let msg = err
                .get("desc")
                .and_then(|d| d.as_str())
                .unwrap_or("unknown QMP error")
                .to_string();
            return Err(QmpError::QemuError(msg));
        }
    }
}
