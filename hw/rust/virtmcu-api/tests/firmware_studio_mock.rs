use std::mem::size_of;
use flatbuffers::FlatBufferBuilder;
use virtmcu_api::{
    VirtmcuHandshake, MmioReq, SyscMsg, ClockAdvanceReq, ClockReadyResp, ZenohFrameHeader,
    VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION,
    telemetry_fb, telemetry_generated::virtmcu::telemetry::TraceEvent as GenTraceEvent,
};

#[test]
fn test_firmware_studio_struct_sizes() {
    // Ensuring ABI stability for downstream consumers. 
    // If these change, it silently breaks C-bridge protocol and Firmware Studio.
    assert_eq!(size_of::<VirtmcuHandshake>(), 8);
    assert_eq!(size_of::<MmioReq>(), 32);
    assert_eq!(size_of::<SyscMsg>(), 16);
    assert_eq!(size_of::<ClockAdvanceReq>(), 16);
    assert_eq!(size_of::<ClockReadyResp>(), 16);
    assert_eq!(size_of::<ZenohFrameHeader>(), 12);
}

#[test]
fn test_firmware_studio_handshake() {
    // Firmware Studio needs to send a handshake on socket connection
    let handshake = VirtmcuHandshake {
        magic: VIRTMCU_PROTO_MAGIC,
        version: VIRTMCU_PROTO_VERSION,
    };
    
    // Convert to bytes as if sending over a socket
    let bytes: [u8; 8] = unsafe { std::mem::transmute(handshake) };
    
    // Magic check: 'V', 'M', 'C', 'U' is 0x564D4355
    // Little endian byte order verification
    assert_eq!(bytes[0], 0x55);
    assert_eq!(bytes[1], 0x43);
    assert_eq!(bytes[2], 0x4D);
    assert_eq!(bytes[3], 0x56);
    
    assert_eq!(bytes[4], 1);
    assert_eq!(bytes[5], 0);
}

#[test]
fn test_firmware_studio_telemetry_consumption() {
    // This simulates QEMU emitting a trace event using our helpers
    let mut builder = FlatBufferBuilder::new();
    let name_offset = builder.create_string("timer0");
    
    let args = telemetry_fb::TraceEventArgs {
        timestamp_ns: 1_234_567_890,
        type_: telemetry_fb::TraceEventType::Peripheral,
        id: 42,
        value: 1,
        device_name: Some(name_offset),
    };
    
    let root = telemetry_fb::create_trace_event(&mut builder, &args);
    builder.finish(root, None);
    let payload = builder.finished_data();

    // THIS IS WHAT FIRMWARE STUDIO WILL DO:
    // They will receive `payload` from Zenoh and parse it using the generated FlatBuffer code.
    let parsed_event = flatbuffers::root::<GenTraceEvent>(payload).expect("Failed to parse flatbuffer");
    
    assert_eq!(parsed_event.timestamp_ns(), 1_234_567_890);
    assert_eq!(parsed_event.id(), 42);
    assert_eq!(parsed_event.value(), 1);
    assert_eq!(parsed_event.device_name(), Some("timer0"));
    // Note: TraceEventType casting handles the i8 enum value mapping
    assert_eq!(parsed_event.type_().0, 2); // 2 == Peripheral
}
