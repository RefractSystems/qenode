use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::thread;

pub struct UnixDataRouter {
    listener: UnixListener,
    clients: Arc<Mutex<Vec<UnixStream>>>,
}

impl UnixDataRouter {
    pub fn new(path: &str) -> Result<Self, String> {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).map_err(|e| e.to_string())?;
        Ok(Self { listener, clients: Arc::new(Mutex::new(Vec::new())) })
    }

    pub fn serve(self) {
        for stream in self.listener.incoming() {
            match stream {
                Ok(stream) => {
                    let clients_clone = Arc::clone(&self.clients);
                    clients_clone
                        .lock()
                        .expect("unix socket operation failed")
                        .push(stream.try_clone().expect("unix socket operation failed"));

                    let mut read_stream = stream;
                    thread::spawn(move || loop {
                        let (topic_bytes, payload) = {
                            let mut topic_len_buf = [0u8; 4];
                            if read_stream.read_exact(&mut topic_len_buf).is_err() {
                                break;
                            }
                            let topic_len = u32::from_le_bytes(topic_len_buf) as usize;

                            let mut topic_bytes = vec![0u8; topic_len];
                            if read_stream.read_exact(&mut topic_bytes).is_err() {
                                break;
                            }

                            let mut payload_len_buf = [0u8; 4];
                            if read_stream.read_exact(&mut payload_len_buf).is_err() {
                                break;
                            }
                            let payload_len = u32::from_le_bytes(payload_len_buf) as usize;

                            let mut payload = vec![0u8; payload_len];
                            if read_stream.read_exact(&mut payload).is_err() {
                                break;
                            }
                            (topic_bytes, payload)
                        };

                        // Broadcast to all other clients
                        let mut buf = Vec::new();
                        buf.extend_from_slice(&(topic_bytes.len() as u32).to_le_bytes());
                        buf.extend_from_slice(&topic_bytes);
                        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                        buf.extend_from_slice(&payload);

                        let mut clients =
                            clients_clone.lock().expect("unix socket operation failed");
                        clients.retain_mut(|client| client.write_all(&buf).is_ok());
                    });
                }
                Err(err) => {
                    ::virtmcu_qom::sim_err!("UnixDataRouter accept error: {}", err);
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
#[cfg(not(miri))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    #[test]
    #[cfg(not(miri))]
    fn test_unix_data_router_broadcast() {
        let dir = tempdir().expect("unix socket operation failed");
        let sock_path = dir.path().join("router.sock");
        let sock_str = sock_path.to_str().expect("unix socket operation failed").to_string();

        let router = UnixDataRouter::new(&sock_str).expect("unix socket operation failed");
        let clients_ref = Arc::clone(&router.clients);

        thread::spawn(move || {
            router.serve();
        });

        // listener.bind happens synchronously in new(), so connect is immediately safe.
        let mut client1 = UnixStream::connect(&sock_str).expect("unix socket operation failed");
        let mut client2 = UnixStream::connect(&sock_str).expect("unix socket operation failed");
        let mut client3 = UnixStream::connect(&sock_str).expect("unix socket operation failed");

        // Deterministic Synchronization: Wait for the router thread to accept all 3 clients
        let start = Instant::now();
        while clients_ref.lock().expect("unix socket operation failed").len() < 3 {
            if start.elapsed() > Duration::from_secs(5) {
                panic!("Timeout waiting for router to accept clients");
            }
            std::thread::yield_now(); // virtmcu-allow: yield reasoning="legacy spinloop"
        }

        let topic = b"test/topic";
        let payload = b"hello world";

        let mut msg = Vec::new();
        msg.extend_from_slice(&(topic.len() as u32).to_le_bytes());
        msg.extend_from_slice(topic);
        msg.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        msg.extend_from_slice(payload);

        client1.write_all(&msg).expect("unix socket operation failed");

        // Set read timeouts so tests don't hang
        client2
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("unix socket operation failed");
        client3
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("unix socket operation failed");

        let mut read_buf = vec![0u8; msg.len()];
        client2.read_exact(&mut read_buf).expect("unix socket operation failed");
        assert_eq!(read_buf, msg);

        let mut read_buf = vec![0u8; msg.len()];
        client3.read_exact(&mut read_buf).expect("unix socket operation failed");
        assert_eq!(read_buf, msg);

        // Also note that client1 receives its own broadcast in this naive implementation.
        // That is acceptable for DataRouter for now, as subscribers filter by topic anyway.
        client1
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("unix socket operation failed");
        let mut read_buf = vec![0u8; msg.len()];
        client1.read_exact(&mut read_buf).expect("unix socket operation failed");
        assert_eq!(read_buf, msg);
    }
}
