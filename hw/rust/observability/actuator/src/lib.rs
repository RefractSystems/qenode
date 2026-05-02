//! Virtmcu actuator device with pluggable transport.
use zenoh::Wait;

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
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
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
            unsafe {
                ptr::copy_nonoverlapping(
                    (s.data.as_ptr().add(idx) as *const u8).add(offset),
                    &raw mut ret as *mut u8,
                    size as usize,
                );
            }
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
            unsafe {
                ptr::copy_nonoverlapping(
                    &raw const val as *const u8,
                    (s.data.as_mut_ptr().add(idx) as *mut u8).add(offset),
                    size as usize,
                );
            }
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

    let router_ptr = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };
    let transport_name = if s.transport.is_null() {
        "zenoh".to_owned()
    } else {
        unsafe { CStr::from_ptr(s.transport).to_string_lossy().into_owned() }
    };

    let prefix = if s.topic_prefix.is_null() {
        "firmware/control".to_owned()
    } else {
        unsafe { CStr::from_ptr(s.topic_prefix).to_string_lossy().into_owned() }
    };

    s.rust_state = actuator_init_internal(s.node_id, transport_name, router_ptr, prefix);
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
        define_prop_string!(c"transport".as_ptr(), VirtmcuActuatorQEMU, transport),
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
    virtmcu_qom::qdev::device_class_set_props_n(dc, VIRTMCU_ACTUATOR_PROPERTIES.as_ptr(), 5);
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
    transport_name: String,
    router: *const c_char,
    topic_prefix: String,
) -> *mut VirtmcuActuatorState {
    let transport: Arc<dyn virtmcu_api::DataTransport> = if transport_name == "unix" {
        let path = if router.is_null() {
            format!("/tmp/virtmcu-coord-{}.sock", { node_id })
        } else {
            unsafe { core::ffi::CStr::from_ptr(router).to_string_lossy().into_owned() }
        };
        match transport_unix::UnixDataTransport::new(&path) {
            Ok(t) => Arc::new(t),
            Err(_) => return ptr::null_mut(),
        }
    } else {
        match unsafe { transport_zenoh::get_or_init_session(router) } {
            Ok(session) => Arc::new(transport_zenoh::ZenohDataTransport::new(session)),
            Err(_) => return ptr::null_mut(),
        }
    };

    let (tx, rx) = bounded(1024);
    let shared = Arc::new(SharedState {
        transport,
        node_id,
        topic_prefix,
        tx_sender: tx,
        drain_cond: Condvar::new(),
        state: Mutex::new(InnerState { running: true, active_vcpu_count: 0 }),
    });

    let bg_thread = start_tx_thread(Arc::clone(&shared), rx);

    let liveliness = if transport_name == "zenoh" {
        match unsafe { transport_zenoh::get_or_init_session(router) } {
            Ok(session) => {
                let hb_topic = format!("sim/actuator/liveliness/{node_id}");
                session.liveliness().declare_token(hb_topic).wait().ok()
            }
            Err(_) => None,
        }
    } else {
        None
    };
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
    let mut payload = Vec::with_capacity(8 + (data_size as usize) * 8);
    payload.extend_from_slice(&vtime_ns.to_le_bytes());
    for val in data.iter().take(data_size as usize) {
        payload.extend_from_slice(&val.to_le_bytes());
    }

    match state.shared.tx_sender.try_send(ActuatorPacket { topic, payload }) {
        Ok(_) | Err(TrySendError::Disconnected(_) | TrySendError::Full(_)) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_actuator_qemu_layout() {
        assert_eq!(
            core::mem::offset_of!(VirtmcuActuatorQEMU, parent_obj),
            0,
            "SysBusDevice must be the first field"
        );
    }
}
