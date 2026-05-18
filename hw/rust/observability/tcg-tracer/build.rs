#![allow(clippy::all, unused_imports, dead_code, unused_variables, unused_mut)] // virtmcu-allow: allow reasoning="Zero unsafe"
#![allow(clippy::all)] // virtmcu-allow: allow reasoning="Zero unsafe"
fn main() {
    println!("cargo:rustc-link-arg=-Wl,--unresolved-symbols=ignore-all"); // virtmcu-allow: print reasoning="cargo build script protocol"
}
