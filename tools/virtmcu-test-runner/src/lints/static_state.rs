use anyhow::Result;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use syn::visit::Visit;
use syn::ItemStatic;
use tracing::{error, info};

pub trait Lint {
    fn name(&self) -> &'static str;
    fn check(&self, target_dir: &Path) -> Result<bool>;
}

pub struct StaticStateLint;

impl Lint for StaticStateLint {
    fn name(&self) -> &'static str {
        "rust_static_state"
    }

    fn check(&self, target_dir: &Path) -> Result<bool> {
        let mut passed = true;
        let hw_rust_dir = target_dir.join("hw/rust");

        if !hw_rust_dir.exists() {
            info!("Skipping static state lint, hw/rust does not exist");
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

            if let Ok(file) = syn::parse_file(&content) {
                let mut visitor = StaticVisitor {
                    path: path.to_path_buf(),
                    content: content.clone(),
                    violations: Vec::new(),
                };
                visitor.visit_file(&file);

                // Check for banned macros using a simple string search for now
                // since they might be expanded in complex ways or part of
                // expressions not easily caught by a simple AST visit without expansion.
                let lines: Vec<&str> = content.lines().collect();
                // virtmcu-allow: static_state reasoning="Linter self-check"
                for macro_name in ["lazy_static!", "thread_local!"] {
                    for (i, line) in lines.iter().enumerate() {
                        if line.contains(macro_name) {
                            if is_suppressed(&lines, i, "static_state") {
                                continue;
                            }
                            let line_num = i + 1;
                            visitor.violations.push(format!(
                                "{}:{}: Banned macro detected ('{}').\n  Fix: Move state into the peripheral struct. MANDATE: // virtmcu-allow: static_state reasoning=\"<reason>\".",
                                path.display(), line_num, macro_name
                            ));
                        }
                    }
                }

                if !visitor.violations.is_empty() {
                    passed = false;
                    for violation in visitor.violations {
                        error!("{}", violation);
                    }
                }
            }
        }

        if passed {
            info!("✓ Rust static state ban lint passed.");
        }

        Ok(passed)
    }
}

struct StaticVisitor {
    path: PathBuf,
    content: String,
    violations: Vec<String>,
}

impl<'ast> Visit<'ast> for StaticVisitor {
    fn visit_item_static(&mut self, node: &'ast ItemStatic) {
        let span = node.ident.span();
        let start_line = span.start().line;

        let lines: Vec<&str> = self.content.lines().collect();

        // Check for suppression on the line itself or the preceding line
        if is_suppressed(&lines, start_line - 1, "static_state") {
            return;
        }

        let is_mut = matches!(node.mutability, syn::StaticMutability::Mut(_));
        let type_str = quote::quote!(#node.ty).to_string().replace(" ", "");

        let banned_types = [
            "Atomic", "Mutex", "OnceCell", "OnceLock", "RwLock", "Cell<", "RefCell<",
        ];
        let has_banned_type = banned_types.iter().any(|&t| type_str.contains(t));

        if is_mut || has_banned_type {
            let actual_type_str = quote::quote!(#node.ty).to_string();
            self.violations.push(format!(
                "{}:{}: Banned static state detected in type: {}.\n  Fix: Move state into the peripheral struct or export from main binary. MANDATE: // virtmcu-allow: static_state reasoning=\"<reason>\".",
                self.path.display(), start_line, actual_type_str
            ));
        }

        // Continue visiting in case there are nested items (rare for static, but good practice)
        syn::visit::visit_item_static(self, node);
    }
}

fn is_suppressed(lines: &[&str], line_idx: usize, rule: &str) -> bool {
    let suppression_pattern = format!("// virtmcu-allow: {}", rule);

    // Check the current line
    if line_idx < lines.len() && lines[line_idx].contains(&suppression_pattern) {
        return true;
    }

    // Check the previous line
    if line_idx > 0 && lines[line_idx - 1].contains(&suppression_pattern) {
        return true;
    }

    false
}
