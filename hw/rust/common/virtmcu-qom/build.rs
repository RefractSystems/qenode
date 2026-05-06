#[allow(clippy::too_many_lines)]
fn main() {
    println!("cargo:rustc-check-cfg=cfg(qemu_headers_present)"); // PRINT_EXCEPTION: cargo build script protocol
    println!("cargo:rustc-check-cfg=cfg(qemu_headers_missing)"); // PRINT_EXCEPTION: cargo build script protocol
    println!("cargo:rustc-check-cfg=cfg(virtmcu_unit_test)"); // PRINT_EXCEPTION: cargo build script protocol

    // Skip everything if running under Miri as it cannot handle FFI/C
    if std::env::var("CARGO_CFG_MIRI").is_ok() || std::env::var("MIRI_SYSROOT").is_ok() {
        let out_path = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        std::fs::write(out_path.join("bindings.rs"), "").unwrap();
        let wrapper_path = out_path.join("qemu_bindings.rs");
        std::fs::write(&wrapper_path, "pub mod qemu {}").unwrap();
        println!("cargo:rustc-cfg=qemu_headers_missing"); // PRINT_EXCEPTION: cargo build script protocol
        return;
    }

    let qemu_dir =
        std::env::var("QEMU_SRC_DIR").unwrap_or_else(|_| "../../../../third_party/qemu".to_owned());
    let build_dir = if std::env::var("VIRTMCU_USE_ASAN").unwrap_or_default() == "1" {
        "build-virtmcu-asan"
    } else {
        "build-virtmcu"
    };

    let qemu_build_dir = std::env::var("QEMU_BUILD_DIR")
        .unwrap_or_else(|_| format!("../../../../third_party/qemu/{build_dir}"));

    // Check if QEMU headers are present
    let osdep_h = std::path::Path::new(&qemu_dir).join("include/qemu/osdep.h");
    if !osdep_h.exists() {
        if std::env::var("VIRTMCU_SKIP_QEMU_HEADERS_WARNING").is_err() {
            println!(
                // PRINT_EXCEPTION: cargo build script protocol
                "cargo:warning=QEMU headers not found at {}. Skipping binding and FFI generation.",
                osdep_h.display()
            );
        }
        // Create an empty bindings file so the build doesn't fail
        let out_path = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        std::fs::write(out_path.join("bindings.rs"), "").unwrap_or_else(|_| std::process::abort()); // "Couldn't write dummy bindings!");
                                                                                                    // Create an empty wrapper too
        let wrapper_path = out_path.join("qemu_bindings.rs");
        std::fs::write(&wrapper_path, "pub mod qemu {}").unwrap();
        println!("cargo:rustc-cfg=qemu_headers_missing"); // PRINT_EXCEPTION: cargo build script protocol
        return;
    }

    println!("cargo:rustc-cfg=qemu_headers_present"); // PRINT_EXCEPTION: cargo build script protocol
    println!("cargo:rerun-if-changed=wrapper.h"); // PRINT_EXCEPTION: cargo build script protocol
    println!("cargo:rerun-if-changed=src/ffi.c"); // PRINT_EXCEPTION: cargo build script protocol
    println!("cargo:rerun-if-changed=src/ffi.h"); // PRINT_EXCEPTION: cargo build script protocol

    let mut builder = cc::Build::new();
    builder.define("_GNU_SOURCE", None);

    if std::env::var("VIRTMCU_UNIT_TEST").is_ok() {
        builder.define("UNIT_TEST", None);
        println!("cargo:rustc-cfg=virtmcu_unit_test"); // PRINT_EXCEPTION: cargo build script protocol
    }

    builder
        .include("src")
        .include(format!("{qemu_dir}/include"))
        .include(&qemu_build_dir)
        .include(format!("{qemu_build_dir}/qapi"))
        .include(format!("{qemu_dir}/linux-headers"))
        .include("/usr/include/glib-2.0")
        .include("/usr/lib/aarch64-linux-gnu/glib-2.0/include")
        .include("/usr/lib/x86_64-linux-gnu/glib-2.0/include")
        .flag("-w")
        .flag("-Wno-unused-parameter")
        .flag("-Wno-sign-compare")
        .file("src/ffi.c")
        .compile("virtmcu-qom-ffi");

    let out_path = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let bindings_file = out_path.join("bindings.rs");

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{qemu_dir}/include"))
        .clang_arg(format!("-I{qemu_build_dir}"))
        .clang_arg(format!("-I{qemu_build_dir}/qapi"))
        .clang_arg(format!("-I{qemu_dir}/linux-headers"))
        .clang_arg("-I/usr/include/glib-2.0")
        .clang_arg("-I/usr/lib/aarch64-linux-gnu/glib-2.0/include")
        .clang_arg("-I/usr/lib/x86_64-linux-gnu/glib-2.0/include")
        .clang_arg("-D_GNU_SOURCE")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .allowlist_type("DeviceState")
        .allowlist_type("SysBusDevice")
        .allowlist_type("MemoryRegion")
        .allowlist_type("CharBackend")
        .allowlist_type("NetClientState")
        .allowlist_type("NetClientInfo")
        .allowlist_type("QemuOpts")
        .allowlist_type("Error")
        .allowlist_type("SSIPeripheral")
        .allowlist_type("SSIBus")
        .allowlist_type("QEMUTimer")
        .allowlist_type("QEMUClockType")
        .allowlist_function("qdev_.*")
        .allowlist_function("sysbus_.*")
        .allowlist_function("memory_region_.*")
        .allowlist_function("qemu_chr_fe_.*")
        .allowlist_function("qemu_new_timer_.*")
        .allowlist_function("timer_.*")
        .allowlist_function("qemu_clock_get_ns")
        .allowlist_function("virtmcu_.*")
        .allowlist_function("object_.*")
        .allowlist_function("ssi_.*")
        .allowlist_var("TYPE_.*")
        .generate()
        .expect("Unable to generate bindings");

    bindings.write_to_file(&bindings_file).expect("Couldn't write bindings!");

    // Create a self-contained wrapper module to isolate lints
    let wrapper_path = out_path.join("qemu_bindings.rs");
    let wrapper_content = format!(
        "#[allow(dead_code, non_snake_case, non_camel_case_types, non_upper_case_globals, clippy::all, clippy::pedantic, unnecessary_transmutes)] // ALLOW_EXCEPTION: Bindgen-generated QEMU bindings\n\
         pub mod qemu {{\n\
             include!({:?});\n\
         }}",
        bindings_file.to_str().unwrap()
    );
    std::fs::write(&wrapper_path, wrapper_content).unwrap();
}
