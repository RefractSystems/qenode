use anyhow::{anyhow, Result};
use std::sync::{Arc, Mutex};
use tokio::time::{timeout, Duration};
use tracing::info;
use virtmcu_api::telemetry_generated::virtmcu::telemetry::root_as_trace_event;
use virtmcu_api::{FlatBufferStructExt, ZenohSPIHeader, ZENOH_SPI_HEADER_SIZE};
use zenoh::sample::Sample;
use zenoh::Session;
use zenoh::Wait;

use flatbuffers::FlatBufferBuilder;
use virtmcu_api::flexray_generated::virtmcu::flexray::{
    root_as_flex_ray_frame, FlexRayFrame, FlexRayFrameArgs,
};
use virtmcu_api::lin_generated::virtmcu::lin::{
    root_as_lin_frame, LinFrame, LinFrameArgs, LinMessageType,
};

#[derive(Clone)]
pub struct AsyncMessageBuffer<T> {
    messages: Arc<Mutex<Vec<T>>>,
}

impl<T> AsyncMessageBuffer<T> {
    pub fn new() -> Self {
        Self {
            messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn push(&self, msg: T) {
        let mut msgs = self.messages.lock().unwrap();
        msgs.push(msg);
    }

    pub async fn wait_for_responses<F>(&self, timeout_secs: u64, predicate: F) -> Result<bool>
    where
        F: Fn(&[T]) -> bool,
    {
        timeout(Duration::from_secs(timeout_secs), async {
            loop {
                {
                    let msgs = self.messages.lock().unwrap();
                    if predicate(&msgs) {
                        return Ok(true);
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| anyhow!("Timed out waiting for predicate"))?
    }

    pub fn clear(&self) {
        let mut msgs = self.messages.lock().unwrap();
        msgs.clear();
    }
}

impl<T> Default for AsyncMessageBuffer<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ActuatorMonitor {
    #[allow(clippy::type_complexity)]
    // virtmcu-allow: allow reasoning="Monitor message types involve complex nested tuples for captured data"
    pub captured_messages: Arc<Mutex<Vec<(String, u64, Vec<f64>)>>>,
    _subscribers: Vec<zenoh::pubsub::Subscriber<()>>,
}

impl ActuatorMonitor {
    pub async fn new(session: &Session, topics: &[&str]) -> Result<Self> {
        let captured_messages = Arc::new(Mutex::new(Vec::new()));
        let mut _subscribers = Vec::new();

        for topic in topics {
            let captured_messages_clone = captured_messages.clone();
            let topic_string = topic.to_string();
            let sub = session
                .declare_subscriber(topic.to_string())
                .callback(move |sample: Sample| {
                    let payload = sample.payload().to_bytes();
                    if let Some((vtime, _seq, inner_payload)) =
                        virtmcu_api::decode_coord_message(&payload)
                    {
                        let mut vals = Vec::new();
                        for chunk in inner_payload.chunks_exact(8) {
                            let val = f64::from_le_bytes(chunk.try_into().unwrap());
                            vals.push(val);
                        }
                        let mut msgs = captured_messages_clone.lock().unwrap();
                        msgs.push((topic_string.clone(), vtime, vals));
                    }
                })
                .await
                .map_err(|e| anyhow!("Failed to subscribe to Actuator: {}", e))?;
            _subscribers.push(sub);
        }

        Ok(Self {
            captured_messages,
            _subscribers,
        })
    }

    pub async fn wait_for_responses<F>(&self, timeout_secs: u64, predicate: F) -> Result<bool>
    where
        F: Fn(&[(String, u64, Vec<f64>)]) -> bool,
    {
        timeout(Duration::from_secs(timeout_secs), async {
            loop {
                {
                    let msgs = self.captured_messages.lock().unwrap();
                    if predicate(&msgs) {
                        return Ok(true);
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| anyhow!("Timed out waiting for Actuator predicate"))?
    }
}

pub struct ChardevMonitor {
    pub captured_text: Arc<Mutex<String>>,
    _subscriber: zenoh::pubsub::Subscriber<()>,
    session: Session,
}

impl ChardevMonitor {
    pub async fn new(session: &Session, topic: &str) -> Result<Self> {
        let captured_text = Arc::new(Mutex::new(String::new()));
        let captured_text_clone = captured_text.clone();

        let sub = session
            .declare_subscriber(topic.to_string())
            .callback(move |sample: Sample| {
                let payload = sample.payload().to_bytes();
                if let Some((_vtime, _seq, inner_payload)) =
                    virtmcu_api::decode_coord_message(&payload)
                {
                    let text = String::from_utf8_lossy(inner_payload);
                    tracing::debug!("Chardev RX: {:?}", text);
                    let mut buf = captured_text_clone.lock().unwrap();
                    buf.push_str(&text);
                }
            })
            .await
            .map_err(|e| anyhow!("Failed to subscribe to Chardev: {}", e))?;

        Ok(Self {
            captured_text,
            _subscriber: sub,
            session: session.clone(),
        })
    }

    pub async fn wait_for_pattern(&self, timeout_secs: u64, pattern: &str) -> Result<bool> {
        timeout(Duration::from_secs(timeout_secs), async {
            loop {
                {
                    let buf = self.captured_text.lock().unwrap();
                    if buf.contains(pattern) {
                        return Ok(true);
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| {
            let buf = self.captured_text.lock().unwrap();
            anyhow!(
                "Timed out waiting for pattern: '{}'. Current buffer contents: {:?}",
                pattern,
                *buf
            )
        })?
    }

    pub async fn clear(&self) {
        let mut buf = self.captured_text.lock().unwrap();
        buf.clear();
    }

    pub async fn publish(&self, topic: &str, vtime_ns: u64, data: &[u8]) -> Result<()> {
        tracing::debug!(
            "Chardev TX (vtime {}): {:?}",
            vtime_ns,
            String::from_utf8_lossy(data)
        );

        let coord_msg = virtmcu_api::encode_coord_message(
            0,
            0,
            vtime_ns,
            0,
            virtmcu_api::core_generated::virtmcu::core::Protocol::Uart,
            data,
        );

        self.session
            .put(topic, coord_msg)
            .await
            .map_err(|e| anyhow!("Failed to publish Chardev: {}", e))?;
        Ok(())
    }
}

use virtmcu_api::topics::sim_topic;

pub struct TelemetryMonitor {
    pub captured_traces: Arc<Mutex<Vec<Vec<u8>>>>,
    _subscriber: zenoh::pubsub::Subscriber<()>,
}

impl TelemetryMonitor {
    pub async fn new(session: &Session, node_id: u32) -> Result<Self> {
        let node_id_str = node_id.to_string();
        let topic = sim_topic::telemetry_events(&node_id_str);
        let captured_traces = Arc::new(Mutex::new(Vec::new()));
        let captured_traces_clone = captured_traces.clone();

        let sub = session
            .declare_subscriber(topic.clone())
            .callback(move |sample: Sample| {
                let payload = sample.payload().to_bytes().to_vec();
                // Validate it's a parseable flatbuffer
                if root_as_trace_event(&payload).is_ok() {
                    let mut traces = captured_traces_clone.lock().unwrap();
                    traces.push(payload);
                }
            })
            .await
            .map_err(|e| anyhow!("Failed to subscribe to telemetry: {}", e))?;

        info!("Subscribed to Telemetry topic: {}", topic);

        Ok(Self {
            captured_traces,
            _subscriber: sub,
        })
    }

    pub async fn wait_for_traces(&self, count: usize, timeout_secs: u64) -> Result<Vec<Vec<u8>>> {
        timeout(Duration::from_secs(timeout_secs), async {
            loop {
                {
                    let traces = self.captured_traces.lock().unwrap();
                    if traces.len() >= count {
                        return Ok(traces.clone());
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| anyhow!("Timed out waiting for {} telemetry traces", count))?
    }

    pub async fn clear(&self) {
        let mut traces = self.captured_traces.lock().unwrap();
        traces.clear();
    }
}

pub struct SpiEchoMonitor {
    _queryable: zenoh::query::Queryable<()>,
}

impl SpiEchoMonitor {
    pub async fn new(session: &Session, bus_id: &str, cs: u32) -> Result<Self> {
        let topic = format!("sim/spi/{}/{}", bus_id, cs);
        info!("Registering SPI Echo Queryable on {}", topic);

        let qable = session
            .declare_queryable(topic)
            .callback(move |query: zenoh::query::Query| {
                if let Some(payload) = query.payload() {
                    let data_bytes = payload.to_bytes();
                    if data_bytes.len() >= ZENOH_SPI_HEADER_SIZE + 4 {
                        // Unpack header to validate, but we just echo the 4 data bytes back
                        let _header = ZenohSPIHeader::unpack(
                            data_bytes[..ZENOH_SPI_HEADER_SIZE].try_into().unwrap(),
                        );
                        let data = &data_bytes[ZENOH_SPI_HEADER_SIZE..ZENOH_SPI_HEADER_SIZE + 4];
                        let _ = query.reply(query.key_expr().clone(), data.to_vec()).wait();
                    }
                }
            })
            .await
            .map_err(|e| anyhow!("Failed to declare SPI queryable: {}", e))?;

        Ok(Self { _queryable: qable })
    }
}

pub struct LinMonitor {
    #[allow(clippy::type_complexity)]
    // virtmcu-allow: allow reasoning="Monitor message types involve complex nested tuples for captured data"
    pub captured_messages: Arc<Mutex<Vec<(LinMessageType, Vec<u8>)>>>,
    _subscriber: zenoh::pubsub::Subscriber<()>,
    session: Session,
}

impl LinMonitor {
    pub async fn new(session: &Session, topic: &str) -> Result<Self> {
        let captured_messages = Arc::new(Mutex::new(Vec::new()));
        let captured_messages_clone = captured_messages.clone();

        let sub = session
            .declare_subscriber(topic)
            .callback(move |sample: Sample| {
                let payload = sample.payload().to_bytes();
                if let Some((_vtime, _seq, inner_payload)) =
                    virtmcu_api::decode_coord_message(&payload)
                {
                    if let Ok(frame) = root_as_lin_frame(inner_payload) {
                        let data_vec = frame.data().map(|d| d.bytes().to_vec()).unwrap_or_default();
                        let mut msgs = captured_messages_clone.lock().unwrap();
                        msgs.push((frame.type_(), data_vec));
                    }
                }
            })
            .await
            .map_err(|e| anyhow!("Failed to subscribe to LIN: {}", e))?;

        Ok(Self {
            captured_messages,
            _subscriber: sub,
            session: session.clone(),
        })
    }

    pub async fn wait_for_responses<F>(&self, timeout_secs: u64, predicate: F) -> Result<bool>
    where
        F: Fn(&[(LinMessageType, Vec<u8>)]) -> bool,
    {
        timeout(Duration::from_secs(timeout_secs), async {
            loop {
                {
                    let msgs = self.captured_messages.lock().unwrap();
                    if predicate(&msgs) {
                        return Ok(true);
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| anyhow!("Timed out waiting for LIN predicate"))?
    }

    pub async fn publish(
        &self,
        topic: &str,
        vtime_ns: u64,
        msg_type: LinMessageType,
        data: Option<&[u8]>,
    ) -> Result<()> {
        let mut builder = FlatBufferBuilder::with_capacity(1024);
        let data_offset = data.map(|d| builder.create_vector(d));

        let mut frame_args = LinFrameArgs {
            delivery_vtime_ns: vtime_ns,
            type_: msg_type,
            ..Default::default()
        };
        if let Some(offset) = data_offset {
            frame_args.data = Some(offset);
        }

        let frame = LinFrame::create(&mut builder, &frame_args);
        builder.finish(frame, None);

        let coord_msg = virtmcu_api::encode_coord_message(
            0,
            0,
            vtime_ns,
            0,
            virtmcu_api::core_generated::virtmcu::core::Protocol::Lin,
            builder.finished_data(),
        );

        self.session
            .put(topic, coord_msg)
            .await
            .map_err(|e| anyhow!("Failed to publish LIN: {}", e))?;
        Ok(())
    }
}

pub struct FlexRayMonitor {
    #[allow(clippy::type_complexity)]
    // virtmcu-allow: allow reasoning="Monitor message types involve complex nested tuples for captured data"
    pub captured_messages: Arc<Mutex<Vec<(u16, Vec<u8>)>>>,
    _subscriber: zenoh::pubsub::Subscriber<()>,
    session: Session,
}

impl FlexRayMonitor {
    pub async fn new(session: &Session, topic: &str) -> Result<Self> {
        let captured_messages = Arc::new(Mutex::new(Vec::new()));
        let captured_messages_clone = captured_messages.clone();

        let sub = session
            .declare_subscriber(topic)
            .callback(move |sample: Sample| {
                let payload = sample.payload().to_bytes();
                if let Some((_vtime, _seq, inner_payload)) =
                    virtmcu_api::decode_coord_message(&payload)
                {
                    if let Ok(frame) = root_as_flex_ray_frame(inner_payload) {
                        let data_vec = frame.data().map(|d| d.bytes().to_vec()).unwrap_or_default();
                        let mut msgs = captured_messages_clone.lock().unwrap();
                        msgs.push((frame.frame_id(), data_vec));
                    }
                }
            })
            .await
            .map_err(|e| anyhow!("Failed to subscribe to FlexRay: {}", e))?;

        Ok(Self {
            captured_messages,
            _subscriber: sub,
            session: session.clone(),
        })
    }

    pub async fn wait_for_responses<F>(&self, timeout_secs: u64, predicate: F) -> Result<bool>
    where
        F: Fn(&[(u16, Vec<u8>)]) -> bool,
    {
        timeout(Duration::from_secs(timeout_secs), async {
            loop {
                {
                    let msgs = self.captured_messages.lock().unwrap();
                    if predicate(&msgs) {
                        return Ok(true);
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| anyhow!("Timed out waiting for FlexRay predicate"))?
    }

    pub async fn publish(
        &self,
        topic: &str,
        vtime_ns: u64,
        frame_id: u16,
        data: Option<&[u8]>,
    ) -> Result<()> {
        let mut builder = FlatBufferBuilder::with_capacity(1024);
        let data_offset = data.map(|d| builder.create_vector(d));

        let mut frame_args = FlexRayFrameArgs {
            delivery_vtime_ns: vtime_ns,
            frame_id,
            ..Default::default()
        };
        if let Some(offset) = data_offset {
            frame_args.data = Some(offset);
        }

        let frame = FlexRayFrame::create(&mut builder, &frame_args);
        builder.finish(frame, None);

        let coord_msg = virtmcu_api::encode_coord_message(
            0,
            0,
            vtime_ns,
            0,
            virtmcu_api::core_generated::virtmcu::core::Protocol::FlexRay,
            builder.finished_data(),
        );

        self.session
            .put(topic, coord_msg)
            .await
            .map_err(|e| anyhow!("Failed to publish FlexRay: {}", e))?;
        Ok(())
    }
}
