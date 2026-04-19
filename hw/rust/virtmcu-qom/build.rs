use std::env;
use std::path::PathBuf;

fn main() {
    let qemu_dir =
        std::env::var("QEMU_SRC_DIR").unwrap_or_else(|_| "../../../third_party/qemu".to_string());
    let build_dir = std::env::var("QEMU_BUILD_DIR")
        .unwrap_or_else(|_| "../../../third_party/qemu/build-virtmcu".to_string());

    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=src/ffi.c");
    println!("cargo:rerun-if-changed=src/ffi.h");

    cc::Build::new()
        .file("src/ffi.c")
        .include(format!("{}/include", qemu_dir))
        .include(&build_dir)
        .include(format!("{}/qapi", build_dir))
        .include(format!("{}/linux-headers", qemu_dir))
        .include("/usr/include/glib-2.0")
        .include("/usr/lib/aarch64-linux-gnu/glib-2.0/include")
        .include("/usr/lib/x86_64-linux-gnu/glib-2.0/include")
        .compile("virtmcu_ffi");

    // Check if QEMU headers are present
    let osdep_h = std::path::Path::new(&qemu_dir).join("include/qemu/osdep.h");
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
    let bindings_file = out_path.join("bindings.rs");
    bindings
        .write_to_file(&bindings_file)
        .expect("Couldn't write bindings!");

    // Create a self-contained wrapper module to isolate lints
    let wrapper_path = out_path.join("qemu_bindings.rs");
    let wrapper_content = format!(
        "#[allow(dead_code, non_snake_case, non_camel_case_types, non_upper_case_globals, clippy::all, unnecessary_transmutes)]\n\
         pub mod qemu {{\n\
             include!({:?});\n\
         }}",
        bindings_file.to_str().unwrap()
    );
    std::fs::write(&wrapper_path, wrapper_content).unwrap();
}
