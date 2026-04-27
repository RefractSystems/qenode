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
    ssi_peripheral_class, vlog,
};
use zenoh::Session;
use zenoh::Wait;

/* ── QOM Object ───────────────────────────────────────────────────────────── */

#[repr(C)]
pub struct ZenohSPI {
    pub parent_obj: SSIPeripheral,

    /* Properties */
    pub node_id: u32,
    pub id: *mut c_char,
    pub router: *mut c_char,

    /* Internal State */
    pub rust_state: *mut ZenohSPIBackend,
}

pub struct ZenohSPIBackend {
    session: Arc<Session>,
    id: String,
}

/* ── Logic ────────────────────────────────────────────────────────────────── */

/// # Safety
/// This function is called by QEMU when an SPI transfer happens. `dev` must be a valid pointer to `ZenohSPI`.
#[no_mangle]
pub unsafe extern "C" fn zenoh_spi_transfer(dev: *mut SSIPeripheral, val: u32) -> u32 {
    let was_locked = virtmcu_qom::sync::Bql::is_held();
    if !was_locked {
        vlog!("[zenoh-spi] WARNING: zenoh_spi_transfer called without BQL!\n");
    }

    // SAFETY: dev is a valid pointer to ZenohSPI provided by QEMU.
    let s = unsafe { &mut *(dev as *mut ZenohSPI) };
    if s.rust_state.is_null() {
        return 0;
    }
    // SAFETY: rust_state is non-null and owned by the device.
    let backend = unsafe { &*s.rust_state };

    // SAFETY: Calling qemu_clock_get_ns is safe under BQL.
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    // SAFETY: dev points to a valid SSIPeripheral.
    let header = unsafe {
        ZenohSPIHeader {
            delivery_vtime_ns: now,
            sequence_number: 0,
            size: 4,
            cs: (*dev).cs,
            cs_index: (*dev).cs_index,
            _padding: [0; 2],
        }
    };

    let mut data = Vec::with_capacity(16 + 4);
    data.extend_from_slice(&header.pack());
    data.extend_from_slice(&val.to_le_bytes());

    // SAFETY: dev points to a valid SSIPeripheral.
    let topic = unsafe { format!("sim/spi/{}/{}", backend.id, (*dev).cs_index) };

    // Release BQL before blocking for Zenoh query
    let _bql = virtmcu_qom::sync::Bql::temporary_unlock();

    // In a real implementation, we might want to use a subscriber for async,
    // but for bit-perfect multi-node SPI, a query is more deterministic.
    let mut received_val = 0u32;
    let mut got_reply = false;

    let replies = backend.session.get(&topic).payload(data).wait();
    if let Ok(replies) = replies {
        while let Ok(reply) = replies.recv() {
            if let Ok(sample) = reply.result() {
                let payload = sample.payload().to_bytes();
                if payload.len() >= 4 {
                    received_val = u32::from_le_bytes(payload[..4].try_into().unwrap_or_default());
                    got_reply = true;
                    break;
                }
            }
        }
    }

    if !got_reply {
        /*
        vlog!(
            "[zenoh-spi] WARNING: No reply for SPI transfer on {}\n",
            topic
        );
        */
    }

    received_val
}

/// # Safety
/// This function is called by QEMU when Chip Select state changes. `dev` must be a valid pointer to `ZenohSPI`.
#[no_mangle]
pub unsafe extern "C" fn zenoh_spi_set_cs(dev: *mut SSIPeripheral, select: bool) -> c_int {
    let _was_locked = virtmcu_qom::sync::Bql::is_held();

    // SAFETY: dev is a valid pointer to ZenohSPI provided by QEMU.
    let s = unsafe { &mut *(dev as *mut ZenohSPI) };
    if s.rust_state.is_null() {
        return 0;
    }
    // SAFETY: rust_state is non-null and owned by the device.
    let backend = unsafe { &*s.rust_state };

    // SAFETY: Calling qemu_clock_get_ns is safe under BQL.
    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    // SAFETY: dev points to a valid SSIPeripheral.
    let header = unsafe {
        ZenohSPIHeader {
            delivery_vtime_ns: now,
            sequence_number: 0,
            size: 0,
            cs: select,
            cs_index: (*dev).cs_index,
            _padding: [0; 2],
        }
    };

    let header_bytes = header.pack();

    // SAFETY: dev points to a valid SSIPeripheral.
    let topic = unsafe { format!("sim/spi/{}/{}/cs", backend.id, (*dev).cs_index) };

    // Release BQL before blocking for Zenoh put
    let _bql = virtmcu_qom::sync::Bql::temporary_unlock();
    let _ = backend.session.put(topic, header_bytes.to_vec()).wait();

    0
}

/// # Safety
/// This function is called by QEMU to realize the device. `dev` must be a valid pointer to `ZenohSPI`.
#[no_mangle]
pub unsafe extern "C" fn zenoh_spi_realize(dev: *mut SSIPeripheral, errp: *mut *mut c_void) {
    // Task 21.7.3: SPI/UART Topology Runtime Assertions
    // Verify that this device is indeed attached to an SSI bus
    // SAFETY: dev points to a valid SSIPeripheral.
    let dev_state = unsafe { &mut (*dev).parent_obj };
    if dev_state.parent_bus.is_null() {
        // SAFETY: errp is a valid error pointer.
        virtmcu_qom::error_setg!(errp, "zenoh-spi: device must be attached to an SSI bus\n");
        return;
    }

    // SAFETY: dev is a valid pointer to ZenohSPI provided by QEMU.
    let s = unsafe { &mut *(dev as *mut ZenohSPI) };

    let id_str = if s.id.is_null() {
        format!("spi{}", s.node_id)
    } else {
        // SAFETY: s.id is a valid C-string if it's not null.
        unsafe { CStr::from_ptr(s.id).to_string_lossy().into_owned() }
    };

    let router_str = if s.router.is_null() { ptr::null() } else { s.router.cast_const() };

    // SAFETY: get_or_init_session handles null and valid pointers.
    // Safety: router validity is guaranteed by the caller.
    let session = match unsafe { virtmcu_zenoh::get_or_init_session(router_str) } {
        Ok(s) => s,
        Err(_e) => {
            // SAFETY: errp is a valid error pointer.
            virtmcu_qom::error_setg!(errp, "zenoh-spi: failed to open Zenoh session");
            return;
        }
    };

    let backend = Box::new(ZenohSPIBackend { session, id: id_str });

    s.rust_state = Box::into_raw(backend);

    // vlog!("[zenoh-spi] Realized (id={}, node={})\n", id_str, s.node_id);
}

/// # Safety
/// This function is called by QEMU when finalizing the device. `obj` must be a valid pointer to `ZenohSPI`.
#[no_mangle]
pub unsafe extern "C" fn zenoh_spi_instance_finalize(obj: *mut Object) {
    // SAFETY: obj is a valid pointer to ZenohSPI provided by QEMU.
    let s = unsafe { &mut *(obj as *mut ZenohSPI) };
    if !s.rust_state.is_null() {
        // SAFETY: rust_state was allocated via Box::into_raw and is non-null.
        unsafe {
            drop(Box::from_raw(s.rust_state));
        }
        s.rust_state = ptr::null_mut();
    }
}

/* ── QOM Boilerplate ──────────────────────────────────────────────────────── */

define_properties!(
    ZENOH_SPI_PROPERTIES,
    [
        define_prop_uint32!(c"node".as_ptr(), ZenohSPI, node_id, 0),
        define_prop_string!(c"id".as_ptr(), ZenohSPI, id),
        define_prop_string!(c"router".as_ptr(), ZenohSPI, router),
    ]
);

/// # Safety
/// This function is called by QEMU to initialize the class. `klass` must be a valid `ObjectClass` pointer.
#[no_mangle]
pub unsafe extern "C" fn zenoh_spi_class_init(klass: *mut ObjectClass, _data: *const c_void) {
    let ssc = ssi_peripheral_class!(klass);
    // SAFETY: ssc is a valid SSIPeripheralClass pointer.
    let dc = unsafe { &mut (*ssc).parent_class };

    // SAFETY: ssc is a valid SSIPeripheralClass pointer.
    unsafe {
        (*ssc).realize = Some(zenoh_spi_realize);
        (*ssc).transfer = Some(zenoh_spi_transfer);
        (*ssc).set_cs = Some(zenoh_spi_set_cs);
        (*ssc).cs_polarity = 1; // SSI_CS_LOW (TODO: make property)
    }

    dc.user_creatable = true;
    virtmcu_qom::device_class_set_props!(dc, ZENOH_SPI_PROPERTIES);
}

static ZENOH_SPI_TYPE_INFO: TypeInfo = TypeInfo {
    name: c"zenoh-spi".as_ptr(),
    parent: TYPE_SSI_PERIPHERAL,
    instance_size: core::mem::size_of::<ZenohSPI>(),
    instance_align: 0,
    instance_init: None,
    instance_post_init: None,
    instance_finalize: Some(zenoh_spi_instance_finalize),
    abstract_: false,
    class_size: core::mem::size_of::<SSIPeripheralClass>(),
    class_init: Some(zenoh_spi_class_init),
    class_base_init: None,
    class_data: ptr::null(),
    interfaces: ptr::null(),
};

declare_device_type!(ZENOH_SPI_TYPE_INIT, ZENOH_SPI_TYPE_INFO);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zenoh_spi_layout() {
        assert_eq!(
            core::mem::offset_of!(ZenohSPI, parent_obj),
            0,
            "SSIPeripheral must be the first field"
        );
    }
}
