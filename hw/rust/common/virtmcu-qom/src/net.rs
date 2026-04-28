#![allow(clippy::panic, clippy::unimplemented)]
#![allow(clippy::missing_safety_doc, clippy::undocumented_unsafe_blocks)]
use core::ffi::{c_char, c_int, c_void};

/// Size of NetClientState struct in QEMU (verified via check-ffi.py / pahole)
pub const QEMU_NET_CLIENT_STATE_SIZE: usize = 376;
/// Size of NetClientInfo struct in QEMU (verified via check-ffi.py / pahole)
pub const QEMU_NET_CLIENT_INFO_SIZE: usize = 208;
/// Size of Netdev struct in QEMU (verified via check-ffi.py / pahole)
pub const QEMU_NETDEV_SIZE: usize = 240;
/// Size of NetdevUnion in QEMU (verified via check-ffi.py / pahole)
pub const QEMU_NETDEV_UNION_SIZE: usize = 224;

#[repr(C)]
/// A struct
pub struct NetClientState {
    /// A struct field
    pub info: *const NetClientInfo,
    _opaque: [u8; QEMU_NET_CLIENT_STATE_SIZE - 8], // Pad to QEMU_NET_CLIENT_STATE_SIZE bytes
}

/// A type alias
pub type NetClientCleanup = Option<unsafe extern "C" fn(nc: *mut NetClientState)>;

/// A type alias
pub type NetClientReceive =
    Option<unsafe extern "C" fn(nc: *mut NetClientState, buf: *const u8, size: usize) -> isize>;

/// A type alias
pub type NetClientCanReceive = Option<unsafe extern "C" fn(nc: *mut NetClientState) -> bool>;

#[repr(C)]
/// A struct
pub struct NetClientInfo {
    /// A struct field
    pub type_id: i32,
    /// A struct field
    pub size: usize,
    /// A struct field
    pub receive: NetClientReceive,
    /// A struct field
    pub receive_raw: Option<unsafe extern "C" fn()>,
    /// A struct field
    pub receive_iov: Option<unsafe extern "C" fn()>,
    /// A struct field
    pub can_receive: NetClientCanReceive,
    /// A struct field
    pub cleanup: NetClientCleanup,
    /// A struct field
    pub _opaque: [u8; QEMU_NET_CLIENT_INFO_SIZE - 56], // Pad to QEMU_NET_CLIENT_INFO_SIZE bytes
}

#[repr(C)]
/// A struct
pub struct Netdev {
    /// A struct field
    pub id: *mut c_char,
    /// A struct field
    pub type_: c_int,
    /// A struct field
    pub u: NetdevUnion,
}

#[repr(C)]
/// A union
pub union NetdevUnion {
    /// A struct field
    pub virtmcu: NetdevVirtmcuOptions,
    /// Padding to match QEMU's Netdev size
    pub _padding: [u8; QEMU_NETDEV_UNION_SIZE],
}

#[repr(C)]
#[derive(Copy, Clone)]
/// A struct
pub struct NetdevVirtmcuOptions {
    /// A struct field
    pub node: *mut c_char,
    /// A struct field
    pub transport: *mut c_char,
    /// A struct field
    pub router: *mut c_char,
    /// A struct field
    pub topic: *mut c_char,
    /// A struct field
    pub has_max_backlog: bool,
    _pad: [u8; 7],
    /// A struct field
    pub max_backlog: u64,
}

/// A constant
pub const NET_CLIENT_DRIVER_VIRTMCU: i32 = 14;

#[repr(C, align(8))]
#[derive(Copy, Clone, Debug)]
/// A struct
pub struct QemuCanFrame {
    /// A struct field
    pub can_id: u32,
    /// A struct field
    pub can_dlc: u8,
    /// A struct field
    pub flags: u8,
    /// A struct field
    pub _padding: [u8; 2],
    /// A struct field
    pub data: [u8; 64],
}

#[repr(C)]
/// A struct
pub struct CanBusClientState {
    /// A struct field
    pub info: *mut CanBusClientInfo,
    /// A struct field
    pub bus: *mut c_void, // CanBusState*
    /// A struct field
    pub peer: *mut c_void,
    /// A struct field
    pub next: *mut CanBusClientState,
    /// A struct field
    pub receive_filter: Option<unsafe extern "C" fn()>,
    /// A struct field
    pub opaque: *mut c_void,
}

#[repr(C)]
/// A struct
pub struct CanBusClientInfo {
    /// A struct field
    pub can_receive: Option<unsafe extern "C" fn(client: *mut CanBusClientState) -> bool>,
    /// A struct field
    pub receive: Option<
        unsafe extern "C" fn(
            client: *mut CanBusClientState,
            frames: *const QemuCanFrame,
            frames_cnt: usize,
        ) -> isize,
    >,
}

#[repr(C)]
/// A struct
pub struct CanHostState {
    /// A struct field
    pub parent: crate::qom::Object,
    /// A struct field
    pub bus: *mut c_void, // CanBusState*
    /// A struct field
    pub bus_client: CanBusClientState,
}

#[repr(C)]
/// A struct
pub struct CanHostClass {
    /// A struct field
    pub parent_class: crate::qom::ObjectClass,
    /// A struct field
    pub connect:
        Option<unsafe extern "C" fn(ch: *mut CanHostState, errp: *mut *mut crate::error::Error)>,
    /// A struct field
    pub disconnect: Option<unsafe extern "C" fn(ch: *mut CanHostState)>,
}

#[cfg(not(any(test, miri)))]
extern "C" {
    /// A function
    pub fn qemu_new_net_client(
        info: *const NetClientInfo,
        peer: *mut NetClientState,
        model: *const c_char,
        name: *const c_char,
    ) -> *mut NetClientState;

    /// A function
    pub fn qemu_can_receive_packet(nc: *mut NetClientState) -> bool;
    /// A function
    pub fn qemu_send_packet(nc: *mut NetClientState, buf: *const u8, size: usize) -> isize;

    /// A function
    pub fn can_bus_insert_client(bus: *mut c_void, client: *mut CanBusClientState) -> c_int;
    /// A function
    pub fn can_bus_remove_client(client: *mut CanBusClientState) -> c_int;
    /// A function
    pub fn can_bus_client_send(
        client: *mut CanBusClientState,
        frames: *const QemuCanFrame,
        frames_cnt: usize,
    ) -> isize;
}

// Miri and generic test stubs
#[cfg(any(test, miri))]
mod stubs {
    use super::*;

    /// Stub
    #[no_mangle]
    pub unsafe extern "C" fn qemu_new_net_client(
        _info: *const NetClientInfo,
        _peer: *mut NetClientState,
        _model: *const c_char,
        _name: *const c_char,
    ) -> *mut NetClientState {
        unimplemented!("Stub for qemu_new_net_client")
    }

    /// Stub
    #[no_mangle]
    pub unsafe extern "C" fn qemu_can_receive_packet(_nc: *mut NetClientState) -> bool {
        unimplemented!("Stub for qemu_can_receive_packet")
    }

    /// Stub
    #[no_mangle]
    pub unsafe extern "C" fn qemu_send_packet(
        _nc: *mut NetClientState,
        _buf: *const u8,
        _size: usize,
    ) -> isize {
        unimplemented!("Stub for qemu_send_packet")
    }

    /// Stub
    #[no_mangle]
    pub unsafe extern "C" fn can_bus_insert_client(
        _bus: *mut c_void,
        _client: *mut CanBusClientState,
    ) -> c_int {
        unimplemented!("Stub for can_bus_insert_client")
    }

    /// Stub
    #[no_mangle]
    pub unsafe extern "C" fn can_bus_remove_client(_client: *mut CanBusClientState) -> c_int {
        unimplemented!("Stub for can_bus_remove_client")
    }

    /// Stub
    #[no_mangle]
    pub unsafe extern "C" fn can_bus_client_send(
        _client: *mut CanBusClientState,
        _frames: *const QemuCanFrame,
        _frames_cnt: usize,
    ) -> isize {
        unimplemented!("Stub for can_bus_client_send")
    }

    /// Stub
    #[no_mangle]
    pub static mut virtmcu_netdev_hook: Option<
        unsafe extern "C" fn(
            netdev: *const Netdev,
            name: *const c_char,
            peer: *mut NetClientState,
            errp: *mut *mut crate::error::Error,
        ) -> c_int,
    > = None;
}

#[cfg(any(test, miri))]
pub use stubs::*;

#[cfg(not(any(test, miri)))]
extern "C" {
    /// A function
    pub static mut virtmcu_netdev_hook: Option<
        unsafe extern "C" fn(
            netdev: *const Netdev,
            name: *const c_char,
            peer: *mut NetClientState,
            errp: *mut *mut crate::error::Error,
        ) -> c_int,
    >;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_net_client_layout() {
        assert!(core::mem::offset_of!(NetClientState, info) == 0);
        assert!(core::mem::size_of::<NetClientState>() == QEMU_NET_CLIENT_STATE_SIZE);
    }

    #[test]
    fn test_net_client_info_layout() {
        assert!(core::mem::offset_of!(NetClientInfo, type_id) == 0);
        assert!(core::mem::offset_of!(NetClientInfo, size) == 8);
        assert!(core::mem::offset_of!(NetClientInfo, receive) == 16);
        assert!(core::mem::size_of::<NetClientInfo>() == QEMU_NET_CLIENT_INFO_SIZE);
    }

    #[test]
    fn test_netdev_layout() {
        assert!(core::mem::offset_of!(Netdev, id) == 0);
        assert!(core::mem::offset_of!(Netdev, type_) == 8);
        assert!(core::mem::offset_of!(Netdev, u) == 16);
        assert!(core::mem::size_of::<Netdev>() == QEMU_NETDEV_SIZE);
    }

    #[test]
    fn test_netdev_virtmcu_options_layout() {
        assert!(core::mem::offset_of!(NetdevVirtmcuOptions, node) == 0);
        assert!(core::mem::offset_of!(NetdevVirtmcuOptions, transport) == 8);
        assert!(core::mem::offset_of!(NetdevVirtmcuOptions, router) == 16);
        assert!(core::mem::offset_of!(NetdevVirtmcuOptions, topic) == 24);
        assert!(core::mem::offset_of!(NetdevVirtmcuOptions, has_max_backlog) == 32);
        assert!(core::mem::offset_of!(NetdevVirtmcuOptions, max_backlog) == 40);
        assert!(core::mem::size_of::<NetdevVirtmcuOptions>() == 48);
    }
}
