use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(if std::env::var("MIRIFLAGS").is_ok() || cfg!(miri) { 1 } else { 256 }))]
    #[test]
    #[allow(deprecated)]
    fn test_fuzz_legacy_frame_parsing(data in prop::collection::vec(any::<u8>(), 0..1024)) {
        // RFC-0042: ZenohFrameHeader is gone. Fuzz the legacy decode_frame shim instead.
        let _ = virtmcu_wire::decode_frame(&data);
    }
}

use virtmcu_wire::wifi_generated::virtmcu::wifi::WifiHeader;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(if std::env::var("MIRIFLAGS").is_ok() || cfg!(miri) { 1 } else { 256 }))]
    #[test]
    fn test_fuzz_wifi_header_parsing(data in prop::collection::vec(any::<u8>(), 0..1024)) {
        if let Ok(decoded) = flatbuffers::root::<WifiHeader>(&data) {
            let _vtime = decoded.delivery_vtime_ns();
            let _size = decoded.size();
            let _channel = decoded.channel();
            let _rssi = decoded.rssi();
            let _snr = decoded.snr();
            let _type = decoded.frame_type();
        }
    }
}
