use flatbuffers::FlatBufferBuilder;

#[test]
fn test_trace_event_power_uw_serialization() {
    let mut builder = FlatBufferBuilder::with_capacity(128);

    // Create device name
    let device_name = builder.create_string("test_motor");

    use virtmcu_api::telemetry_generated::virtmcu::telemetry::{
        TraceEvent, TraceEventArgs, TraceEventType,
    };

    let args = TraceEventArgs {
        timestamp_ns: 1000,
        type_: TraceEventType::POWER_STATE,
        id: 42,
        value: 1,
        device_name: Some(device_name),
        power_uw: 500000, // 0.5W
    };

    let offset = TraceEvent::create(&mut builder, &args);
    builder.finish(offset, None);

    let buf = builder.finished_data();

    // Verify using the generated root type
    let event =
        flatbuffers::root::<virtmcu_api::telemetry_generated::virtmcu::telemetry::TraceEvent>(buf)
            .expect("test should succeed");

    assert_eq!(event.timestamp_ns(), 1000);
    assert_eq!(event.type_().0, 3); // POWER_STATE
    assert_eq!(event.id(), 42);
    assert_eq!(event.power_uw(), 500000);
    assert_eq!(event.device_name(), Some("test_motor"));
}
