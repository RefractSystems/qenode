use crossbeam_channel::{unbounded, Sender};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::thread;
use virtmcu_api::{DataCallback, DataTransport};

pub struct UnixDataTransport {
    _stream: Arc<Mutex<UnixStream>>,
    subscriptions: Arc<Mutex<Vec<(String, DataCallback)>>>,
    tx: Sender<(String, Vec<u8>)>,
}

impl UnixDataTransport {
    pub fn new(path: &str) -> Result<Self, String> {
        let stream = UnixStream::connect(path).map_err(|e| e.to_string())?;
        let mut read_stream = stream.try_clone().map_err(|e| e.to_string())?;
        let stream = Arc::new(Mutex::new(stream));
        let subscriptions: Arc<Mutex<Vec<(String, DataCallback)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = unbounded::<(String, Vec<u8>)>();

        let stream_clone = Arc::clone(&stream);
        let subscriptions_clone = Arc::clone(&subscriptions);

        // RX thread
        thread::spawn(move || loop {
            let (topic, payload) = {
                let mut topic_len_buf = [0u8; 4];
                if read_stream.read_exact(&mut topic_len_buf).is_err() {
                    break;
                }
                let topic_len = u32::from_le_bytes(topic_len_buf) as usize;

                let mut topic_buf = vec![0u8; topic_len];
                if read_stream.read_exact(&mut topic_buf).is_err() {
                    break;
                }
                let topic = String::from_utf8_lossy(&topic_buf).into_owned();

                let mut payload_len_buf = [0u8; 4];
                if read_stream.read_exact(&mut payload_len_buf).is_err() {
                    break;
                }
                let payload_len = u32::from_le_bytes(payload_len_buf) as usize;

                let mut payload = vec![0u8; payload_len];
                if read_stream.read_exact(&mut payload).is_err() {
                    break;
                }
                (topic, payload)
            };

            let subs = subscriptions_clone.lock().unwrap();
            for (sub_topic, callback) in subs.iter() {
                if sub_topic == &topic || topic.starts_with(sub_topic) {
                    callback(&payload);
                }
            }
        });

        // TX thread
        let stream_clone_tx = Arc::clone(&stream);
        thread::spawn(move || {
            while let Ok((topic, payload)) = rx.recv() {
                let mut buf = Vec::new();
                let topic_bytes = topic.as_bytes();
                buf.extend_from_slice(&(topic_bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(topic_bytes);
                buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                buf.extend_from_slice(&payload);

                let mut stream = stream_clone_tx.lock().unwrap();
                if stream.write_all(&buf).is_err() {
                    break;
                }
            }
        });

        Ok(Self { _stream: stream, subscriptions, tx })
    }
}

impl DataTransport for UnixDataTransport {
    fn publish(&self, topic: &str, payload: &[u8]) -> Result<(), String> {
        self.tx.send((topic.to_string(), payload.to_vec())).map_err(|e| e.to_string())
    }

    fn subscribe(&self, topic: &str, callback: DataCallback) -> Result<(), String> {
        self.subscriptions.lock().unwrap().push((topic.to_string(), callback));
        Ok(())
    }
}
