use core::ffi::{c_char, c_void};

#[repr(C)]
pub struct NetClientState {
    _opaque: [u8; 0],
}

pub type NetCanReceive = extern "C" fn(nc: *mut NetClientState) -> bool;
pub type NetReceive = extern "C" fn(nc: *mut NetClientState, buf: *const u8, size: usize) -> isize;
pub type NetCleanup = extern "C" fn(nc: *mut NetClientState);

#[repr(C)]
pub struct NetClientInfo {
    pub type_id: i32,
    pub size: usize,
    pub receive: *mut c_void,
    pub receive_raw: *mut c_void,
    pub receive_iov: *mut c_void,
    pub cleanup: *mut c_void,
    pub can_receive: *mut c_void,
    pub query_rx_filter: *mut c_void,
    pub announce: *mut c_void,
    pub set_vnet_hdr_len: *mut c_void,
    pub set_vnet_le: *mut c_void,
    pub set_vnet_be: *mut c_void,
    pub get_vnet_hdr_len: *mut c_void,
    pub set_offload: *mut c_void,
    pub has_ufo: *mut c_void,
    pub has_vnet_hdr: *mut c_void,
    pub has_vnet_hdr_len: *mut c_void,
    pub using_vnet_hdr: *mut c_void,
}

extern "C" {
    pub fn qemu_new_net_client(
        info: *const NetClientInfo,
        peer: *mut NetClientState,
        model: *const c_char,
        name: *const c_char,
    ) -> *mut NetClientState;
    
    pub fn qemu_send_packet(nc: *mut NetClientState, buf: *const u8, size: usize);
}
