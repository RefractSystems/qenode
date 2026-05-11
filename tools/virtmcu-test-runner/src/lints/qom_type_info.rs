use anyhow::Result;
use ignore::WalkBuilder;
use std::path::Path;
use tracing::{error, info};

use crate::lints::static_state::Lint;

pub struct QomTypeInfoLint;

impl Lint for QomTypeInfoLint {
    fn name(&self) -> &'static str {
        "qom_type_info"
    }

    fn check(&self, target_dir: &Path) -> Result<bool> {
        let mut passed = true;
        let hw_rust_dir = target_dir.join("hw/rust");

        if !hw_rust_dir.exists() {
            return Ok(true);
        }

        let walker = WalkBuilder::new(&hw_rust_dir)
            .add_custom_ignore_filename(".geminiignore")
            .build();

        for result in walker {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("rs") {
                continue;
            }

            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Use simple regex for TypeInfo blocks for now, matching the Python logic but in Rust
            // because full syn parsing of arbitrary initializers can be complex.
            let type_info_re =
                regex::Regex::new(r"(?s)static\s+\w+:\s*TypeInfo\s*=\s*TypeInfo\s*\{(.*?)\};")
                    .unwrap();

            for match_ in type_info_re.captures_iter(&content) {
                let inner_content = &match_[1];

                let parent_re = regex::Regex::new(r"parent:\s*([^,]+),").unwrap();
                #[allow(clippy::regex_creation_in_loops)]
                let class_size_re = regex::Regex::new(r"class_size:\\s*([^,]+),").unwrap();

                let parent = parent_re
                    .captures(inner_content)
                    .map(|c| c[1].trim().to_string());
                let class_size = class_size_re
                    .captures(inner_content)
                    .map(|c| c[1].trim().to_string());

                if let (Some(p), Some(cs)) = (parent, class_size) {
                    if p.contains("TYPE_SYS_BUS_DEVICE") && !cs.contains("SysBusDeviceClass") {
                        passed = false;
                        let line_no = content[..match_.get(0).unwrap().start()].lines().count() + 1;
                        error!(
                            "{}:{}: TypeInfo has parent TYPE_SYS_BUS_DEVICE but class_size is '{}'.",
                            path.display(), line_no, cs
                        );
                        error!("  Fix: Set class_size to core::mem::size_of::<virtmcu_qom::qdev::SysBusDeviceClass>()");
                    }
                }
            }
        }

        if passed {
            info!("✓ QOM TypeInfo metadata check passed.");
        }

        Ok(passed)
    }
}
