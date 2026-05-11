use anyhow::Result;
use ignore::WalkBuilder;
use regex::Regex;
use std::path::Path;
use tracing::{error, info};

use crate::lints::static_state::Lint;

pub struct RustMagicNumbersLint;

impl Lint for RustMagicNumbersLint {
    fn name(&self) -> &'static str {
        "rust_magic_numbers"
    }

    fn check(&self, target_dir: &Path) -> Result<bool> {
        let mut passed = true;
        let hw_rust_dir = target_dir.join("hw/rust");

        if !hw_rust_dir.exists() {
            return Ok(true);
        }

        let allowed_literals = ["0", "1", "0x0", "0x1", "128", "256", "512", "1024"];
        let magic_re = Regex::new(r"\b(0x[0-9a-fA-F]+|[0-9]+)\b").unwrap();
        let const_re = Regex::new(r"^\s*(pub\s+)?(const|static)\s+").unwrap();
        let enum_re = Regex::new(r"^\s*[A-Z][a-zA-Z0-9_]*\s*=\s*[0-9]+").unwrap();

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

            let path_str = path.to_str().unwrap_or("");
            if path_str.contains("/tests/")
                || path_str.ends_with("build.rs")
                || path_str.contains("_generated.rs")
            {
                continue;
            }

            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Strip comments (simple version)
            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let line_no_comment = if let Some(idx) = line.find("//") {
                    &line[..idx]
                } else {
                    line
                };

                if const_re.is_match(line_no_comment)
                    || enum_re.is_match(line_no_comment)
                    || line_no_comment.contains("align(")
                {
                    continue;
                }

                if line_no_comment.contains("error_setg!") {
                    continue;
                }

                for m in magic_re.find_iter(line_no_comment) {
                    let val_str = m.as_str();
                    if allowed_literals.contains(&val_str) {
                        continue;
                    }

                    // Check for array size [0; 128]
                    let array_size_pattern = format!(r"\[\s*.*;\s*{}\s*\]", regex::escape(val_str));
                    let array_size_re = Regex::new(&array_size_pattern).unwrap();
                    if array_size_re.is_match(line_no_comment) {
                        continue;
                    }

                    passed = false;
                    error!(
                        "{}:{}: Magic number '{}' found.\n  Fix: Extract to a named 'const'.",
                        path.display(),
                        i + 1,
                        val_str
                    );
                }
            }
        }

        if passed {
            info!("✓ Rust magic numbers check passed.");
        }

        Ok(passed)
    }
}
