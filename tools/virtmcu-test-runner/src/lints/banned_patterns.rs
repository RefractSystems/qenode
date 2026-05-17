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
                fix: "Replace with condvar/channel. Quick Tip: Avoid sleeping which breaks deterministic virtual time. See docs/guide/09-engineering-mandates.md. MANDATE: // virtmcu-allow: sleep reasoning=\"<reason>\" inline.",
                inc_dirs: vec!["hw/rust"],
                exc_list: vec![],
            },
            Rule {
                name: "mutex",
                pattern: r"Mutex<",
                message: "Banned Mutex<T> found in peripheral state.",
                fix: "Replace std::sync::Mutex<T> with virtmcu_qom::sync::Mutex<T> or Atomics. Quick Tip: Standard Mutex deadlocks with the BQL. BqlGuarded is DEPRECATED. See docs/rfcs/0018-safe-peripheral-bql-yielding.md. MANDATE: // virtmcu-allow: mutex reasoning=\"<reason>\".",
                inc_dirs: vec!["hw/rust/comms", "hw/rust/mcu", "hw/rust/observability"],
                exc_list: vec![],
            },
            Rule {
                name: "bql",
                pattern: r"Bql::lock\(\)|SafeSubscription",
                message: "Banned BQL usage or SafeSubscription found.",
                fix: "Use VtimeIngress (RFC-0021) instead of SafeSubscription. Use virtmcu_qom::sync::Mutex for state. Quick Tip: Do not block async threads on the BQL. See docs/rfcs/0018-safe-peripheral-bql-yielding.md and RFC-0021. MANDATE: // virtmcu-allow: bql reasoning=\"<reason>\".",
                inc_dirs: vec!["hw/rust/comms", "hw/rust/mcu", "hw/rust/observability"],
                exc_list: vec![],
            },
            Rule {
                name: "no_std",
                pattern: r"#\!\[no_std\]",
                message: "Misleading #![no_std] found.",
                fix: "Remove #![no_std]. Quick Tip: VirtMCU plugins run in userspace and rely on std. See docs/guide/09-engineering-mandates.md. MANDATE: // virtmcu-allow: no_std reasoning=\"<reason>\" inline.",
                inc_dirs: vec!["hw/rust"],
                exc_list: vec![],
            },
            Rule {
                name: "ne_bytes",
                pattern: r"to_ne_bytes|from_ne_bytes",
                message: "Banned to_ne_bytes/from_ne_bytes found.",
                fix: "Use to_le_bytes()/from_le_bytes() with a wire-order comment. Quick Tip: Simulation state must be endian-independent. See docs/rfcs/0012-data-serialization.md. MANDATE: // virtmcu-allow: ne_bytes reasoning=\"<reason>\".",
                inc_dirs: vec!["hw/rust"],
                exc_list: vec![],
            },
            Rule {
                name: "rng",
                pattern: r"rand::thread_rng\b",
                message: "Banned rand::thread_rng found.",
                fix: "Use seed_for_quantum() from transport-zenoh. Quick Tip: Wall-clock/random seeding destroys determinism. See docs/rfcs/0020-deterministic-test-orchestration-seeding.md. MANDATE: // virtmcu-allow: rng reasoning=\"<reason>\".",
                inc_dirs: vec!["hw/rust"],
                exc_list: vec![],
            },
            Rule {
                name: "yield",
                pattern: r"std::thread::yield_now\b",
                message: "Banned thread::yield_now() found.",
                fix: "Replace with QemuCond::wait_yielding_bql or the new MmioDevice trait. Quick Tip: Explicit yields can starve the QEMU main loop. See docs/rfcs/0018-safe-peripheral-bql-yielding.md. MANDATE: // virtmcu-allow: yield reasoning=\"<reason>\" inline.",
                inc_dirs: vec!["hw/rust"],
                exc_list: vec![],
            },
            Rule {
                name: "allow",
                pattern: r"#\[allow\(",
                message: "Banned #[allow(] found in production code.",
                fix: "Fix the underlying lint instead. Quick Tip: Strict lints are required for Enterprise SOTA quality. See docs/guide/09-engineering-mandates.md. MANDATE: // virtmcu-allow: allow reasoning=\"<reason>\" inline (for tests only).",
                inc_dirs: vec!["hw/rust"],
                exc_list: vec!["target", "tests", "_generated.rs", "build.rs"],
            },
            Rule {
                name: "unwrap_or_fallback",
                pattern: r"\.unwrap_or\((0|0\.0|u(8|16|32|64)::MAX|\[0[^\]]*\])\)",
                message: "Banned unwrap_or(0/MAX/default) found.",
                fix: "Fail Loudly! Use expect() or propagate errors instead of silently using defaults. Quick Tip: Silent fallbacks cause divergence. See docs/rfcs/0022-fail-loudly-and-panic-linting.md. MANDATE: // virtmcu-allow: unwrap_or_fallback reasoning=\"<reason>\".",
                inc_dirs: vec!["hw/rust", "tools"],
                exc_list: vec!["tests"],
            },
            Rule {
                name: "warn_macro",
                pattern: r"(sim_warn!|tracing::warn!|log::warn!)",
                message: "Banned warn! macro found.",
                fix: "Fail Loudly! Use assert!, panic!, or sim_panic! instead. Quick Tip: Warning without crashing breaks determinism. See docs/rfcs/0022-fail-loudly-and-panic-linting.md. MANDATE: // virtmcu-allow: warn_macro reasoning=\"<reason>\".",
                inc_dirs: vec!["hw/rust", "tools"],
                exc_list: vec!["virtmcu-test-runner", "virtmcu-cli"],
            },
            Rule {
                name: "test_sleep",
                pattern: r"(tokio::time::sleep|std::thread::sleep)\(",
                message: "Banned sleep in integration tests.",
                fix: "Use wait_for_output_passive or async signals instead of polling/sleeping. Quick Tip: Deterministic tests must be event-driven. See docs/rfcs/0020-deterministic-test-orchestration-seeding.md. MANDATE: // virtmcu-allow: test_sleep reasoning=\"<reason>\".",
                inc_dirs: vec!["tests/native_integration"],
                exc_list: vec![],
            },
            Rule {
                name: "test_hardcoded_path",
                pattern: r#""(?:/tmp/|/var/tmp)[\w\-\.]*""#,
                message: "Banned hardcoded temp path in tests.",
                fix: "Use env.tmp_path() to avoid parallel collisions. Quick Tip: Tests run concurrently, hardcoded paths cause cross-talk.",
                inc_dirs: vec!["tests/native_integration"],
                exc_list: vec![],
            },
            Rule {
                name: "test_hardcoded_port",
                pattern: r#""127\.0\.0\.1:[1-9]\d{0,3}""#,
                message: "Banned hardcoded port in tests.",
                fix: "Use port 0 for OS-assigned unique port. Quick Tip: Prevents 'address already in use' errors in parallel CI.",
                inc_dirs: vec!["tests/native_integration"],
                exc_list: vec![],
            },
            Rule {
                name: "test_declare_subscriber",
                pattern: r"\.declare_subscriber\(",
                message: "Banned raw declare_subscriber in tests.",
                fix: "Use env.safe_subscribe() which waits for Zenoh discovery readiness. Quick Tip: Raw pub/sub races against Zenoh scout intervals.",
                inc_dirs: vec!["tests/native_integration"],
                exc_list: vec!["tools/virtmcu-test-runner"],
            },
            Rule {
                name: "env_in_peripheral",
                // Matches two related violations:
                //   1. UdsDataTransport::new( — the bare constructor silently reads
                //      VIRTMCU_SIM_ID; new_with_fed_id( is NOT matched (no paren after "new").
                //   2. "VIRTMCU_SIM_ID" — any direct read of the federation env var.
                // Other env vars (VIRTMCU_TRANSPORT, VIRTMCU_ZENOH_ROUTER, etc.) are allowed.
                // transport-uds is excluded: new() itself is the one permitted holder.
                pattern: r#"UdsDataTransport::new\(|"VIRTMCU_SIM_ID""#,
                message: "Banned VIRTMCU_SIM_ID read or UdsDataTransport::new() in peripheral code.",
                fix: "Use UdsDataTransport::new_with_fed_id(path, node_id, fed_id) where fed_id \
                      comes from the peripheral's 'federation-id' QOM property. \
                      new() silently reads VIRTMCU_SIM_ID — violates DI and races in concurrent \
                      tests. See AGENTS.md §4 'Env Var Reads in Peripherals Are BANNED'. \
                      MANDATE: // virtmcu-allow: env_in_peripheral reasoning=\"<reason>\".",
                inc_dirs: vec!["hw/rust"],
                exc_list: vec!["transport-uds", "tests", "build.rs"],
            },
            Rule {
                name: "topic_qom_property",
                pattern: r#"(?:pub\s+topic\s*:\s*(?:QomString|virtmcu_qom::qom::QomString)|define_prop_\w+\s*!\s*\(\s*"topic")"#,
                message: "Banned `topic` QOM property declaration in peripheral code (RFC-0042).",
                fix: "Replace `topic` with `link-name` (a QomString property). In realize(), call \
                      transport.register_link(node_id, &link_name, protocol, LinkRole::Both) to obtain \
                      a link_id. Use VtimeIngress::new_for_link(link_id, …) for ingress and \
                      transport.reserve_link(link_id, size) for egress. The deprecated \
                      DataTransport::reserve(topic, size) and VtimeIngress::new(topic, …) APIs must \
                      not appear in new code. See RFC-0042 and hw/rust/examples/reference-peripheral \
                      for the Gold Standard pattern. \
                      MANDATE: // virtmcu-allow: topic_qom_property reasoning=\"<reason>\".",
                inc_dirs: vec![
                    "hw/rust/buses",
                    "hw/rust/bridges",
                    "hw/rust/physics",
                    "hw/rust/mcu",
                    "hw/rust/examples",
                    "hw/rust/observability",
                ],
                exc_list: vec![],
            },
            Rule {
                name: "new_unchecked_in_peripheral",
                pattern: r"BqlContext::new_unchecked\(\)",
                message: "Banned BqlContext::new_unchecked() in peripheral code.",
                fix: "BqlContext is created only by the framework (macro dispatch, ClosureTimer \
                      trampoline). Your Peripheral::read/write/realize receives ctx: &BqlContext — \
                      pass it down. Creating one here bypasses RFC-0041 compile-time BQL proof. \
                      See RFC-0041. MANDATE: // virtmcu-allow: new_unchecked_in_peripheral \
                      reasoning=\"<reason>\".",
                inc_dirs: vec![
                    "hw/rust/buses",
                    "hw/rust/physics",
                    "hw/rust/mcu",
                    "hw/rust/examples",
                    "hw/rust/observability",
                    "hw/rust/bridges",
                ],
                exc_list: vec![
                    "hw/rust/observability/telemetry",
                    "hw/rust/observability/tcg-tracer",
                    "hw/rust/bridges/mmio-socket-bridge",
                    "hw/rust/bridges/remote-port",
                    "hw/rust/bridges/uart",
                    "hw/rust/buses/ieee802154",
                    "hw/rust/buses/flexray",
                    "hw/rust/buses/ethernet",
                ],
            },
            Rule {
                name: "unsafe_in_peripheral",
                pattern: r"\bunsafe\s*\{",
                message: "Banned unsafe block in peripheral code (RFC-0026).",
                fix: "Peripheral code must be zero-unsafe. Use framework APIs: ClosureTimer \
                      (instead of extern \"C\" callbacks), dynamic_cast_qom (instead of \
                      deref_qom_ptr), VtimeIngress. See RFC-0026. \
                      MANDATE: // virtmcu-allow: unsafe_in_peripheral reasoning=\"<reason>\".",
                inc_dirs: vec![
                    "hw/rust/buses",
                    "hw/rust/physics",
                    "hw/rust/mcu",
                    "hw/rust/examples",
                    "hw/rust/observability",
                    "hw/rust/bridges",
                ],
                exc_list: vec![
                    "hw/rust/observability/telemetry",
                    "hw/rust/observability/tcg-tracer",
                    "hw/rust/bridges/mmio-socket-bridge",
                    "hw/rust/bridges/remote-port",
                    "hw/rust/bridges/uart",
                    "hw/rust/buses/ieee802154",
                    "hw/rust/buses/flexray",
                    "hw/rust/buses/ethernet",
                ],
            },
            Rule {
                name: "clippy_all_in_ref_peripheral",
                // The Gold Standard crate must be fully lint-clean without blanket suppression.
                // #![allow(clippy::all)] defeats the entire workspace lint enforcement system and
                // is the most dangerous escape hatch an agent can add. Zero-suppress tolerance here.
                pattern: r"#!\[allow\(clippy::all",
                message: "Banned #![allow(clippy::all)] in the Gold Standard reference peripheral.",
                fix: "Fix the underlying clippy lint instead of suppressing all of clippy. \
                      The reference peripheral must be lint-clean with only narrow suppressions \
                      (e.g. clippy::panic with virtmcu-allow) — never a blanket allow(clippy::all). \
                      This crate is the Gold Standard template; blanket suppression here corrupts \
                      every future peripheral that copies it.",
                inc_dirs: vec!["hw/rust/examples/reference-peripheral"],
                exc_list: vec![],
            },
            Rule {
                name: "deprecated_cast_in_ref_peripheral",
                // Bans the four deprecated opaque-pointer recovery helpers in the reference
                // peripheral, which is the Gold Standard template. These are replaced by
                // dynamic_cast_qom (RFC-0041). Suppression is BANNED here — the reference
                // peripheral must be clean.
                pattern: r"\b(?:deref_qom_ptr|opaque_to_state)(?:_const)?\s*[(:!]",
                message: "Banned deprecated QOM cast in hw/rust/examples/reference-peripheral.",
                fix: "Replace deref_qom_ptr / opaque_to_state with \
                      dynamic_cast_qom::<T>(ptr).expect(\"FATAL: QOM type mismatch\"). \
                      The reference peripheral is the Gold Standard template and must be \
                      fully migrated. See RFC-0041.",
                inc_dirs: vec!["hw/rust/examples/reference-peripheral"],
                exc_list: vec![],
            },
            Rule {
                name: "drain_token_in_ref_peripheral",
                // Bans the old DrainToken type name in the reference peripheral. The correct
                // type is VcpuDrain (RFC-0041). Suppression is BANNED here.
                pattern: r"\bDrainToken\b",
                message: "Banned DrainToken in hw/rust/examples/reference-peripheral.",
                fix: "Replace DrainToken with VcpuDrain. The Peripheral trait methods now \
                      receive ctx: &BqlContext — the BQL proof is separate from the drain \
                      guard. See RFC-0041 and the VcpuDrain docs.",
                inc_dirs: vec!["hw/rust/examples/reference-peripheral"],
                exc_list: vec![],
            },
            Rule {
                name: "extern_c_timer_cb",
                // Matches `extern "C" fn name(anything: *mut c_void)` with void return —
                // the exact signature of a QEMU timer callback. Does NOT match MMIO shims
                // (which take offset/size args) or netdev/SSI callbacks (different arg types).
                pattern: r#"extern\s+"C"\s+fn\s+\w+\s*\(\s*\w+\s*:\s*\*mut\s+(?:core::ffi::)?c_void\s*\)"#,
                message: "Banned extern \"C\" timer callback in peripheral code.",
                fix: "Replace with ClosureTimer::new(clock_type, move |ctx| { ... }). \
                      The closure receives &BqlContext automatically; opaque pointer recovery \
                      (opaque_to_state) is no longer needed. See RFC-0041 Tier 2 and P0.3. \
                      MANDATE: // virtmcu-allow: extern_c_timer_cb reasoning=\"<reason>\".",
                inc_dirs: vec![
                    "hw/rust/buses",
                    "hw/rust/observability",
                    "hw/rust/mcu",
                ],
                exc_list: vec![],
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
            info!("Rust banned patterns lint passed.");
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
