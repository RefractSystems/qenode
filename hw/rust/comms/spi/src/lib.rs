#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"
#![allow(clippy::if_not_else)]
#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::panic_in_result_fn
    )
)]
use virtmcu_api::FlatBufferStructExt;
use zenoh::Wait;
extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, CStr};
use core::ptr;

use virtmcu_api::ZenohSPIHeader;
use virtmcu_qom::ssi::SSIPeripheral;
use virtmcu_qom::sync::{Condvar, Mutex, VcpuDrain};
use virtmcu_qom::timer::{qemu_clock_get_ns, QEMU_CLOCK_VIRTUAL};

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
    pub transport_link: virtmcu_qom::qom::QomLink<dyn virtmcu_api::DataTransport>,

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
    pub transport: Option<Arc<dyn virtmcu_api::DataTransport>>,
    pub qemu_dev_ptr: *mut VirtmcuSPIQEMU,
    pub _liveliness: Option<zenoh::liveliness::LivelinessToken>,
}

impl virtmcu_qom::device::PeripheralState for VirtmcuSPIState {
    type QomType = VirtmcuSPIQEMU;

    fn new(qemu_dev: &Self::QomType) -> Self {
        let id = if qemu_dev.id.is_null() {
            format!("spi{}", qemu_dev.node_id)
        } else {
            unsafe { CStr::from_ptr(qemu_dev.id).to_string_lossy().into_owned() }
        };

        let transport_name = if qemu_dev.transport.is_null() {
            "zenoh".to_owned()
        } else {
            unsafe { CStr::from_ptr(qemu_dev.transport).to_string_lossy().into_owned() }
        };

        let router = if qemu_dev.router.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(qemu_dev.router).to_string_lossy().into_owned() }
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
    fn realize(&mut self) -> Result<(), alloc::string::String> {
        let router_ptr = if self.router.is_empty() {
            ptr::null()
        } else {
            self.router.as_ptr() as *const c_char
        };

        if self.transport.is_none() {
            let transport: Arc<dyn virtmcu_api::DataTransport> = if self.transport_name == "unix" {
                let path = if router_ptr.is_null() {
                    format!("/tmp/virtmcu-coord-{}.sock", self.node_id) // virtmcu-allow: absolute_path reasoning="Legacy script"
                } else {
                    self.router.clone()
                };
                match transport_unix::UdsDataTransport::new(&path) {
                    Ok(t) => Arc::new(t),
                    Err(_) => return Err("spi: failed to open unix socket".into()),
                }
            } else {
                match unsafe { transport_zenoh::get_or_init_session(router_ptr) } {
                    Ok(session) => Arc::new(transport_zenoh::ZenohDataTransport::new(session)),
                    Err(_) => return Err("spi: failed to open Zenoh session".into()),
                }
            };
            self.transport = Some(transport);
        }

        if self.transport_name == "zenoh" {
            if let Ok(session) = unsafe { transport_zenoh::get_or_init_session(router_ptr) } {
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
        _token: &virtmcu_qom::device::DrainToken,
    ) -> virtmcu_qom::device::MmioResult<'_> {
        virtmcu_qom::device::MmioDevice::read(self, addr, size)
    }

    fn write(&self, addr: u64, val: u64, size: u32, _token: &virtmcu_qom::device::DrainToken) {
        virtmcu_qom::device::MmioDevice::write(self, addr, val, size);
    }

    fn condvar(&self) -> &Condvar {
        &self.cond
    }

    // virtmcu-allow: mutex reasoning="Required by Peripheral trait API for Condvar"
    fn wait_mutex(&self) -> &Mutex<()> {
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
    fn wait_mutex(&self) -> &Mutex<()> {
        &self.wait_mutex
    }
}

/// # Safety
/// This function is called by QEMU when an SPI transfer happens.
#[no_mangle]
pub unsafe extern "C" fn spi_transfer(dev: *mut SSIPeripheral, val: u32) -> u32 {
    let s = unsafe { &mut *(dev as *mut VirtmcuSPIQEMU) };
    if s.state.is_null() {
        return 0;
    }
    let backend = unsafe { &*s.state };
    let _guard = backend.drain.acquire();

    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let header =
        unsafe { ZenohSPIHeader::new(now, 0, SPI_WORD_SIZE_U32, (*dev).cs, (*dev).cs_index, 0) };

    let mut data = Vec::with_capacity(virtmcu_api::ZENOH_SPI_HEADER_SIZE + SPI_WORD_SIZE);
    data.extend_from_slice(header.pack());
    data.extend_from_slice(&val.to_le_bytes());

    let topic = unsafe { format!("sim/spi/{}/{}", backend.id, (*dev).cs_index) };

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
pub unsafe extern "C" fn spi_set_cs(dev: *mut SSIPeripheral, select: bool) -> c_int {
    let s = unsafe { &mut *(dev as *mut VirtmcuSPIQEMU) };
    if s.state.is_null() {
        return 0;
    }
    let backend = unsafe { &*s.state };
    let _guard = backend.drain.acquire();

    let now = unsafe { qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) } as u64;
    let header = unsafe { ZenohSPIHeader::new(now, 0, 0, select, (*dev).cs_index, 0) };

    let header_bytes = header.pack();

    let topic = unsafe { format!("sim/spi/{}/{}/cs", backend.id, (*dev).cs_index) };

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
