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
pub mod error;
pub mod icount;
pub mod irq;
pub mod net;
pub mod qdev;
pub mod qom;
pub mod sync;
pub mod timer;
