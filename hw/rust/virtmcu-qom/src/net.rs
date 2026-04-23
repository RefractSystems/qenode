use core::ffi::{c_char, c_int, c_void};

#[repr(C)]
/// A struct
pub struct NetClientState {
    /// A struct field
    pub info: *const NetClientInfo,
    _opaque: [u8; 376 - 8], // Pad to 376 bytes
}

/// A type alias
pub type NetCanReceive = unsafe extern "C" fn(nc: *mut NetClientState) -> bool;
/// A type alias
pub type NetReceive =
    unsafe extern "C" fn(nc: *mut NetClientState, buf: *const u8, size: usize) -> isize;
/// A type alias
pub type NetCleanup = unsafe extern "C" fn(nc: *mut NetClientState);

#[repr(C)]
/// A struct
pub struct NetClientInfo {
    /// A struct field
    pub type_id: i32,
    /// A struct field
    pub size: usize,
    /// A struct field
    pub receive: Option<NetReceive>,
    /// A struct field
    pub receive_raw: *mut c_void,
    /// A struct field
    pub receive_iov: *mut c_void,
    /// A struct field
    pub cleanup: Option<NetCleanup>,
    /// A struct field
    pub can_receive: Option<NetCanReceive>,
    /// A struct field
    pub _opaque: [u8; 208 - 56], // Pad to 208 bytes (56 bytes of known fields including padding)
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
    pub zenoh: NetdevZenohOptions,
}

#[repr(C)]
#[derive(Copy, Clone)]
/// A struct
pub struct NetdevZenohOptions {
    /// A struct field
    pub node: *mut c_char,
    /// A struct field
    pub router: *mut c_char,
    /// A struct field
    pub topic: *mut c_char,
}

/// A constant
pub const NET_CLIENT_DRIVER_ZENOH: i32 = 14;

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
pub struct CanBusClientState {
    /// A struct field
    pub info: *mut CanBusClientInfo,
    /// A struct field
    pub bus: *mut c_void,
    /// A struct field
    pub link_down: c_int,
    /// A struct field
    pub next: [*mut c_void; 2],
    /// A struct field
    pub peer: *mut CanBusClientState,
    /// A struct field
    pub model: *mut c_char,
    /// A struct field
    pub name: *mut c_char,
    /// A struct field
    pub destructor: Option<unsafe extern "C" fn(client: *mut CanBusClientState)>,
    /// A struct field
    pub fd_mode: bool,
    /// A struct field
    pub _padding: [u8; 7], // 80 bytes
}

#[repr(C)]
/// A struct
pub struct CanHostState {
    /// A struct field
    pub oc: crate::qom::Object,
    /// A struct field
    pub bus: *mut c_void,
    /// A struct field
    pub bus_client: CanBusClientState,
}

#[repr(C)]
/// A struct
pub struct CanHostClass {
    /// A struct field
    pub oc: crate::qom::ObjectClass,
    /// A struct field
    pub connect:
        Option<unsafe extern "C" fn(ch: *mut CanHostState, errp: *mut *mut crate::error::Error)>,
    /// A struct field
    pub disconnect: Option<unsafe extern "C" fn(ch: *mut CanHostState)>,
}

extern "C" {
    /// A function
    pub fn qemu_new_net_client(
        info: *const NetClientInfo,
        peer: *mut NetClientState,
        model: *const c_char,
        name: *const c_char,
    ) -> *mut NetClientState;

    /// A function
    pub fn qemu_send_packet(nc: *mut NetClientState, buf: *const u8, size: usize);

    /// A function
    pub fn qemu_can_receive_packet(nc: *mut NetClientState) -> bool;

    /// A static
    pub static mut virtmcu_zenoh_netdev_hook: Option<
        unsafe extern "C" fn(
            netdev: *const Netdev,
            name: *const c_char,
            peer: *mut NetClientState,
            errp: *mut *mut crate::error::Error,
        ) -> c_int,
    >;

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

unsafe impl Sync for NetClientInfo {}
unsafe impl Sync for CanBusClientInfo {}
