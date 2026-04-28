// ==============================================================================
// print_rust_sizes.rs
// 
// Rust template for printing the sizes and alignments of Rust structures.
// Used for debugging FFI layout mismatches with C.
// ==============================================================================

// Example: Stub out any QEMU dependencies if needed to compile standalone
mod memory { pub struct MemoryRegion; }
mod qom { pub struct Object { _opaque: [u8; 40] } }
mod error { pub struct Error; }

// Example: Include the source file directly or define the structs here
// include!("hw/rust/common/virtmcu-qom/src/net.rs");

fn main() {
    println!("Add the structures you want to measure to tools/ffi_layout_check/print_rust_sizes.rs");

    /* Example usage:
    println!("CanBusClientState size: {}, align: {}", std::mem::size_of::<CanBusClientState>(), std::mem::align_of::<CanBusClientState>());
    println!("CanBusClientInfo size: {}, align: {}", std::mem::size_of::<CanBusClientInfo>(), std::mem::align_of::<CanBusClientInfo>());
    println!("QemuCanFrame size: {}, align: {}", std::mem::size_of::<QemuCanFrame>(), std::mem::align_of::<QemuCanFrame>());
    */
}
