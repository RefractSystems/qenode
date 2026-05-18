use anyhow::Result;

use tokio::process::Command;
use virtmcu_test_runner::TestContext;

#[tokio::test]
async fn test_svd2header_cli() -> Result<()> {
    let ctx = TestContext::new()?;

    // Find the virtmcu-cli binary
    let cli = ctx.find_binary("virtmcu-cli")?;

    let svd_file = ctx.workspace_root.join("hw/defs/actuator.svd");
    let out_header = ctx.tmp_path("actuator_generated.h");
    let out_c = ctx.tmp_path("actuator_test.c");

    // Generate header
    let status = Command::new(&cli)
        .args([
            "platform",
            "svd2-header",
            svd_file.to_str().unwrap(),
            "-o",
            out_header.to_str().unwrap(),
        ])
        .status()
        .await?;

    assert!(status.success(), "virtmcu-cli platform svd2header failed");

    // Create a minimal C file that includes the header
    let c_content = r#"
#include "actuator_generated.h"

void test_func() {
    // Just a dummy function to ensure the file is not empty
    volatile uint32_t val = ACTUATOR->ID;
    (void)val;
}
"#;
    std::fs::write(&out_c, c_content)?;

    // Compile the C file to verify _Static_asserts pass
    let out_o = ctx.tmp_path("actuator_test.o");
    let gcc_status = Command::new("arm-none-eabi-gcc")
        .args([
            "-c",
            "-std=c11",
            "-Wall",
            "-Werror",
            "-I",
            out_header.parent().unwrap().to_str().unwrap(),
            out_c.to_str().unwrap(),
            "-o",
            out_o.to_str().unwrap(),
        ])
        .status()
        .await?;

    assert!(
        gcc_status.success(),
        "arm-none-eabi-gcc failed to compile generated header (Static Asserts failed)"
    );

    Ok(())
}
