use std::env;
use std::path::PathBuf;

fn main() {
    let qemu_dir = "../../../third_party/qemu";
    let build_dir = "../../../third_party/qemu/build-virtmcu";

    println!("cargo:rerun-if-changed=wrapper.h");

    // Check if QEMU headers are present
    let osdep_h = std::path::Path::new(qemu_dir).join("include/qemu/osdep.h");
    if !osdep_h.exists() {
        println!(
            "cargo:warning=QEMU headers not found at {:?}. Skipping binding generation.",
            osdep_h
        );
        // Create an empty bindings file so the build doesn't fail
        let out_path = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        std::fs::write(out_path.join("bindings.rs"), "").expect("Couldn't write dummy bindings!");
        return;
    }

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{}/include", qemu_dir))
        .clang_arg(format!("-I{}", build_dir))
        .clang_arg(format!("-I{}/qapi", build_dir))
        .clang_arg(format!("-I{}/linux-headers", qemu_dir))
        .clang_arg("-I/usr/include/glib-2.0")
        .clang_arg("-I/usr/lib/aarch64-linux-gnu/glib-2.0/include")
        .clang_arg("-I/usr/lib/x86_64-linux-gnu/glib-2.0/include") // support x86_64 too just in case
        .allowlist_type("TypeInfo")
        .allowlist_type("ObjectClass")
        .allowlist_type("Property")
        .allowlist_type("DeviceState")
        .allowlist_type("DeviceClass")
        .allowlist_type("SysBusDevice")
        .allowlist_type("MemoryRegion")
        .allowlist_type("MemoryRegionOps")
        .allowlist_type("Chardev")
        .allowlist_type("ChardevClass")
        .allowlist_type("NetClientState")
        .allowlist_type("NetClientInfo")
        .allowlist_type("CPUState")
        .allowlist_type("QemuMutex")
        .allowlist_type("QemuCond")
        .layout_tests(true)
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
