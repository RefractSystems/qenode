sed -i '/pub struct ZenohFrameHeader {/i \
#[repr(u8)]\n\
#[derive(Debug, Copy, Clone, PartialEq, Eq)]\n\
pub enum WiFiFrameType {\n\
    Management = 0,\n\
    Control = 1,\n\
    Data = 2,\n\
}\n\
\n\
impl Default for WiFiFrameType {\n\
    fn default() -> Self { WiFiFrameType::Management }\n\
}\n\
\n\
#[repr(C, packed)]\n\
#[derive(Debug, Copy, Clone, Default)]\n\
pub struct ZenohWiFiHeader {\n\
    pub delivery_vtime_ns: u64,\n\
    pub size: u32,\n\
    pub channel: u16,\n\
    pub rssi: i8,\n\
    pub snr: i8,\n\
    pub frame_type: u8,\n\
    pub _padding: [u8; 3],\n\
}\n\
\n\
const _: () = assert!(\n\
    core::mem::size_of::<ZenohWiFiHeader>() == 20,\n\
    "ZenohWiFiHeader must be exactly 20 bytes"\n\
);\n\
' hw/rust/virtmcu-api/src/lib.rs
