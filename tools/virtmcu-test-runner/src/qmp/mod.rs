use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream as UnixStreamSync;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::debug;

pub struct QmpClient {
    stream: UnixStream,
}

impl QmpClient {
    pub async fn connect(path: &Path) -> Result<Self> {
        let mut stream = UnixStream::connect(path).await?;

        // Initial handshake: QEMU sends greeting, we send qmp_capabilities
        let mut buf = [0u8; 4096];
        let bytes_read = stream.read(&mut buf).await?;
        debug!(
            "QMP Greeting: {}",
            String::from_utf8_lossy(&buf[..bytes_read])
        );

        stream
            .write_all(b"{\"execute\": \"qmp_capabilities\"}\n")
            .await?;
        let bytes_read = stream.read(&mut buf).await?;
        debug!(
            "QMP Capabilities response: {}",
            String::from_utf8_lossy(&buf[..bytes_read])
        );

        Ok(Self { stream })
    }

    pub async fn execute_with_args(&mut self, cmd: &str, args: Option<Value>) -> Result<Value> {
        let mut request = json!({
            "execute": cmd
        });

        if let Some(args_val) = args {
            request["arguments"] = args_val;
        }

        let payload = format!("{}\n", request);
        self.stream.write_all(payload.as_bytes()).await?;

        // Loop to find the actual response, ignoring events
        let mut buf = [0u8; 8192];
        loop {
            let bytes_read = self.stream.read(&mut buf).await?;
            if bytes_read == 0 {
                return Err(anyhow!("QMP Socket reached EOF"));
            }

            let resp_str = String::from_utf8_lossy(&buf[..bytes_read]);
            // Responses might be separated by newlines
            for line in resp_str.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                debug!("QMP Response Line: {}", line);
                let parsed: Value = serde_json::from_str(line)?;
                if parsed.get("return").is_some() || parsed.get("error").is_some() {
                    return Ok(parsed);
                }
            }
        }
    }

    pub async fn execute(&mut self, cmd: &str) -> Result<Value> {
        self.execute_with_args(cmd, None).await
    }

    pub async fn quit(&mut self) -> Result<()> {
        self.execute("quit").await?;
        Ok(())
    }

    pub async fn cont(&mut self) -> Result<()> {
        self.execute("cont").await?;
        Ok(())
    }

    pub async fn system_reset(&mut self) -> Result<()> {
        self.execute("system_reset").await?;
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        self.execute("stop").await?;
        Ok(())
    }
}

/// A synchronous QMP client for use in Drop handlers
pub struct QmpClientSync {
    stream: UnixStreamSync,
}

impl QmpClientSync {
    pub fn connect(path: &Path) -> Result<Self> {
        let mut stream = UnixStreamSync::connect(path)?;
        stream.set_read_timeout(Some(std::time::Duration::from_millis(500)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_millis(500)))?;

        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf)?;
        stream.write_all(b"{\"execute\": \"qmp_capabilities\"}\n")?;
        let _ = stream.read(&mut buf)?;

        Ok(Self { stream })
    }

    pub fn execute(&mut self, cmd: &str) -> Result<()> {
        let request = json!({ "execute": cmd });
        let payload = format!("{}\n", request);
        self.stream.write_all(payload.as_bytes())?;
        Ok(())
    }

    pub fn quit(&mut self) -> Result<()> {
        self.execute("quit")
    }
}
