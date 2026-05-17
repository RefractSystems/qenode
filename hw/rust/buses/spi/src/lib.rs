#![allow(clippy::all, unused_imports, dead_code, unused_variables, unused_mut)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::all)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![allow(clippy::if_not_else)]
#![allow(clippy::not_unsafe_ptr_arg_deref)]
// virtmcu-allow: allow reasoning="Zero unsafe"
// virtmcu-allow: allow reasoning="Pending P1 migration: deref_qom_ptr/opaque_to_state replaced by dynamic_cast_qom"
#![allow(deprecated)]
#![allow(clippy::missing_safety_doc)]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]

use virtmcu_wire::FlatBufferStructExt;
use zenoh::Wait;
extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, CStr};
use core::ptr;

use virtmcu_qom::ssi::SSIPeripheral;
use virtmcu_qom::sync::{Condvar, Mutex, VcpuDrain};
use virtmcu_qom::timer::QEMU_CLOCK_VIRTUAL;
use virtmcu_wire::ZenohSPIHeader;

const SPI_WORD_SIZE: usize = 4;
const SPI_WORD_SIZE_U32: u32 = 4;

#[repr(C)]
#[derive(virtmcu_qom::MmioDevice)]
#[virtmcu_qom::macros::qom_device(name = "spi", parent = "ssi-peripheral")]
pub struct VirtmcuSPIQEMU {
    pub parent_obj: SSIPeripheral,
    pub iomem: virtmcu_qom::memory::MemoryRegion,

    #[qom_property]
    pub node_id: u32,
    #[qom_property]
    pub transport: *mut c_char,
    #[qom_property]
    pub id: *mut c_char,
    #[qom_property]
    pub router: *mut c_char,

    #[qom_link(target = "virtmcu-transport-hub")]
    pub transport_link: virtmcu_qom::qom::QomLink<dyn virtmcu_wire::DataTransport>,

    #[qom_state]
    pub state: VirtmcuSPIState,
}

pub struct VirtmcuSPIState {
    pub node_id: u32,
    pub id: String,
    pub transport_name: String,
    pub router: String,
    pub drain: VcpuDrain,
    pub cond: Condvar,
    pub wait_mutex: Mutex<()>, // virtmcu-allow: mutex reasoning="Required by Peripheral trait"
    pub transport: Option<Arc<dyn virtmcu_wire::DataTransport>>,
    pub qemu_dev_ptr: *mut VirtmcuSPIQEMU,
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
}

impl virtmcu_qom::device::PeripheralState for VirtmcuSPIState {
    type QomType = VirtmcuSPIQEMU;

    fn new(qemu_dev: &Self::QomType) -> Self {
        let id = if qemu_dev.id.is_null() {
            format!("spi{}", qemu_dev.node_id)
        } else {
            virtmcu_qom::ffi_call! { CStr::from_ptr(qemu_dev.id).to_string_lossy().into_owned() }
        };

        let transport_name = if qemu_dev.transport.is_null() {
            "zenoh".to_owned()
        } else {
            virtmcu_qom::ffi_call! { CStr::from_ptr(qemu_dev.transport).to_string_lossy().into_owned() }
        };

        let router = if qemu_dev.router.is_null() {
            String::new()
        } else {
            virtmcu_qom::ffi_call! { CStr::from_ptr(qemu_dev.router).to_string_lossy().into_owned() }
        };

        Self {
            node_id: qemu_dev.node_id,
            id,
            transport_name,
            router,
            drain: VcpuDrain::new(),
            cond: Condvar::new(),
            wait_mutex: Mutex::new(()),
            transport: qemu_dev.transport_link.get(),
            qemu_dev_ptr: core::ptr::from_ref(qemu_dev).cast_mut(),
            _liveliness: None,
        }
    }
}

impl virtmcu_qom::device::Peripheral for VirtmcuSPIState {
    fn realize(
        &mut self,
        ctx: &virtmcu_qom::device::BqlContext,
    ) -> Result<(), alloc::string::String> {
        let router_ptr = if self.router.is_empty() {
            ptr::null()
        } else {
            self.router.as_ptr() as *const c_char
        };

        if self.transport.is_none() {
            let transport: Arc<dyn virtmcu_wire::DataTransport> = if self.transport_name == "unix" {
                let path = if router_ptr.is_null() {
                    format!("/tmp/virtmcu-coord-{}.sock", self.node_id) // virtmcu-allow: absolute_path reasoning="Legacy script"
                } else {
                    self.router.clone()
                };
                // virtmcu-allow: env_in_peripheral reasoning="Not yet ported: needs federation-id QOM property + new_with_fed_id"
                match transport_uds::UdsDataTransport::new(&path, self.node_id) {
                    Ok(t) => Arc::new(t),
                    Err(_) => return Err("spi: failed to open unix socket".into()),
                }
            } else {
                match virtmcu_qom::ffi_call! { transport_zenoh::get_or_init_session(router_ptr) } {
                    Ok(session) => {
                        Arc::new(transport_zenoh::ZenohDataTransport::new(session, self.node_id))
                    }
                    Err(_) => return Err("spi: failed to open Zenoh session".into()),
                }
            };
            self.transport = Some(transport);
        }

        if self.transport_name == "zenoh" {
            if let Ok(session) =
                virtmcu_qom::ffi_call! { transport_zenoh::get_or_init_session(router_ptr) }
            {
                let hb_topic = format!("sim/spi/liveliness/{}", self.node_id);
                self._liveliness = session.liveliness().declare_token(hb_topic).wait().ok();
            }
        }

        Ok(())
    }

    fn read(
        &self,
        addr: u64,
        size: u32,
        ctx: &virtmcu_qom::device::BqlContext,
    ) -> virtmcu_qom::device::MmioResult<'_> {
        virtmcu_qom::device::MmioDevice::read(self, addr, size)
    }

    fn write(&self, addr: u64, val: u64, size: u32, ctx: &virtmcu_qom::device::BqlContext) {
        virtmcu_qom::device::MmioDevice::write(self, addr, val, size);
    }

    fn condvar(&self) -> &Condvar {
        &self.cond
    }

    // virtmcu-allow: mutex reasoning="Required by Peripheral trait API for Condvar"
    #[rustfmt::skip]
    fn wait_mutex(&self) -> &Mutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
}

impl virtmcu_qom::device::MmioDevice for VirtmcuSPIState {
    fn read(&self, _addr: u64, _size: u32) -> virtmcu_qom::device::MmioResult<'_> {
        let _guard = self.drain.acquire();
        virtmcu_qom::device::MmioResult::Ready(0)
    }

    fn write(&self, _addr: u64, _val: u64, _size: u32) {
        let _guard = self.drain.acquire();
    }

    fn condvar(&self) -> &Condvar {
        &self.cond
    }

    // virtmcu-allow: mutex reasoning="Required by Peripheral trait API for Condvar"
    #[rustfmt::skip]
    fn wait_mutex(&self) -> &Mutex<()> { // virtmcu-allow: mutex reasoning="State managed securely"
        &self.wait_mutex
    }
}

/// # Safety
/// This function is called by QEMU when an SPI transfer happens.
#[no_mangle]
pub extern "C" fn spi_transfer(dev: *mut SSIPeripheral, val: u32) -> u32 {
    let s = virtmcu_qom::timer::deref_qom_ptr::<VirtmcuSPIQEMU>(dev as *mut core::ffi::c_void);
    if s.state.is_null() {
        return 0;
    }
    let backend = virtmcu_qom::ffi_call! { &*s.state };
    let _guard = backend.drain.acquire();

    let now = virtmcu_qom::telemetry::get_global_vtime();
    let header = virtmcu_qom::ffi_call! { ZenohSPIHeader::new(now, 0, SPI_WORD_SIZE_U32, (*dev).cs, (*dev).cs_index, 0) };

    let mut data = Vec::with_capacity(virtmcu_wire::ZENOH_SPI_HEADER_SIZE + SPI_WORD_SIZE);
    data.extend_from_slice(header.pack());
    data.extend_from_slice(&val.to_le_bytes());

    let topic = virtmcu_qom::ffi_call! { format!("sim/spi/{}/{}", backend.id, (*dev).cs_index) };

    if let Some(transport) = &backend.transport {
        // TODO(Task-11): transport.query() bypasses the PDES quantum barrier.
        // Full migration requires coordinator-aware SPI co-simulator.
        match transport.query(&topic, &data) {
            Ok(payload) => {
                if payload.len() >= SPI_WORD_SIZE {
                    u32::from_le_bytes(payload[..SPI_WORD_SIZE].try_into().unwrap_or_default())
                } else {
                    0
                }
            }
            Err(_) => 0,
        }
    } else {
        0
    }
}

/// # Safety
/// This function is called by QEMU when Chip Select state changes.
#[no_mangle]
pub extern "C" fn spi_set_cs(dev: *mut SSIPeripheral, select: bool) -> c_int {
    let s = virtmcu_qom::timer::deref_qom_ptr::<VirtmcuSPIQEMU>(dev as *mut core::ffi::c_void);
    if s.state.is_null() {
        return 0;
    }
    let backend = virtmcu_qom::ffi_call! { &*s.state };
    let _guard = backend.drain.acquire();

    let now = virtmcu_qom::telemetry::get_global_vtime();
    let header =
        virtmcu_qom::ffi_call! { ZenohSPIHeader::new(now, 0, 0, select, (*dev).cs_index, 0) };

    let header_bytes = header.pack();

    let topic = virtmcu_qom::ffi_call! { format!("sim/spi/{}/{}/cs", backend.id, (*dev).cs_index) };

    if let Some(transport) = &backend.transport {
        if let Ok(mut reservation) = transport.reserve(&topic, header_bytes.len()) {
            reservation.buffer_mut().copy_from_slice(header_bytes);
            let _ = reservation.commit(0, 0);
        }
    }

    0
}

virtmcu_qom::register_peripheral!(VirtmcuSPIQEMU);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spi_layout() {
        assert_eq!(
            core::mem::offset_of!(VirtmcuSPIQEMU, parent_obj),
            0,
            "SSIPeripheral must be the first field"
        );
    }
}
