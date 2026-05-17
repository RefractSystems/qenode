use core::mem::size_of;
use proptest::prelude::*;
use virtmcu_wire::ZenohFrameHeader;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(if std::env::var("MIRIFLAGS").is_ok() || cfg!(miri) { 1 } else { 256 }))]
    #[test]
    fn test_fuzz_netdev_header_parsing(data in prop::collection::vec(any::<u8>(), 0..1024)) {
        if data.len() >= size_of::<ZenohFrameHeader>() {
            let mut header = ZenohFrameHeader::default();
            header.0.copy_from_slice(&data[..size_of::<ZenohFrameHeader>()]);
            // Ensure no panic
            let _ = &data[size_of::<ZenohFrameHeader>()..];
            let _ = header.delivery_vtime_ns();
            let _ = header.sequence_number();
            let _ = header.size();
        }
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
