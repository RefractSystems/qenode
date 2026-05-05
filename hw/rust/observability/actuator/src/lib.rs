unsafe extern "C" fn allow_set_link(
    _obj: *mut virtmcu_qom::qom::Object,
    _name: *const core::ffi::c_char,
    _val: *mut virtmcu_qom::qom::Object,
    _errp: *mut *mut virtmcu_qom::error::Error,
) {
}
// Virtmcu actuator device with pluggable transport.

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::{c_char, c_uint, c_void, CStr};
use core::ptr;
use core::time::Duration;
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use std::sync::{Condvar, Mutex};
use std::thread::JoinHandle;
use virtmcu_qom::memory::{
    memory_region_init_io, MemoryRegion, MemoryRegionOps, DEVICE_LITTLE_ENDIAN,
};
use virtmcu_qom::qdev::{sysbus_init_mmio, SysBusDevice};
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::sync::Bql;
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties, device_class,
    error_setg,
};

#[repr(C)]
pub struct VirtmcuActuatorQEMU {
    pub parent_obj: SysBusDevice,
    pub mmio: MemoryRegion,

    /* Properties */
    pub node_id: u32,
    pub transport: *mut c_char,
    pub router: *mut c_char,
    pub topic_prefix: *mut c_char,
    pub debug: bool,

    /* Links */
    pub transport_hub: *mut Object,

    /* Registers */
    pub actuator_id: u32,
    pub data_size: u32,
    pub data: [f64; 8],

    /* Rust state */
    pub rust_state: *mut VirtmcuActuatorState,
}

struct ActuatorPacket {
    topic: String,
    payload: Vec<u8>,
}

pub struct VirtmcuActuatorState {
    shared: Arc<SharedState>,
    bg_thread: Option<JoinHandle<()>>,
    pub _liveliness: Option<alloc::boxed::Box<dyn virtmcu_api::LivelinessToken>>,
}

pub struct InnerState {
    running: bool,
    active_vcpu_count: usize,
}

struct SharedState {
    transport: Arc<dyn virtmcu_api::DataTransport>,
    node_id: u32,
    topic_prefix: String,

    tx_sender: Sender<ActuatorPacket>,
    drain_cond: Condvar,
    state: Mutex<InnerState>, // MUTEX_EXCEPTION: used with Condvar for teardown
}

impl Drop for VirtmcuActuatorState {
    fn drop(&mut self) {
        {
            let mut lock =
                self.shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            lock.running = false;
        }

        // Wait for all vCPU threads to drain (bounded: avoids permanent deadlock)
        let mut lock = self.shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while lock.active_vcpu_count > 0 {
            let bql_unlock = Bql::temporary_unlock();
            let (new_lock, timed_out) = self
                .shared
                .drain_cond
                .wait_timeout(lock, Duration::from_secs(30))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            lock = new_lock;
            drop(bql_unlock);
            if timed_out.timed_out() {
                break;
            }
        }

        if let Some(handle) = self.bg_thread.take() {
            let bql_unlock = Bql::temporary_unlock();
            let _ = handle.join();
            drop(bql_unlock);
        }
    }
}

struct VcpuCountGuard<'a>(&'a SharedState);
impl Drop for VcpuCountGuard<'_> {
    fn drop(&mut self) {
        let mut lock = self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        lock.active_vcpu_count = lock.active_vcpu_count.saturating_sub(1);
        if lock.active_vcpu_count == 0 {
            self.0.drain_cond.notify_all();
        }
    }
}

const REG_ACTUATOR_ID: u64 = 0x00;
const REG_DATA_SIZE: u64 = 0x04;
const REG_GO: u64 = 0x08;
const REG_DATA_START: u64 = 0x10;

/// # Safety
/// This function is called by QEMU. opaque must be a valid pointer to VirtmcuActuatorQEMU.
#[no_mangle]
pub unsafe extern "C" fn actuator_read(opaque: *mut c_void, addr: u64, size: c_uint) -> u64 {
    let s = unsafe { &mut *(opaque as *mut VirtmcuActuatorQEMU) };

    if addr == REG_ACTUATOR_ID {
        u64::from(s.actuator_id)
    } else if addr == REG_DATA_SIZE {
        u64::from(s.data_size)
    } else if (REG_DATA_START..REG_DATA_START + 8 * 8).contains(&addr) {
        let idx = ((addr - REG_DATA_START) / 8) as usize;
        let offset = ((addr - REG_DATA_START) % 8) as usize;
        let mut ret: u64 = 0;
        if offset + (size as usize) <= 8 {
            let bytes = s.data[idx].to_le_bytes();
            let mut ret_bytes = [0u8; 8];
            ret_bytes[..size as usize].copy_from_slice(&bytes[offset..offset + size as usize]);
            ret = u64::from_le_bytes(ret_bytes);
        }
        ret
    } else {
        if s.debug {
            virtmcu_qom::sim_warn!("actuator_read: unhandled offset 0x{:x}", addr);
        }
        0
    }
}

/// # Safety
/// This function is called by QEMU. opaque must be a valid pointer to VirtmcuActuatorQEMU.
#[no_mangle]
pub unsafe extern "C" fn actuator_write(opaque: *mut c_void, addr: u64, val: u64, size: c_uint) {
    let s = unsafe { &mut *(opaque as *mut VirtmcuActuatorQEMU) };

    if addr == REG_ACTUATOR_ID {
        s.actuator_id = val as u32;
    } else if addr == REG_DATA_SIZE {
        s.data_size = val as u32;
        if s.data_size > 8 {
            s.data_size = 8;
        }
    } else if addr == REG_GO {
        if val == 1 && !s.rust_state.is_null() {
            let rs = unsafe { &*s.rust_state };
            actuator_publish(rs, s.actuator_id, s.data_size, &s.data);
        }
    } else if (REG_DATA_START..REG_DATA_START + 8 * 8).contains(&addr) {
        let idx = ((addr - REG_DATA_START) / 8) as usize;
        let offset = ((addr - REG_DATA_START) % 8) as usize;
        if offset + (size as usize) <= 8 {
            let val_bytes = val.to_le_bytes();
            let mut data_bytes = s.data[idx].to_le_bytes();
            data_bytes[offset..offset + size as usize].copy_from_slice(&val_bytes[..size as usize]);
            s.data[idx] = f64::from_le_bytes(data_bytes);
        }
    } else if s.debug {
        virtmcu_qom::sim_warn!("actuator_write: unhandled offset 0x{:x} val=0x{:x}", addr, val);
    }
}

static VIRTMCU_ACTUATOR_OPS: MemoryRegionOps = MemoryRegionOps {
    read: Some(actuator_read),
    write: Some(actuator_write),
    read_with_attrs: ptr::null(),
    write_with_attrs: ptr::null(),
    endianness: DEVICE_LITTLE_ENDIAN,
    _padding1: [0; 4],
    valid: virtmcu_qom::memory::MemoryRegionValidRange {
        min_access_size: 4,
        max_access_size: 8,
        unaligned: false,
        _padding: [0; 7],
        accepts: ptr::null(),
    },
    impl_: virtmcu_qom::memory::MemoryRegionImplRange {
        min_access_size: 1,
        max_access_size: 8,
        unaligned: false,
        _padding: [0; 7],
    },
};

/// # Safety
/// This function is called by QEMU to realize the device. dev must be a valid pointer to VirtmcuActuatorQEMU.
#[no_mangle]
pub unsafe extern "C" fn actuator_realize(dev: *mut c_void, errp: *mut *mut c_void) {
    let s = unsafe { &mut *(dev as *mut VirtmcuActuatorQEMU) };

    unsafe {
        memory_region_init_io(
            &raw mut s.mmio,
            dev as *mut Object,
            &raw const VIRTMCU_ACTUATOR_OPS,
            dev,
            c"actuator".as_ptr(),
            0x100,
        );
        sysbus_init_mmio(dev as *mut SysBusDevice, &raw mut s.mmio);
    }

    let prefix = if s.topic_prefix.is_null() {
        "firmware/control".to_owned()
    } else {
        unsafe { CStr::from_ptr(s.topic_prefix).to_string_lossy().into_owned() }
    };

    if s.transport_hub.is_null() {
        error_setg!(errp, "Strict DI violation: transport_hub link is required.");
        return;
    }

    unsafe {
        virtmcu_qom::qom::object_property_set_bool(
            s.transport_hub,
            c"realized".as_ptr(),
            true,
            errp as *mut *mut virtmcu_qom::error::Error,
        );
    }
    let ptr_u64 = unsafe {
        virtmcu_qom::qom::object_property_get_uint(
            s.transport_hub,
            c"transport_ptr".as_ptr(),
            errp as *mut *mut virtmcu_qom::error::Error,
        )
    };
    if ptr_u64 == 0 {
        virtmcu_qom::error_setg!(
            errp,
            "Strict DI violation: failed to acquire transport from hub."
        );
        return;
    }
    let transport_ref =
        unsafe { &*(ptr_u64 as *const alloc::sync::Arc<dyn virtmcu_api::DataTransport>) };
    let transport_arc = alloc::sync::Arc::clone(transport_ref);

    s.rust_state = actuator_init_internal(s.node_id, prefix, transport_arc);
    if s.rust_state.is_null() {
        error_setg!(errp, "actuator: failed to initialize Rust backend");
    }
}

/// # Safety
/// This function is called by QEMU when finalizing the device. obj must be a valid pointer to VirtmcuActuatorQEMU.
#[no_mangle]
pub unsafe extern "C" fn actuator_instance_finalize(obj: *mut Object) {
    let s = unsafe { &mut *(obj as *mut VirtmcuActuatorQEMU) };
    if !s.rust_state.is_null() {
        unsafe {
            drop(Box::from_raw(s.rust_state));
        }
        s.rust_state = ptr::null_mut();
    }
}

/// # Safety
/// This function is called by QEMU on instance initialization. obj must be a valid pointer to VirtmcuActuatorQEMU.
#[no_mangle]
pub unsafe extern "C" fn actuator_instance_init(obj: *mut Object) {
    let s = unsafe { &mut *(obj as *mut VirtmcuActuatorQEMU) };
    s.topic_prefix = ptr::null_mut();
    s.transport = ptr::null_mut();
}

define_properties!(
    VIRTMCU_ACTUATOR_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), VirtmcuActuatorQEMU, node_id, 0),
        define_prop_string!(c"router".as_ptr(), VirtmcuActuatorQEMU, router),
        define_prop_string!(c"topic-prefix".as_ptr(), VirtmcuActuatorQEMU, topic_prefix),
        virtmcu_qom::define_prop_bool!(c"debug".as_ptr(), VirtmcuActuatorQEMU, debug, false),
    ]
);

/// # Safety
/// This function is called by QEMU to initialize the class. klass must be a valid pointer to ObjectClass.
#[no_mangle]
pub unsafe extern "C" fn actuator_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let dc = device_class!(klass);
    unsafe {
        (*dc).realize = Some(actuator_realize);
        (*dc).user_creatable = true;
    }
    virtmcu_qom::device_class_set_props!(dc, VIRTMCU_ACTUATOR_PROPERTIES);

    unsafe {
        virtmcu_qom::qom::object_class_property_add_link(
            klass,
            c"transport".as_ptr(),
            c"virtmcu-transport-hub".as_ptr(),
            core::mem::offset_of!(VirtmcuActuatorQEMU, transport_hub) as isize,
            Some(allow_set_link),
            virtmcu_qom::qom::OBJ_PROP_LINK_STRONG,
        );
    }
}

#[used]
static VIRTMCU_ACTUATOR_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"actuator".as_ptr(),
    parent: c"sys-bus-device".as_ptr(),
    instance_size: core::mem::size_of::<VirtmcuActuatorQEMU>(),
    instance_align: 0,
    instance_init: Some(actuator_instance_init),
    instance_post_init: None,
    instance_finalize: Some(actuator_instance_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>(),
    class_init: Some(actuator_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(VIRTMCU_ACTUATOR_TYPE_INIT, VIRTMCU_ACTUATOR_TYPE_INFO);

/* ── Internal Logic ───────────────────────────────────────────────────────── */

fn start_tx_thread(shared: Arc<SharedState>, rx: Receiver<ActuatorPacket>) -> JoinHandle<()> {
    std::thread::spawn(move || loop {
        {
            let lock = shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if !lock.running && rx.is_empty() {
                break;
            }
        }
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(packet) => {
                let _ = shared.transport.publish(&packet.topic, &packet.payload);
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    })
}

fn actuator_init_internal(
    node_id: u32,
    topic_prefix: String,
    transport: Arc<dyn virtmcu_api::DataTransport>,
) -> *mut VirtmcuActuatorState {
    let (tx, rx) = bounded(1024);
    let shared = Arc::new(SharedState {
        transport: Arc::clone(&transport),
        node_id,
        topic_prefix,
        tx_sender: tx,
        drain_cond: Condvar::new(),
        state: Mutex::new(InnerState { running: true, active_vcpu_count: 0 }),
    });

    let bg_thread = start_tx_thread(Arc::clone(&shared), rx);

    let hb_topic = format!("sim/actuator/liveliness/{node_id}");
    let liveliness = transport.declare_liveliness(&hb_topic);

    Box::into_raw(Box::new(VirtmcuActuatorState {
        shared,
        bg_thread: Some(bg_thread),
        _liveliness: liveliness,
    }))
}

fn actuator_publish(
    state: &VirtmcuActuatorState,
    actuator_id: u32,
    data_size: u32,
    data: &[f64; 8],
) {
    {
        let mut lock = state.shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !lock.running {
            return;
        }
        lock.active_vcpu_count += 1;
    }
    let _guard = VcpuCountGuard(&state.shared);

    let vtime_ns =
        unsafe { virtmcu_qom::timer::qemu_clock_get_ns(virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL) }
            as u64;

    let topic = format!("{}/{}/{}", state.shared.topic_prefix, state.shared.node_id, actuator_id);
    let mut data_payload = Vec::with_capacity((data_size as usize) * 8);
    for val in data.iter().take(data_size as usize) {
        data_payload.extend_from_slice(&val.to_le_bytes());
    }
    let payload = virtmcu_api::encode_frame(vtime_ns, 0, &data_payload);

    match state.shared.tx_sender.try_send(ActuatorPacket { topic, payload }) {
        Ok(_) | Err(TrySendError::Disconnected(_) | TrySendError::Full(_)) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Dummy symbols to satisfy the linker during cargo test
    #[no_mangle]
    pub extern "C" fn qemu_clock_get_ns(_: i32) -> i64 {
        0
    }
    #[no_mangle]
    pub extern "C" fn register_dso_module_init(_: *const c_void, _: i32) {}
    #[no_mangle]
    pub extern "C" fn type_register_static(_: *const c_void) {}
    #[no_mangle]
    pub extern "C" fn object_class_dynamic_cast_assert(
        _: *mut c_void,
        _: *const c_char,
        _: *const c_char,
        _: i32,
        _: *const c_char,
    ) -> *mut c_void {
        ptr::null_mut()
    }
    #[no_mangle]
    pub extern "C" fn device_class_set_props_n(_: *mut c_void, _: *const c_void) {}
    #[no_mangle]
    pub extern "C" fn object_class_property_add_link(
        _: *mut c_void,
        _: *const c_char,
        _: *const c_char,
        _: isize,
        _: Option<unsafe extern "C" fn()>,
        _: i32,
    ) {
    }
    #[no_mangle]
    pub extern "C" fn memory_region_init_io(
        _: *mut c_void,
        _: *mut c_void,
        _: *const c_void,
        _: *mut c_void,
        _: *const c_char,
        _: u64,
    ) {
    }
    #[no_mangle]
    pub extern "C" fn sysbus_init_mmio(_: *mut c_void, _: *mut c_void) {}
    #[no_mangle]
    pub extern "C" fn object_property_set_bool(
        _: *mut c_void,
        _: *const c_char,
        _: bool,
        _: *mut *mut c_void,
    ) {
    }
    #[no_mangle]
    pub extern "C" fn object_property_get_uint(
        _: *mut c_void,
        _: *const c_char,
        _: *mut *mut c_void,
    ) -> u64 {
        0
    }
    #[no_mangle]
    pub static mut qdev_prop_uint32: [u8; 1] = [0];
    #[no_mangle]
    pub static mut qdev_prop_string: [u8; 1] = [0];
    #[no_mangle]
    pub static mut qdev_prop_bool: [u8; 1] = [0];
    #[no_mangle]
    pub extern "C" fn virtmcu_is_bql_locked() -> bool {
        false
    }
    #[no_mangle]
    pub extern "C" fn virtmcu_safe_bql_unlock() {}
    #[no_mangle]
    pub extern "C" fn virtmcu_safe_bql_force_lock() {}
    #[no_mangle]
    pub unsafe extern "C" fn error_setg_internal(
        _: *mut *mut c_void,
        _: *const c_char,
        _: i32,
        _: *const c_char,
        _: *const c_char,
    ) {
    }

    #[test]
    fn test_actuator_qemu_layout() {
        assert_eq!(
            core::mem::offset_of!(VirtmcuActuatorQEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }

    #[test]
    fn test_actuator_mmio_access() {
        let mut device = VirtmcuActuatorQEMU {
            parent_obj: unsafe { core::mem::zeroed() },
            mmio: unsafe { core::mem::zeroed() },
            node_id: 1,
            transport: ptr::null_mut(),
            router: ptr::null_mut(),
            topic_prefix: ptr::null_mut(),
            debug: false,
            transport_hub: ptr::null_mut(),
            actuator_id: 0x1234,
            data_size: 0,
            data: [0.0; 8],
            rust_state: ptr::null_mut(),
        };
        let opaque = &mut device as *mut _ as *mut c_void;

        // Test reading fixed registers
        assert_eq!(unsafe { actuator_read(opaque, REG_ACTUATOR_ID, 4) }, 0x1234);
        assert_eq!(unsafe { actuator_read(opaque, REG_DATA_SIZE, 4) }, 0);

        // Test writing fixed registers
        unsafe { actuator_write(opaque, REG_DATA_SIZE, 5, 4) };
        assert_eq!(device.data_size, 5);

        // Test data array access (full u64/f64)
        let val: f64 = 1.23456789;
        let val_u64 = u64::from_le_bytes(val.to_le_bytes());
        unsafe { actuator_write(opaque, REG_DATA_START, val_u64, 8) };
        assert_eq!(device.data[0], val);
        assert_eq!(unsafe { actuator_read(opaque, REG_DATA_START, 8) }, val_u64);

        // Test partial access (offset 4, size 4)
        let val2: u32 = 0xDEADBEEF;
        unsafe { actuator_write(opaque, REG_DATA_START + 4, val2 as u64, 4) };
        let data_bytes = device.data[0].to_le_bytes();
        let high_bytes =
            u32::from_le_bytes([data_bytes[4], data_bytes[5], data_bytes[6], data_bytes[7]]);
        assert_eq!(high_bytes, val2);
        assert_eq!(unsafe { actuator_read(opaque, REG_DATA_START + 4, 4) }, val2 as u64);

        // Test unaligned/partial access (offset 1, size 4)
        let val3: u32 = 0x11223344;
        unsafe { actuator_write(opaque, REG_DATA_START + 1, val3 as u64, 4) };
        let data_bytes = device.data[0].to_le_bytes();
        let partial_bytes =
            u32::from_le_bytes([data_bytes[1], data_bytes[2], data_bytes[3], data_bytes[4]]);
        assert_eq!(partial_bytes, val3);
        assert_eq!(unsafe { actuator_read(opaque, REG_DATA_START + 1, 4) }, val3 as u64);
    }
}
