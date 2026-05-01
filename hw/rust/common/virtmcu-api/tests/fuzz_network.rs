#![allow(clippy::undocumented_unsafe_blocks)]
use core::mem::size_of;
use proptest::prelude::*;
use virtmcu_api::ZenohFrameHeader;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(if std::env::var("MIRIFLAGS").is_ok() || cfg!(miri) { 1 } else { 256 }))]
    #[test]
    fn test_fuzz_netdev_header_parsing(data in prop::collection::vec(any::<u8>(), 0..1024)) {
        if data.len() >= size_of::<ZenohFrameHeader>() {
            let mut header = ZenohFrameHeader::default();
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    core::ptr::from_mut(&mut header).cast::<u8>(),
                    size_of::<ZenohFrameHeader>(),
                );
            }
            // Ensure no panic
            let _ = &data[size_of::<ZenohFrameHeader>()..];
            let _ = header.delivery_vtime_ns();
            let _ = header.sequence_number();
            let _ = header.size();
        }
    }
}

use virtmcu_api::wifi_generated::virtmcu::wifi::WifiHeader;

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
