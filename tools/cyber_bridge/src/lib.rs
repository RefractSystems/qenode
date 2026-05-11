pub mod physics;
pub mod resd_parser;

use std::sync::Arc;
use virtmcu_api::{ClockAdvanceReq, ClockReadyResp, FlatBufferStructExt, TimeAuthorityTransport};
use zenoh::{Session, Wait};

/// A Zenoh-backed implementation of the `TimeAuthorityTransport` trait.
pub struct ZenohTimeAuthorityTransport {
    session: Arc<Session>,
    topic: String,
}

impl ZenohTimeAuthorityTransport {
    /// Creates a new `ZenohTimeAuthorityTransport` using the provided Zenoh session and node ID.
    pub fn new(session: Arc<Session>, node_id: u32) -> Self {
        let topic = format!("sim/clock/advance/{node_id}");
        Self { session, topic }
    }
}

impl TimeAuthorityTransport for ZenohTimeAuthorityTransport {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use virtmcu_api::{ClockAdvanceReq, ClockReadyResp, FlatBufferStructExt, CLOCK_ERROR_OK};
    use zenoh::Wait;

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn test_zenoh_time_authority_transport() {
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

        let transport = ZenohTimeAuthorityTransport::new(Arc::clone(&session), node_id);
        let req = ClockAdvanceReq::new(1000, 5000, 5);

        // Give some time for discovery in peer mode
        tokio::time::sleep(Duration::from_millis(100)).await;

        let resp = transport
            .advance(req, Duration::from_secs(5))
            .expect("Advance failed");

        assert_eq!(resp.current_vtime_ns(), 6000);
        assert_eq!(resp.quantum_number(), 5);
    }
}
