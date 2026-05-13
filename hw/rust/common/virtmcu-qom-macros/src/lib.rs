use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(MmioDevice)]
pub fn derive_mmio_device(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    // We expect the struct to have `pub parent_obj: SysBusDevice` and `pub rust_state: *mut State`
    // but the trait is implemented on the State object (e.g., VirtmcuSensorState), not the QEMU struct.
    // Actually, looking at rust-dummy, the operations take the opaque pointer which is the QEMU struct.
    // Let's generate a trait `MmioDevice` implementation block and the `MemoryRegionOps`.

    // The name here is likely the State struct (e.g., VirtmcuSensorState) or the QEMU struct.
    // The easiest is to make the user apply `#[derive(MmioDevice)]` to the QEMU struct,
    // which has `rust_state` pointing to the state struct that implements the trait.
    // Wait, the prompt says:
    // Create a `#[derive(MmioDevice)]` proc-macro... consuming the safe `MmioDevice` trait implementation.

    // Let's generate the `MemoryRegionOps` with `name` being the struct that implements `MmioDevice`.
    // We need to know the QEMU struct name or have it passed as an attribute.
    // Actually, in `sensor`, `s` is `VirtmcuSensorQEMU`, and `state` is `VirtmcuSensorState`.
    // It's better if `MmioDevice` is implemented on `VirtmcuSensorState` and we generate the C callbacks.
    // But how do we know the QEMU struct name?

    // Let's define the macro to be applied on the QEMU struct instead.
    // `#[derive(MmioDevice)]` on `VirtmcuSensorQEMU`.
    // The macro generates `VIRTM_MMIO_OPS` constant? Let's just generate the read/write functions and ops.

    let qemu_struct = name;
    let ops_name =
        syn::Ident::new(&format!("{}_OPS", name.to_string().to_uppercase()), name.span());
    let read_fn =
        syn::Ident::new(&format!("{}_read_shim", name.to_string().to_lowercase()), name.span());
    let write_fn =
        syn::Ident::new(&format!("{}_write_shim", name.to_string().to_lowercase()), name.span());

    let expanded = quote! {
        const BQL_YIELD_TIMEOUT_MS: u32 = 100;
        const MAX_ACCESS_SIZE: u32 = 8;

        unsafe extern "C" fn #read_fn(opaque: *mut core::ffi::c_void, offset: u64, size: core::ffi::c_uint) -> u64 {
            let s = unsafe { &mut *(opaque as *mut #qemu_struct) };
            if s.rust_state.is_null() {
                return 0;
            }
            let state = unsafe { &*s.rust_state };

            let mut res = virtmcu_qom::device::MmioDevice::read(state, offset, size as u32);
            match res {
                virtmcu_qom::device::MmioResult::Ready(val) => val,
                virtmcu_qom::device::MmioResult::Wait { mut condition, mut ready_val } => {
                    while !condition() {
                        let guard = virtmcu_qom::device::MmioDevice::wait_mutex(state).lock();
                        let _ = virtmcu_qom::device::MmioDevice::condvar(state).wait_yielding_bql(guard, BQL_YIELD_TIMEOUT_MS);
                    }
                    ready_val()
                }
            }
        }

        unsafe extern "C" fn #write_fn(opaque: *mut core::ffi::c_void, offset: u64, value: u64, size: core::ffi::c_uint) {
            let s = unsafe { &mut *(opaque as *mut #qemu_struct) };
            if s.rust_state.is_null() {
                return;
            }
            let state = unsafe { &*s.rust_state };
            virtmcu_qom::device::MmioDevice::write(state, offset, value, size as u32);
        }

        pub static #ops_name: virtmcu_qom::memory::MemoryRegionOps = virtmcu_qom::memory::MemoryRegionOps {
            read: Some(#read_fn),
            write: Some(#write_fn),
            read_with_attrs: core::ptr::null(),
            write_with_attrs: core::ptr::null(),
            endianness: virtmcu_qom::memory::DEVICE_LITTLE_ENDIAN,
            _padding1: [0; 4],
            valid: virtmcu_qom::memory::MemoryRegionValidRange {
                min_access_size: 1,
                max_access_size: MAX_ACCESS_SIZE,
                unaligned: false,
                _padding: [0; 7],
                accepts: core::ptr::null(),
            },
            impl_: virtmcu_qom::memory::MemoryRegionImplRange {
                min_access_size: 1,
                max_access_size: MAX_ACCESS_SIZE,
                unaligned: false,
                _padding: [0; 7],
            },
        };
    };

    TokenStream::from(expanded)
}
