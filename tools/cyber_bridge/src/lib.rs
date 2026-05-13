pub mod physics;
pub mod physics_transport;
pub mod resd_parser;

use std::sync::Arc;
use virtmcu_api::{ClockAdvanceReq, ClockReadyResp, FlatBufferStructExt, PhysicalNodeTransport};
use zenoh::{Session, Wait};

/// A Zenoh-backed implementation of the `PhysicalNodeTransport` trait.
pub struct ZenohPhysicalNodeTransport {
    session: Arc<Session>,
    topic: String,
}

impl ZenohPhysicalNodeTransport {
    /// Creates a new `ZenohPhysicalNodeTransport` using the provided Zenoh session and node ID.
    pub fn new(session: Arc<Session>, node_id: u32) -> Self {
        let topic = format!("sim/clock/advance/{node_id}");
        Self { session, topic }
    }
}

impl PhysicalNodeTransport for ZenohPhysicalNodeTransport {
    fn advance(
        &self,
        req: ClockAdvanceReq,
        timeout: core::time::Duration,
    ) -> Option<ClockReadyResp> {
        let bytes = req.pack();

        let replies = self
            .session
            .get(&self.topic)
            .payload(bytes)
            .timeout(timeout)
            .wait()
            .ok()?;

        while let Ok(reply) = replies.recv() {
            if let Ok(sample) = reply.result() {
                let data = sample.payload().to_bytes();
                return ClockReadyResp::unpack_slice(&data);
            }
        }

        None
    }
}
pub use virtmcu_api::ActuatorMap;

pub struct ZenohActuatorSink {
    buffer: Arc<std::sync::Mutex<ActuatorMap>>,
    _subscriber: zenoh::pubsub::Subscriber<()>,
}

impl ZenohActuatorSink {
    pub async fn new(
        session: &zenoh::Session,
        topic_prefix: &str,
        node_id: u32,
    ) -> anyhow::Result<Self> {
        let buffer = Arc::new(std::sync::Mutex::new(ActuatorMap::new()));
        let buffer_clone = Arc::clone(&buffer);
        let filter = format!("{}/{}/{}", topic_prefix, node_id, "**");
        let subscriber = session
            .declare_subscriber(filter)
            .callback(move |sample| {
                let raw = sample.payload().to_bytes();
                if raw.len() < virtmcu_api::ZENOH_FRAME_HEADER_SIZE {
                    return;
                }
                let Some((header, data_bytes)) = virtmcu_api::decode_frame(&raw) else {
                    return;
                };
                let vtime = header.delivery_vtime_ns();
                let topic = sample.key_expr().as_str();
                let Some(actuator_id_str) = topic.split('/').next_back() else {
                    return;
                };
                let Ok(actuator_id) = actuator_id_str.parse::<u32>() else {
                    return;
                };

                let mut vals: Vec<f64> = Vec::new();
                for chunk in data_bytes.chunks_exact(8) {
                    if let Ok(arr) = <[u8; 8]>::try_from(chunk) {
                        vals.push(f64::from_le_bytes(arr));
                    }
                }
                if vals.is_empty() {
                    return;
                }

                if let Ok(mut map) = buffer_clone.lock() {
                    map.entry(vtime).or_default().insert(actuator_id, vals);
                }
            })
            .wait()
            .map_err(|e| anyhow::anyhow!("Failed to declare actuator subscriber: {}", e))?;

        Ok(Self {
            buffer,
            _subscriber: subscriber,
        })
    }

    pub fn drain(&self) -> ActuatorMap {
        let mut map = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use virtmcu_api::{ClockAdvanceReq, ClockReadyResp, FlatBufferStructExt, CLOCK_ERROR_OK};
    use zenoh::Wait;

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_zenoh_physical_node_transport() {
        let mut config = zenoh::Config::default();
        let _ = config.insert_json5("mode", "\"peer\"");
        let _ = config.insert_json5("scouting/multicast/enabled", "true");

        let session = Arc::new(zenoh::open(config).wait().unwrap());
        let node_id = 42;
        let topic = format!("sim/clock/advance/{node_id}");

        // Fake QEMU side: a queryable that echoes back the request in a response
        let session_clone = Arc::clone(&session);
        let _queryable = session_clone
            .declare_queryable(&topic)
            .callback(move |query| {
                let data = query.payload().unwrap().to_bytes();
                let req = ClockAdvanceReq::unpack_slice(&data).unwrap();
                let resp = ClockReadyResp::new(
                    req.absolute_vtime_ns() + req.delta_ns(),
                    0,
                    CLOCK_ERROR_OK,
                    req.quantum_number(),
                );
                let _ = query
                    .reply(query.key_expr().clone(), resp.pack().to_vec())
                    .wait();
            })
            .wait()
            .unwrap();

        let transport = ZenohPhysicalNodeTransport::new(Arc::clone(&session), node_id);
        let req = ClockAdvanceReq::new(1000, 5000, 5);

        // Give some time for discovery in peer mode
        tokio::time::sleep(Duration::from_millis(100)).await;

        let resp = transport
            .advance(req, Duration::from_secs(5))
            .expect("Advance failed");

        assert_eq!(resp.current_vtime_ns(), 6000);
        assert_eq!(resp.quantum_number(), 5);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_zenoh_actuator_sink() {
        let mut config = zenoh::Config::default();
        let _ = config.insert_json5("mode", "\"peer\"");
        let _ = config.insert_json5("scouting/multicast/enabled", "true");

        let session = Arc::new(zenoh::open(config).wait().unwrap());
        let node_id = 0;
        let prefix = "firmware/control";

        let sink = ZenohActuatorSink::new(&session, prefix, node_id)
            .await
            .unwrap();

        let topic = format!("{}/{}/7", prefix, node_id);
        let vals = vec![1.0f64, 2.0f64];
        let mut data_payload: Vec<u8> = Vec::new();
        for val in &vals {
            data_payload.extend_from_slice(&val.to_le_bytes());
        }
        let payload = virtmcu_api::encode_frame(1000, 0, &data_payload);

        session.put(&topic, payload).wait().unwrap();

        // Give some time for Zenoh delivery
        tokio::time::sleep(Duration::from_millis(200)).await;

        let drained = sink.drain();
        assert_eq!(drained.get(&1000).unwrap().get(&7), Some(&vals));

        let drained_again = sink.drain();
        assert!(drained_again.is_empty());
    }
}
