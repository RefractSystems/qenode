use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::info;

pub struct ArtifactCache {
    cache_dir: PathBuf,
}

impl ArtifactCache {
    pub fn new(workspace_root: PathBuf) -> Result<Self> {
        let cache_dir = workspace_root.join("target/test-artifacts");
        std::fs::create_dir_all(&cache_dir)?;
        Ok(Self { cache_dir })
    }

    pub async fn get_firmware_asm(&self, asm_content: &str) -> Result<PathBuf> {
        // Simple hash-based caching
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        asm_content.hash(&mut hasher);
        let hash = hasher.finish();

        let elf_path = self.cache_dir.join(format!("firmware_{:x}.elf", hash));
        if elf_path.exists() {
            return Ok(elf_path);
        }

        info!("Compiling cached firmware...");
        let tmp_s = self.cache_dir.join(format!("tmp_{:x}.S", hash));
        let tmp_ld = self.cache_dir.join(format!("tmp_{:x}.ld", hash));

        std::fs::write(&tmp_s, asm_content)?;
        std::fs::write(
            &tmp_ld,
            "ENTRY(_start)\nSECTIONS { . = 0x40000000; .text : { *(.text*) } }",
        )?;

        let mut cmd = Command::new("arm-none-eabi-gcc");
        cmd.args(["-mcpu=cortex-a15", "-nostdlib", "-T"])
            .arg(&tmp_ld)
            .arg(&tmp_s)
            .arg("-o")
            .arg(&elf_path);

        let status = cmd.status().await?;
        if !status.success() {
            return Err(anyhow!("Failed to compile firmware ASM"));
        }

        let _ = std::fs::remove_file(tmp_s);
        let _ = std::fs::remove_file(tmp_ld);

        Ok(elf_path)
    }

    pub async fn get_dtb_dts(&self, dts_content: &str) -> Result<PathBuf> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        dts_content.hash(&mut hasher);
        let hash = hasher.finish();

        let dtb_path = self.cache_dir.join(format!("board_{:x}.dtb", hash));
        if dtb_path.exists() {
            return Ok(dtb_path);
        }

        info!("Compiling cached DTB...");
        let tmp_dts = self.cache_dir.join(format!("tmp_{:x}.dts", hash));
        std::fs::write(&tmp_dts, dts_content)?;

        let mut cmd = Command::new("dtc");
        cmd.args(["-I", "dts", "-O", "dtb", "-o"])
            .arg(&dtb_path)
            .arg(&tmp_dts);

        let status = cmd.status().await?;
        if !status.success() {
            return Err(anyhow!("Failed to compile DTB"));
        }

        // let _ = std::fs::remove_file(tmp_dts);

        Ok(dtb_path)
    }
}

pub struct GuestApp {
    pub name: String,
    pub elf_path: PathBuf,
    pub dtb_path: PathBuf,
}

impl GuestApp {
    pub fn find(workspace_root: &Path, name: &str) -> Result<Self> {
        let base = workspace_root.join("tests/fixtures/guest_apps").join(name);
        if !base.exists() {
            return Err(anyhow!(
                "Guest app fixture '{}' not found at {}",
                name,
                base.display()
            ));
        }

        // Standard pattern: name/hello.elf or name/name.elf
        let elf_candidates = [base.join("hello.elf"), base.join(format!("{}.elf", name))];

        let dtb_candidates = [base.join("minimal.dtb"), base.join(format!("{}.dtb", name))];

        let elf_path = elf_candidates
            .iter()
            .find(|p| p.exists())
            .cloned()
            .ok_or_else(|| anyhow!("No ELF found for guest app {}", name))?;

        let dtb_path = dtb_candidates
            .iter()
            .find(|p| p.exists())
            .cloned()
            .ok_or_else(|| anyhow!("No DTB found for guest app {}", name))?;

        Ok(Self {
            name: name.to_string(),
            elf_path,
            dtb_path,
        })
    }
}
