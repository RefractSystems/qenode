use anyhow::Result;
use ignore::WalkBuilder;
use regex::Regex;
use std::path::Path;
use tracing::{error, info};

use crate::lints::static_state::Lint;

pub struct RustBannedPatternsLint;

struct Rule {
    name: &'static str,
    pattern: &'static str,
    message: &'static str,
    fix: &'static str,
    inc_dirs: Vec<&'static str>,
    exc_list: Vec<&'static str>,
}

impl Lint for RustBannedPatternsLint {
    fn name(&self) -> &'static str {
        "rust_banned_patterns"
    }

    fn check(&self, target_dir: &Path) -> Result<bool> {
        let mut passed = true;

        let rules = vec![
            Rule {
                name: "sleep",
                pattern: r"thread::sleep",
                message: "Banned thread::sleep found.",
                fix: "Replace with condvar/channel. MANDATE: // virtmcu-allow: sleep reasoning=\"<reason>\" inline.",
                inc_dirs: vec!["hw/rust"],
                exc_list: vec![],
            },
            Rule {
                name: "mutex",
                pattern: r"Mutex<",
                message: "Banned Mutex<T> in peripheral state.",
                fix: "Replace Mutex<T> with BqlGuarded<T> from virtmcu_qom::sync. MANDATE: // virtmcu-allow: mutex reasoning=\"<reason>\".",
                inc_dirs: vec!["hw/rust/comms", "hw/rust/mcu", "hw/rust/observability"],
                exc_list: vec![],
            },
            Rule {
                name: "bql",
                pattern: r"Bql::lock\(\)|SafeSubscription",
                message: "Banned BQL usage found in async/comms paths.",
                fix: "Remove Bql::lock()/SafeSubscription, or use lock-free channels. MANDATE: // virtmcu-allow: bql reasoning=\"<reason>\".",
                inc_dirs: vec!["hw/rust/comms"],
                exc_list: vec![],
            },
            Rule {
                name: "no_std",
                pattern: r"#\!\[no_std\]",
                message: "Misleading #![no_std] found.",
                fix: "Remove #![no_std]. MANDATE: // virtmcu-allow: no_std reasoning=\"<reason>\" inline.",
                inc_dirs: vec!["hw/rust"],
                exc_list: vec![],
            },
            Rule {
                name: "ne_bytes",
                pattern: r"to_ne_bytes|from_ne_bytes",
                message: "Banned to_ne_bytes/from_ne_bytes found.",
                fix: "Use to_le_bytes()/from_le_bytes() with a wire-order comment. MANDATE: // virtmcu-allow: ne_bytes reasoning=\"<reason>\".",
                inc_dirs: vec!["hw/rust"],
                exc_list: vec![],
            },
            Rule {
                name: "rng",
                pattern: r"rand::thread_rng\b",
                message: "Banned rand::thread_rng found.",
                fix: "Use seed_for_quantum() from transport-zenoh. MANDATE: // virtmcu-allow: rng reasoning=\"<reason>\".",
                inc_dirs: vec!["hw/rust"],
                exc_list: vec![],
            },
            Rule {
                name: "yield",
                pattern: r"std::thread::yield_now\b",
                message: "Banned thread::yield_now() found.",
                fix: "Replace with QemuCond::wait_yielding_bql or the new MmioDevice trait. MANDATE: // virtmcu-allow: yield reasoning=\"<reason>\" inline.",
                inc_dirs: vec!["hw/rust"],
                exc_list: vec![],
            },
            Rule {
                name: "allow",
                pattern: r"#\[allow\(",
                message: "Banned #[allow(] found in production code.",
                fix: "Fix the underlying lint instead. MANDATE: // virtmcu-allow: allow reasoning=\"<reason>\" inline (for tests only).",
                inc_dirs: vec!["hw/rust"],
                exc_list: vec!["target", "tests", "_generated.rs", "build.rs"],
            },
            Rule {
                name: "test_sleep",
                pattern: r"(tokio::time::sleep|std::thread::sleep)\(",
                message: "Banned sleep in integration tests.",
                fix: "Use wait_for_output_passive or async signals instead of polling/sleeping. MANDATE: // virtmcu-allow: test_sleep reasoning=\"<reason>\".",
                inc_dirs: vec!["tests/native_integration"],
                exc_list: vec![],
            },
            Rule {
                name: "test_hardcoded_path",
                pattern: r#""(?:/tmp/|/var/tmp)[\w\-\.]*""#,
                message: "Banned hardcoded temp path in tests.",
                fix: "Use env.tmp_path() to avoid parallel collisions.",
                inc_dirs: vec!["tests/native_integration"],
                exc_list: vec![],
            },
            Rule {
                name: "test_hardcoded_port",
                pattern: r#""127\.0\.0\.1:[1-9]\d{0,3}""#,
                message: "Banned hardcoded port in tests.",
                fix: "Use port 0 for OS-assigned unique port.",
                inc_dirs: vec!["tests/native_integration"],
                exc_list: vec![],
            },
            Rule {
                name: "test_declare_subscriber",
                pattern: r"\.declare_subscriber\(",
                message: "Banned raw declare_subscriber in tests.",
                fix: "Use env.safe_subscribe() which waits for Zenoh discovery readiness.",
                inc_dirs: vec!["tests/native_integration"],
                exc_list: vec!["tools/virtmcu-test-runner"],
            },
        ];

        let compiled_rules: Vec<(Rule, Regex)> = rules
            .into_iter()
            .map(|r| {
                let re = Regex::new(r.pattern).unwrap();
                (r, re)
            })
            .collect();

        let walker = WalkBuilder::new(target_dir)
            .add_custom_ignore_filename(".geminiignore")
            .build();

        let spinloop_re = Regex::new(r"(?m)(?:while|loop|for)[\s\S]{1,1000}?(?:attempts|retry|count)\s*[-+]=\s*1[\s\S]{1,1000}?(?:thread::sleep|yield_now|cvar\.wait_timeout)").unwrap();
        let drop_bound_re = Regex::new(r"(?m)impl\s+Drop\s+for[\s\S]{1,500}?(?:while|loop|for)[\s\S]{1,200}?(?:attempts|retry|count)\s*<").unwrap();

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

            let path_str = path.to_str().unwrap_or("");
            let lines: Vec<&str> = content.lines().collect();

            for (rule, re) in &compiled_rules {
                if !rule.inc_dirs.is_empty() && !rule.inc_dirs.iter().any(|d| path_str.contains(d))
                {
                    continue;
                }

                if rule.exc_list.iter().any(|e| path_str.contains(e)) {
                    continue;
                }

                for (i, line) in lines.iter().enumerate() {
                    if re.is_match(line) {
                        if is_suppressed(&lines, i, rule.name) {
                            continue;
                        }
                        passed = false;
                        error!(
                            "{}:{}: {}\n  Fix: {}",
                            path.display(),
                            i + 1,
                            rule.message,
                            rule.fix
                        );
                    }
                }
            }

            // Multi-line lints
            if path_str.contains("hw/rust") {
                // Spinloop
                if let Some(m) = spinloop_re.find(&content) {
                    if !is_suppressed_multiline(&content, m.start(), "spinloop") {
                        passed = false;
                        let line_no = content[..m.start()].lines().count() + 1;
                        error!(
                            "{}:{}: Banned bounded spinloop found.\n  Fix: Use Condvar for waiting without iteration bounds (the Drain pattern) to avoid time-bomb UAF.",
                            path.display(), line_no
                        );
                    }
                }

                // Drop bound
                if let Some(m) = drop_bound_re.find(&content) {
                    if !is_suppressed_multiline(&content, m.start(), "drop_bound") {
                        passed = false;
                        let line_no = content[..m.start()].lines().count() + 1;
                        error!(
                            "{}:{}: Banned bounded loop in Drop implementation.\n  Fix: Drop implementations must use the Drain pattern and NOT use bounded iteration limits.",
                            path.display(), line_no
                        );
                    }
                }
            }
        }

        if passed {
            info!("✓ Rust banned patterns lint passed.");
        }

        Ok(passed)
    }
}

fn is_suppressed(lines: &[&str], line_idx: usize, rule: &str) -> bool {
    let suppression_pattern = format!("// virtmcu-allow: {}", rule);

    if line_idx < lines.len() && lines[line_idx].contains(&suppression_pattern) {
        return true;
    }

    if line_idx > 0 && lines[line_idx - 1].contains(&suppression_pattern) {
        return true;
    }

    false
}

fn is_suppressed_multiline(content: &str, pos: usize, rule: &str) -> bool {
    let suppression_pattern = format!("// virtmcu-allow: {}", rule);
    let start = pos.saturating_sub(100);
    content[start..pos].contains(&suppression_pattern)
}
