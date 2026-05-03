use virtmcu_api::FlatBufferStructExt;
use zenoh::Wait;
extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_void, CStr};
use core::ptr;

use virtmcu_api::ZenohSPIHeader;
use virtmcu_qom::qom::{Object, ObjectClass, TypeInfo};
use virtmcu_qom::ssi::{SSIPeripheral, SSIPeripheralClass, TYPE_SSI_PERIPHERAL};
use virtmcu_qom::timer::{qemu_clock_get_ns, QEMU_CLOCK_VIRTUAL};
use virtmcu_qom::{
    declare_device_type, define_prop_string, define_prop_uint32, define_properties,
    ssi_peripheral_class,
};

/* ── QOM Object ───────────────────────────────────────────────────────────── */

#[repr(C)]
pub struct VirtmcuSPI {
    pub parent_obj: SSIPeripheral,

    /* Properties */
    pub node_id: u32,
    pub transport: *mut c_char,
    pub id: *mut c_char,
    pub router: *mut c_char,

    /* Internal State */
    pub rust_state: *mut VirtmcuSPIBackend,
}

pub struct VirtmcuSPIBackend {
    transport: Arc<dyn virtmcu_api::DataTransport>,
    id: String,
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
}

/* ── Logic ────────────────────────────────────────────────────────────────── */

/// # Safety
/// This function is called by QEMU when an SPI transfer happens. `dev` must be a valid pointer to `VirtmcuSPI`.
#[no_mangle]
pub unsafe extern "C" fn spi_transfer(dev: *mut SSIPeripheral, val: u32) -> u32 {
    let was_locked = virtmcu_qom::sync::Bql::is_held();
    if !was_locked {
        virtmcu_qom::sim_warn!("spi_transfer called without BQL!");
    }

    // SAFETY: dev is a valid pointer to VirtmcuSPI provided by QEMU.
    let s = unsafe { &mut *(dev as *mut VirtmcuSPI) };
    if s.rust_state.is_null() {
        return 0;
    }
    // SAFETY: rust_state is non-null and owned by the device.
    let backend = unsafe { &*s.rust_state };

    // SAFETY: Calling qemu_clock_get_ns is safe under BQL.
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    // SAFETY: dev points to a valid SSIPeripheral.
    let header = unsafe { ZenohSPIHeader::new(now, 0, 4, (*dev).cs, (*dev).cs_index, 0) };

    let mut data = Vec::with_capacity(virtmcu_api::ZENOH_SPI_HEADER_SIZE + 4);
    data.extend_from_slice(header.pack());
    data.extend_from_slice(&val.to_le_bytes());

    // SAFETY: dev points to a valid SSIPeripheral.
    let topic = unsafe { format!("sim/spi/{}/{}", backend.id, (*dev).cs_index) };

    // Release BQL before blocking for query
    let _bql = virtmcu_qom::sync::Bql::temporary_unlock();

    match backend.transport.query(&topic, &data) {
        Ok(payload) => {
            if payload.len() >= 4 {
                u32::from_le_bytes(payload[..4].try_into().unwrap_or_default())
            } else {
                0
            }
        }
        Err(_) => 0,
    }
}

/// # Safety
/// This function is called by QEMU when Chip Select state changes. `dev` must be a valid pointer to `VirtmcuSPI`.
#[no_mangle]
pub unsafe extern "C" fn spi_set_cs(dev: *mut SSIPeripheral, select: bool) -> c_int {
    let _was_locked = virtmcu_qom::sync::Bql::is_held();

    // SAFETY: dev is a valid pointer to VirtmcuSPI provided by QEMU.
    let s = unsafe { &mut *(dev as *mut VirtmcuSPI) };
    if s.rust_state.is_null() {
        return 0;
    }
    // SAFETY: rust_state is non-null and owned by the device.
    let backend = unsafe { &*s.rust_state };

    // SAFETY: Calling qemu_clock_get_ns is safe under BQL.
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    // SAFETY: dev points to a valid SSIPeripheral.
    let header = unsafe { ZenohSPIHeader::new(now, 0, 0, select, (*dev).cs_index, 0) };

    let header_bytes = header.pack();

    // SAFETY: dev points to a valid SSIPeripheral.
    let topic = unsafe { format!("sim/spi/{}/{}/cs", backend.id, (*dev).cs_index) };

    let _ = backend.transport.publish(&topic, header_bytes);

    0
}

/// # Safety
/// This function is called by QEMU to realize the device. `dev` must be a valid pointer to `VirtmcuSPI`.
#[no_mangle]
pub unsafe extern "C" fn spi_realize(dev: *mut SSIPeripheral, errp: *mut *mut c_void) {
    let dev_state = unsafe { &mut (*dev).parent_obj };
    if dev_state.parent_bus.is_null() {
        virtmcu_qom::error_setg!(errp, "spi: device must be attached to an SSI bus\n");
        return;
    }

    let s = unsafe { &mut *(dev as *mut VirtmcuSPI) };

    let id_str = if s.id.is_null() {
        format!("spi{}", s.node_id)
    } else {
        unsafe { CStr::from_ptr(s.id).to_string_lossy().into_owned() }
    };

    let transport_name = if s.transport.is_null() {
        "zenoh".to_owned()
    } else {
        unsafe { CStr::from_ptr(s.transport).to_string_lossy().into_owned() }
    };

    let router_ptr = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };

    let transport: Arc<dyn virtmcu_api::DataTransport> = if transport_name == "unix" {
        let path = if router_ptr.is_null() {
            format!("/tmp/virtmcu-coord-{}.sock", s.node_id)
        } else {
            unsafe { core::ffi::CStr::from_ptr(router_ptr).to_string_lossy().into_owned() }
        };
        match transport_unix::UnixDataTransport::new(&path) {
            Ok(t) => Arc::new(t),
            Err(_) => {
                virtmcu_qom::error_setg!(errp, "spi: failed to open unix socket");
                return;
            }
        }
    } else {
        match unsafe { transport_zenoh::get_or_init_session(router_ptr) } {
            Ok(session) => Arc::new(transport_zenoh::ZenohDataTransport::new(session)),
            Err(_) => {
                virtmcu_qom::error_setg!(errp, "spi: failed to open Zenoh session");
                return;
            }
        }
    };

    let liveliness = if transport_name == "zenoh" {
        match unsafe { transport_zenoh::get_or_init_session(router_ptr) } {
            Ok(session) => {
                let hb_topic = format!("sim/spi/liveliness/{}", s.node_id);
                session.liveliness().declare_token(hb_topic).wait().ok()
            }
            Err(_) => None,
        }
    } else {
        None
    };
    let backend = Box::new(VirtmcuSPIBackend { transport, id: id_str, _liveliness: liveliness });

    s.rust_state = Box::into_raw(backend);
}

/// # Safety
/// This function is called by QEMU when finalizing the device. `obj` must be a valid pointer to `VirtmcuSPI`.
#[no_mangle]
pub unsafe extern "C" fn spi_instance_finalize(obj: *mut Object) {
    let s = unsafe { &mut *(obj as *mut VirtmcuSPI) };
    if !s.rust_state.is_null() {
        unsafe {
            drop(Box::from_raw(s.rust_state));
        }
        s.rust_state = ptr::null_mut();
    }
}

/* ── QOM Boilerplate ──────────────────────────────────────────────────────── */

define_properties!(
    VIRTM_SPI_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), VirtmcuSPI, node_id, 0),
        define_prop_string!(c"transport".as_ptr(), VirtmcuSPI, transport),
        define_prop_string!(c"id".as_ptr(), VirtmcuSPI, id),
        define_prop_string!(c"router".as_ptr(), VirtmcuSPI, router),
    ]
);

/// # Safety
/// This function is called by QEMU to initialize the class. `klass` must be a valid `ObjectClass` pointer.
#[no_mangle]
pub unsafe extern "C" fn spi_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let ssc = ssi_peripheral_class!(klass);
    let dc = unsafe { &mut (*ssc).parent_class };

    unsafe {
        (*ssc).realize = Some(spi_realize);
        (*ssc).transfer = Some(spi_transfer);
        (*ssc).set_cs = Some(spi_set_cs);
        (*ssc).cs_polarity = 1; // SSI_CS_LOW
    }

    dc.user_creatable = true;
    virtmcu_qom::device_class_set_props!(dc, VIRTM_SPI_PROPERTIES);
}

#[used]
static VIRTM_SPI_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"spi".as_ptr(),
    parent: TYPE_SSI_PERIPHERAL,
    instance_size: core::mem::size_of::<VirtmcuSPI>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: Some(spi_instance_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<SSIPeripheralClass>(),
    class_init: Some(spi_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(VIRTM_SPI_TYPE_INIT, VIRTM_SPI_TYPE_INFO);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spi_layout() {
        assert_eq!(
            core::mem::offset_of!(VirtmcuSPI, parent_obj),
            0,
            "SSIPeripheral must be the first field"
        );
    }
}
