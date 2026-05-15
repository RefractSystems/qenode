use anyhow::Result;

use tokio::process::Command;
use virtmcu_test_runner::TestContext;

#[tokio::test]
async fn test_svd_patch_cli() -> Result<()> {
    let ctx = TestContext::new()?;

    let input_svd = ctx.workspace_root.join("hw/defs/actuator.svd");
    let patch_yaml = ctx
        .workspace_root
        .join("tests/fixtures/svd_patching/patch.yaml");
    let output_svd = ctx.tmp_path("patched_actuator.svd");

    let status = Command::new("cargo")
        .args([
            "run",
            "-Z",
            "bindeps",
            "-p",
            "virtmcu-cli",
            "--",
            "platform",
            "patch-svd",
            input_svd.to_str().unwrap(),
            patch_yaml.to_str().unwrap(),
            "--output",
            output_svd.to_str().unwrap(),
        ])
        .status()
        .await?;

    assert!(status.success(), "virtmcu-cli platform patch-svd failed");

    let patched_content = std::fs::read_to_string(&output_svd)?;
    // The bash script checked: grep -A 5 "<name>GO</name>" "$OUTPUT_SVD" | grep -q "0x1"
    assert!(
        patched_content.contains("0x1"),
        "resetValue was not found in the patched SVD"
    );

    Ok(())
}
