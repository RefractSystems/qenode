fn main() {
    println!("cargo:rustc-link-arg=-Wl,--unresolved-symbols=ignore-all");
}
