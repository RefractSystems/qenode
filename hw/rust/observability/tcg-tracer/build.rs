fn main() {
    println!("cargo:rustc-link-arg=-Wl,--unresolved-symbols=ignore-all"); // virtmcu-allow: print reasoning="cargo build script protocol"
}
