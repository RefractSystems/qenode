use core::ffi::{c_char, c_int, c_void};

#[repr(C)]
pub struct NetClientState {
    pub info: *const NetClientInfo,
    _opaque: [u8; 376 - 8], // Pad to 376 bytes
}

pub type NetCanReceive = unsafe extern "C" fn(nc: *mut NetClientState) -> bool;
pub type NetReceive =
    unsafe extern "C" fn(nc: *mut NetClientState, buf: *const u8, size: usize) -> isize;
pub type NetCleanup = unsafe extern "C" fn(nc: *mut NetClientState);

#[repr(C)]
pub struct NetClientInfo {
    pub type_id: i32,
    pub size: usize,
    pub receive: Option<NetReceive>,
    pub receive_raw: *mut c_void,
    pub receive_iov: *mut c_void,
    pub cleanup: Option<NetCleanup>,
    pub can_receive: Option<NetCanReceive>,
    pub _opaque: [u8; 208 - 56], // Pad to 208 bytes (56 bytes of known fields including padding)
}

#[repr(C)]
pub struct Netdev {
    pub type_: c_int,
    pub u: NetdevUnion,
}

#[repr(C)]
pub union NetdevUnion {
    pub zenoh: NetdevZenohOptions,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct NetdevZenohOptions {
    pub node: *mut c_char,
    pub router: *mut c_char,
    pub topic: *mut c_char,
}

pub const NET_CLIENT_DRIVER_ZENOH: i32 = 14;

extern "C" {
    pub fn qemu_new_net_client(
        info: *const NetClientInfo,
        peer: *mut NetClientState,
        model: *const c_char,
        name: *const c_char,
    ) -> *mut NetClientState;

    pub fn qemu_send_packet(nc: *mut NetClientState, buf: *const u8, size: usize);

    pub static mut virtmcu_zenoh_netdev_hook: Option<
        unsafe extern "C" fn(
            netdev: *const Netdev,
            name: *const c_char,
            peer: *mut NetClientState,
            errp: *mut *mut crate::error::Error,
        ) -> c_int,
    >;
}

// const _: () = assert!(core::mem::size_of::<NetClientState>() == 376);
// const _: () = assert!(core::mem::size_of::<NetClientInfo>() == 208);

unsafe impl Sync for NetClientInfo {}
