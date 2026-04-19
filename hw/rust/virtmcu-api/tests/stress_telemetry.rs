use flatbuffers::FlatBufferBuilder;
use std::time::Instant;
use virtmcu_api::{
    telemetry_fb, telemetry_generated::virtmcu::telemetry::TraceEvent as GenTraceEvent,
};

#[test]
fn test_stress_telemetry_serialization() {
    let mut builder = FlatBufferBuilder::new();
    let num_events = 100_000;

    let start = Instant::now();
    for i in 0..num_events {
        builder.reset();

        let device_name = if i % 2 == 0 {
            Some(builder.create_string(&format!("device_{}", i)))
        } else {
            None
        };

        let args = telemetry_fb::TraceEventArgs {
            timestamp_ns: i as u64 * 1000,
            type_: telemetry_fb::TraceEventType::Irq,
            id: i as u32,
            value: (i % 2) as u32,
            device_name,
        };

        let root = telemetry_fb::create_trace_event(&mut builder, &args);
        builder.finish(root, None);

        let payload = builder.finished_data();
        let parsed_event = flatbuffers::root::<GenTraceEvent>(payload).expect("Failed to parse");

        assert_eq!(parsed_event.timestamp_ns(), i as u64 * 1000);
        assert_eq!(parsed_event.id(), i as u32);
    }
    let duration = start.elapsed();
    println!(
        "Serialized and deserialized {} events in {:?}",
        num_events, duration
    );
}
