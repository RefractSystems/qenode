#![allow(clippy::panic)] // virtmcu-allow: allow reasoning="Fail Loudly"

use std::sync::Mutex as StdMutex;
use virtmcu_qom::device::{DrainToken, MmioDevice, MmioResult, Peripheral, PeripheralState};
use virtmcu_qom::sync::{Bql, Condvar, Mutex};
use virtmcu_qom_macros::{qom_device, MmioDevice};

pub struct MockState {
    pub condvar: Condvar,
    pub wait_mutex: Mutex<()>,
    pub counter: StdMutex<u64>,
}

impl PeripheralState for MockState {
    type QomType = MockDevice;

    fn new(_qemu_dev: &Self::QomType) -> Self {
        Self { condvar: Condvar::new(), wait_mutex: Mutex::new(()), counter: StdMutex::new(0) }
    }
}

impl Peripheral for MockState {
    fn read(&self, _offset: u64, _size: u32, _token: &DrainToken) -> MmioResult<'_> {
        MmioResult::Ready(0)
    }

    fn write(&self, _offset: u64, _value: u64, _size: u32, _token: &DrainToken) {}

    fn condvar(&self) -> &Condvar {
        &self.condvar
    }

    fn wait_mutex(&self) -> &Mutex<()> {
        &self.wait_mutex
    }
}

impl MmioDevice for MockState {
    fn read(&self, offset: u64, _size: u32) -> MmioResult<'_> {
        if offset == 0x0 {
            MmioResult::wait_for(
                || {
                    let mut lock = self.counter.lock().expect("mutex poisoned");
                    if *lock > 0 {
                        *lock -= 1;
                        true
                    } else {
                        false
                    }
                },
                || 42,
                || 0,
            )
        } else {
            MmioResult::Ready(0)
        }
    }

    fn write(&self, _offset: u64, _value: u64, _size: u32) {}

    fn condvar(&self) -> &Condvar {
        &self.condvar
    }

    fn wait_mutex(&self) -> &Mutex<()> {
        &self.wait_mutex
    }
}

#[qom_device(name = "mock-device", parent = "sys-bus-device")]
#[derive(MmioDevice)]
pub struct MockDevice {
    #[allow(dead_code)]
    parent_obj: virtmcu_qom::qdev::SysBusDevice,
    iomem: virtmcu_qom::memory::MemoryRegion,
    #[qom_state]
    state: MockState,
}

struct SyncPtr<T>(*mut T);
// SAFETY: test only
unsafe impl<T> Send for SyncPtr<T> {}
// SAFETY: test only
unsafe impl<T> Sync for SyncPtr<T> {}
impl<T> SyncPtr<T> {
    fn get(self) -> *mut T {
        self.0
    }
}

#[test]
fn test_mmio_wait_blocks_via_condvar() {
    let mut dev = MockDevice::new_mock();
    let state = Box::into_raw(Box::new(MockState::new(&dev)));
    dev.state = state;

    let _bql_guard = Bql::lock();

    // In a background thread, wait a bit and signal the condvar.
    let state_ptr = SyncPtr(dev.state);
    let signal_thread = std::thread::spawn(move || {
        std::thread::sleep(core::time::Duration::from_millis(50)); // virtmcu-allow: sleep reasoning="Wait for main thread to block in mock test"
        let state = unsafe { &*state_ptr.get() };
        *state.counter.lock().expect("mutex poisoned") += 1;
        state.condvar.notify_all();
    });

    let ops = &MOCKDEVICE_OPS;
    let read_fn = ops.read.expect("read fn must exist");

    let opaque = core::ptr::from_mut::<MockDevice>(&mut dev).cast::<core::ffi::c_void>();

    let val = unsafe { read_fn(opaque, 0x0, 4) };

    assert_eq!(val, 42);

    signal_thread.join().expect("thread join failed");

    unsafe {
        drop(Box::from_raw(state));
    }
}
