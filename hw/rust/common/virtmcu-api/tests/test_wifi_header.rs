use virtmcu_api::wifi_generated::virtmcu::wifi::{WifiHeader, WifiHeaderArgs};

#[test]
fn test_wifi_header_encode_decode() -> Result<(), String> {
    let mut builder = flatbuffers::FlatBufferBuilder::new();
    let args = WifiHeaderArgs {
        delivery_vtime_ns: 123456789,
        size: 1500,
        channel: 6,
        rssi: -45,
        snr: 20,
        frame_type: 2, // Data
    };

    let header =
        virtmcu_api::wifi_generated::virtmcu::wifi::WifiHeader::create(&mut builder, &args);
    builder.finish(header, None);

    let buf = builder.finished_data();

    // Decode
    let decoded = flatbuffers::root::<WifiHeader>(buf).map_err(|e| format!("{e:?}"))?;
    assert_eq!(decoded.delivery_vtime_ns(), 123456789);
    assert_eq!(decoded.size(), 1500);
    assert_eq!(decoded.channel(), 6);
    assert_eq!(decoded.rssi(), -45);
    assert_eq!(decoded.snr(), 20);
    assert_eq!(decoded.frame_type(), 2);
    Ok(())
}
