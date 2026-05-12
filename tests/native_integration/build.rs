fn main() {
    // Resolve the path to the stress_adapter binary provided by Cargo's artifact dependency feature.
    // If we're running with -Z bindeps, Cargo will provide this environment variable.
    println!("cargo:rerun-if-env-changed=CARGO_BIN_FILE_STRESS_ADAPTER_stress_adapter");
    if let Ok(path) = std::env::var("CARGO_BIN_FILE_STRESS_ADAPTER_stress_adapter") {
        println!("cargo:rustc-env=STRESS_ADAPTER_BIN={}", path);
    }
}
