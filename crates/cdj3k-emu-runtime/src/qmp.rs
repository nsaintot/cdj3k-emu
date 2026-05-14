use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::Value;

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
    /// Connect and perform the QMP capabilities negotiation.
    pub fn connect(port: u16) -> Result<Self, QmpError> {
        let addr = format!("127.0.0.1:{}", port);
        let stream = TcpStream::connect(&addr)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let reader = BufReader::new(stream.try_clone()?);
        let mut client = Self { stream, reader };

        // Read the QMP greeting banner.
        client.read_line()?;

        // Negotiate capabilities.
        client.execute("qmp_capabilities", &serde_json::json!({}))?;

        Ok(client)
    }

    /// Poll until QMP becomes available or timeout expires.
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

    /// Execute a QMP command.  Returns the "return" field on success.
    pub fn execute(&mut self, cmd: &str, args: &Value) -> Result<Value, QmpError> {
        let msg = if args.as_object().map(|m| m.is_empty()).unwrap_or(true) {
            serde_json::json!({ "execute": cmd })
        } else {
            serde_json::json!({ "execute": cmd, "arguments": args })
        };

        let mut line = serde_json::to_string(&msg)?;
        line.push('\n');
        self.stream.write_all(line.as_bytes())?;

        // Read response lines until we see a "return" or "error" key.
        loop {
            let resp_str = self.read_line()?;
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
            // Could be an event - skip and read next line.
        }
    }

    /// Hot-add a raw block device file as a named blockdev node.
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

    /// Hot-remove a device by QMP id.
    pub fn device_del(&mut self, id: &str) -> Result<Value, QmpError> {
        self.execute("device_del", &serde_json::json!({ "id": id }))
    }

    /// Remove a blockdev node (call after device_del settles).
    pub fn blockdev_del(&mut self, node_name: &str) -> Result<Value, QmpError> {
        self.execute(
            "blockdev-del",
            &serde_json::json!({ "node-name": node_name }),
        )
    }

    /// Swap the medium behind a named drive (id= from -drive if=none).
    /// Triggers virtio_blk_change_media → virtio_notify_config on the QEMU side,
    /// which fires virtblk_config_changed → revalidate_disk in the Pioneer guest.
    pub fn blockdev_change_medium(
        &mut self,
        drive_id: &str,
        filename: &str,
        format: &str,
    ) -> Result<Value, QmpError> {
        self.execute(
            "blockdev-change-medium",
            &serde_json::json!({
                "device": drive_id,
                "filename": filename,
                "format": format
            }),
        )
    }

    /// Return the current backing filename for a named drive.
    ///
    /// - `Ok(Some(path))` - drive found, medium path is `path`
    /// - `Ok(None)`       - drive found but no inserted medium
    /// - `Err(())`        - QMP call failed or drive not found; caller should keep existing state
    ///
    /// Tries `inserted.image.filename` first, falling back to `inserted.file`
    /// when it's a plain path string instead of a node object.
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
        Err(()) // drive not found in list
    }

    /// Send ACPI power button event - guest systemd handles graceful shutdown.
    /// Errors are swallowed: if QMP is already gone the caller's next
    /// `wait_or_kill` will SIGKILL the child anyway, and surfacing the error
    /// would just race with the QEMU socket closing on us mid-shutdown.
    pub fn system_powerdown(&mut self) -> Result<(), QmpError> {
        let _ = self.execute("system_powerdown", &serde_json::json!({}));
        Ok(())
    }

    /// Send QMP "quit" - triggers clean QEMU shutdown.
    pub fn quit(&mut self) -> Result<(), QmpError> {
        // Ignore errors: QEMU may close the socket before we read the response.
        let _ = self.execute("quit", &serde_json::json!({}));
        Ok(())
    }

    fn read_line(&mut self) -> Result<String, QmpError> {
        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        Ok(line)
    }
}
