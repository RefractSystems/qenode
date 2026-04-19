use proptest::prelude::*;
use std::mem::size_of;
use virtmcu_api::ZenohFrameHeader;

proptest! {
    #[test]
    fn test_fuzz_netdev_header_parsing(data in prop::collection::vec(any::<u8>(), 0..1024)) {
        if data.len() >= size_of::<ZenohFrameHeader>() {
            let mut header = ZenohFrameHeader::default();
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    &mut header as *mut _ as *mut u8,
                    size_of::<ZenohFrameHeader>(),
                );
            }
            // Ensure no panic
            let _payload = &data[size_of::<ZenohFrameHeader>()..];
            let _vtime = header.delivery_vtime_ns;
            let _size = header.size;
        }
    }
}

use virtmcu_api::wifi_generated::virtmcu::wifi::WifiHeader;

proptest! {
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
