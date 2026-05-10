#[allow(clippy::too_many_lines, clippy::std_instead_of_core)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=VIRTMCU_UNIT_TEST"); // virtmcu-allow: print reasoning="cargo build script protocol"
    println!("cargo:rerun-if-env-changed=VIRTMCU_USE_ASAN"); // virtmcu-allow: print reasoning="cargo build script protocol"
    println!("cargo:rerun-if-env-changed=QEMU_SRC_DIR"); // virtmcu-allow: print reasoning="cargo build script protocol"
    println!("cargo:rerun-if-env-changed=QEMU_BUILD_DIR"); // virtmcu-allow: print reasoning="cargo build script protocol"

    println!("cargo:rustc-check-cfg=cfg(qemu_headers_present)"); // virtmcu-allow: print reasoning="cargo build script protocol"
    println!("cargo:rustc-check-cfg=cfg(qemu_headers_missing)"); // virtmcu-allow: print reasoning="cargo build script protocol"
    println!("cargo:rustc-check-cfg=cfg(virtmcu_unit_test)"); // virtmcu-allow: print reasoning="cargo build script protocol"
                                                              // Skip everything if running under Miri as it cannot handle FFI/C
    if std::env::var("CARGO_CFG_MIRI").is_ok() || std::env::var("MIRI_SYSROOT").is_ok() {
        let out_path = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
        std::fs::write(out_path.join("bindings.rs"), "")?;
        let wrapper_path = out_path.join("qemu_bindings.rs");
        std::fs::write(&wrapper_path, "pub mod qemu {}")?;
        println!("cargo:rustc-cfg=qemu_headers_missing"); // virtmcu-allow: print reasoning="cargo build script protocol"
        return Ok(());
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
    let has_headers = osdep_h.exists();

    if has_headers {
        println!("cargo:rustc-cfg=qemu_headers_present"); // virtmcu-allow: print reasoning="cargo build script protocol"
    } else {
        if std::env::var("VIRTMCU_SKIP_QEMU_HEADERS_WARNING").is_err() {
            println!(
                // virtmcu-allow: print reasoning="cargo build script protocol"
                "cargo:warning=QEMU headers not found at {}. Skipping binding generation.",
                osdep_h.display()
            );
        }
        // Create an empty bindings file so the build doesn't fail
        let out_path = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
        std::fs::write(out_path.join("bindings.rs"), "").unwrap_or_else(|_| std::process::abort()); // "Couldn't write dummy bindings!");
                                                                                                    // Create an empty wrapper too
        let wrapper_path = out_path.join("qemu_bindings.rs");
        std::fs::write(&wrapper_path, "pub mod qemu {}")?;
        println!("cargo:rustc-cfg=qemu_headers_missing"); // virtmcu-allow: print reasoning="cargo build script protocol"
                                                          // If we are NOT in unit test mode, we can't compile ffi.c because it needs headers
        if std::env::var("VIRTMCU_UNIT_TEST").is_err()
            && std::env::var("CARGO_FEATURE_STANDALONE").is_err()
        {
            return Ok(());
        }
    }

    println!("cargo:rerun-if-changed=wrapper.h"); // virtmcu-allow: print reasoning="cargo build script protocol"
    println!("cargo:rerun-if-changed=src/ffi.c"); // virtmcu-allow: print reasoning="cargo build script protocol"
    println!("cargo:rerun-if-changed=src/ffi.h"); // virtmcu-allow: print reasoning="cargo build script protocol"
    let mut builder = cc::Build::new();
    builder.define("_GNU_SOURCE", None);

    if std::env::var("VIRTMCU_UNIT_TEST").is_ok()
        || std::env::var("CARGO_FEATURE_STANDALONE").is_ok()
    {
        builder.define("UNIT_TEST", None);
        println!("cargo:rustc-cfg=virtmcu_unit_test"); // virtmcu-allow: print reasoning="cargo build script protocol"
    }

    builder.include("src");

    if has_headers {
        builder
            .include(format!("{qemu_dir}/include"))
            .include(&qemu_build_dir)
            .include(format!("{qemu_build_dir}/qapi"))
            .include(format!("{qemu_dir}/linux-headers"))
            .include("/usr/include/glib-2.0")
            .include("/usr/lib/aarch64-linux-gnu/glib-2.0/include")
            .include("/usr/lib/x86_64-linux-gnu/glib-2.0/include");
    }

    builder
        .flag("-w")
        .flag("-Wno-unused-parameter")
        .flag("-Wno-sign-compare")
        .file("src/ffi.c")
        .compile("virtmcu-qom-ffi");

    if !has_headers {
        return Ok(());
    }

    let out_path = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
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
        .allowlist_type("TypeInfo")
        .allowlist_type("DeviceClass")
        .allowlist_type("ChardevClass")
        .allowlist_type("CanBusClientInfo")
        .allowlist_type("qemu_can_frame")
        .allowlist_type("CanHostState")
        .allowlist_type("CPUState")
        .allowlist_type("QemuCond")
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
        .map_err(|_| "Unable to generate bindings")?;

    bindings.write_to_file(&bindings_file)?;

    // Create a self-contained wrapper module to isolate lints
    let wrapper_path = out_path.join("qemu_bindings.rs");
    let wrapper_content = format!(
        "#[allow(dead_code, non_snake_case, non_camel_case_types, non_upper_case_globals, clippy::all, clippy::pedantic, unnecessary_transmutes, clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used, clippy::panic, clippy::todo, clippy::unimplemented)] // virtmcu-allow: allow reasoning=\"Bindgen-generated QEMU bindings\"\n
         pub mod qemu {{\n\
             include!({:?});\n\
         }}",
        bindings_file
            .to_str()
            .ok_or("build script I/O failed: path is not valid UTF-8")?
    );

    std::fs::write(&wrapper_path, wrapper_content)?;
    Ok(())
}
