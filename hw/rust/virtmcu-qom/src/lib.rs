#![allow(
    clippy::missing_safety_doc,
    clippy::collapsible_match,
    dead_code,
    unused_imports,
    clippy::len_zero
)]
#![no_std]
pub mod chardev;
pub mod cpu;
pub mod irq;
pub mod net;
pub mod proto;
pub mod qom;
pub mod sync;
pub mod timer;
