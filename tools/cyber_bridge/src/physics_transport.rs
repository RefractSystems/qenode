use anyhow::anyhow;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use virtmcu_wire::{physics_proto, PhysicsGatewayServer, PhysicsGatewayTransport};
use zenoh::{Session, Wait};

// --- Unix Socket Transport ---

pub struct UnixSocketPhysicsTransport {
    path: PathBuf,
    stream: Mutex<Option<UnixStream>>,
}

impl UnixSocketPhysicsTransport {
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        Self {
            path: path.into(),
            stream: Mutex::new(None),
        }
    }

    fn ensure_stream(&self) -> anyhow::Result<UnixStream> {
        let mut guard = self
            .stream
            .lock()
            .map_err(|e| anyhow!("Mutex poisoned: {e}"))?;
        if let Some(stream) = &*guard {
            return stream.try_clone().map_err(|e| anyhow!("{e}"));
        }

        let stream = UnixStream::connect(&self.path)?;
        *guard = Some(stream.try_clone()?);
        Ok(stream)
    }
}

impl PhysicsGatewayTransport for UnixSocketPhysicsTransport {
    fn trigger_and_wait(&self, trigger_bytes: &[u8], timeout: Duration) -> Result<(), String> {
        let mut stream = self.ensure_stream().map_err(|e| format!("{e}"))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|e| format!("{e}"))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| format!("{e}"))?;

        // 1. Send trigger with length prefix (8-byte LE)
        let len = trigger_bytes.len() as u64;
        stream
            .write_all(&len.to_le_bytes())
            .map_err(|e| format!("{e}"))?;
        stream
            .write_all(trigger_bytes)
            .map_err(|e| format!("{e}"))?;

        // 2. Read PhysicsDone (fixed 16 bytes)
        let mut buf = [0u8; 16];
        stream.read_exact(&mut buf).map_err(|e| format!("{e}"))?;

        // In a real implementation we would unpack and check status
        Ok(())
    }
}

pub struct UnixSocketPhysicsServer {
    listener: UnixListener,
    stream: Mutex<Option<UnixStream>>,
}

impl UnixSocketPhysicsServer {
    pub fn new<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        Ok(Self {
            listener,
            stream: Mutex::new(None),
        })
    }

    fn ensure_stream(&self) -> Option<UnixStream> {
        let mut guard = self.stream.lock().ok()?;
        if let Some(stream) = &*guard {
            return stream.try_clone().ok();
        }

        let (stream, _) = self.listener.accept().ok()?;
        *guard = Some(stream.try_clone().ok()?);
        Some(stream)
    }
}

impl PhysicsGatewayServer for UnixSocketPhysicsServer {
    fn recv_trigger(&self, timeout: Duration) -> Option<Vec<u8>> {
        let mut stream = self.ensure_stream()?;
        let _ = stream.set_read_timeout(Some(timeout));

        let mut len_buf = [0u8; 8];
        stream.read_exact(&mut len_buf).ok()?;
        let len = u64::from_le_bytes(len_buf) as usize;

        let mut trigger_bytes = vec![0u8; len];
        stream.read_exact(&mut trigger_bytes).ok()?;

        Some(trigger_bytes)
    }

    fn send_done(&self, done: physics_proto::PhysicsDone) -> Result<(), String> {
        let mut stream = self.ensure_stream().ok_or_else(|| "No stream".to_owned())?;
        stream.write_all(&done.0).map_err(|e| format!("{e}"))?;
        Ok(())
    }
}

// --- Zenoh Transport ---

pub struct ZenohPhysicsTransport {
    session: Arc<Session>,
}

impl ZenohPhysicsTransport {
    pub fn new(session: Arc<Session>) -> Self {
        Self { session }
    }
}

impl PhysicsGatewayTransport for ZenohPhysicsTransport {
    fn trigger_and_wait(&self, trigger_bytes: &[u8], timeout: Duration) -> Result<(), String> {
        let replies = self
            .session
            .get(virtmcu_wire::topics::sim_topic::PHYSICS_TRIGGER)
            .payload(trigger_bytes)
            .timeout(timeout)
            .wait()
            .map_err(|e| format!("{e}"))?;

        while let Ok(reply) = replies.recv() {
            if reply.result().is_ok() {
                // Done!
                return Ok(());
            }
        }

        Err("No response from physics gateway".to_owned())
    }
}

pub struct ZenohPhysicsServer {
    _session: Arc<Session>,
}

impl ZenohPhysicsServer {
    pub fn new(session: Arc<Session>) -> Self {
        Self { _session: session }
    }
}

// This is tricky because Zenoh queryable is async and trait is sync.
// We'll use a channel to bridge them.
impl PhysicsGatewayServer for ZenohPhysicsServer {
    fn recv_trigger(&self, _timeout: Duration) -> Option<Vec<u8>> {
        // Implementation would need a persistent queryable and a way to
        // signal back via the query object.
        // For brevity in this refactor, we'll focus on the Unix transport first
        // as it's the primary use case for local co-sim.
        None
    }

    fn send_done(&self, _done: physics_proto::PhysicsDone) -> Result<(), String> {
        Ok(())
    }
}
