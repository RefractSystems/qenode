#!/usr/bin/env python3
"""
perf_trend.py — Performance trend tracking for virtmcu .

Reads the benchmark results produced by tests/fixtures/guest_apps/perf_bench/bench.py
(tests/fixtures/guest_apps/perf_bench/last_results.json) and compares them against a saved baseline.

Usage:
    python3 scripts/perf_trend.py --save-baseline    # persist current results as baseline
    python3 scripts/perf_trend.py --check            # compare against baseline; exit 1 on regression
    python3 scripts/perf_trend.py --show             # print baseline and current side-by-side

Regression thresholds (PLAN §16.5):
    MIPS regression:     > 5 %  compared to baseline → fail
    P99 latency increase: > 10 % compared to baseline → fail
"""

import argparse
import json
import logging
import sys
from pathlib import Path

logger = logging.getLogger(__name__)

WORKSPACE = Path(__file__).resolve().parent.parent

RESULTS_PATH = Path(WORKSPACE) / "tests" / "fixtures" / "guest_apps" / "perf_bench" / "last_results.json"
BASELINE_PATH = Path(WORKSPACE) / "tests" / "fixtures" / "guest_apps" / "perf_bench" / "baseline.json"

# Regression thresholds.
MIPS_REGRESSION_PCT = 5.0  # fail if MIPS drops by more than this percentage
P99_REGRESSION_PCT = 10.0  # fail if P99 latency increases by more than this percentage


def load_json(path: str) -> list[dict]:
    with Path(path).open() as f:
        return json.load(f)


def extract_mips(records: list[dict]) -> dict[str, float]:
    """Return {mode: mips} for all IPS records."""
    return {r["mode"]: r["mips"] for r in records if "mips" in r}


def extract_latency(records: list[dict]) -> dict | None:
    """Return the first latency record (contains p50_us / p99_us / stalls)."""
    for r in records:
        if "p99_us" in r:
            return r
    return None


def check_regression(baseline: list[dict], current: list[dict]) -> list[str]:
    """Return a list of failure messages; empty means no regressions."""
    failures = []

    baseline_mips = extract_mips(baseline)
    current_mips = extract_mips(current)

    for mode, base_val in baseline_mips.items():
        if mode not in current_mips:
            continue
        cur_val = current_mips[mode]
        if base_val > 0:
            drop_pct = (base_val - cur_val) / base_val * 100
            if drop_pct > MIPS_REGRESSION_PCT:
                failures.append(
                    f"MIPS regression in '{mode}': {cur_val:.1f} vs baseline {base_val:.1f} "
                    f"(drop {drop_pct:.1f}% > threshold {MIPS_REGRESSION_PCT}%)"
                )

    base_lat = extract_latency(baseline)
    cur_lat = extract_latency(current)

    if base_lat and cur_lat:
        base_p99 = base_lat.get("p99_us", 0)
        cur_p99 = cur_lat.get("p99_us", 0)
        if base_p99 > 0:
            increase_pct = (cur_p99 - base_p99) / base_p99 * 100
            if increase_pct > P99_REGRESSION_PCT:
                failures.append(
                    f"P99 latency regression: {cur_p99:.0f} µs vs baseline {base_p99:.0f} µs "
                    f"(increase {increase_pct:.1f}% > threshold {P99_REGRESSION_PCT}%)"
                )

    return failures


def print_comparison(baseline: list[dict], current: list[dict]) -> None:
    logger.info(f"{'Metric':<30} {'Baseline':>12} {'Current':>12} {'Change':>10}")
    logger.info("-" * 66)

    bm = extract_mips(baseline)
    cm = extract_mips(current)
    for mode in sorted(set(list(bm) + list(cm))):
        bv = bm.get(mode, 0)
        cv = cm.get(mode, 0)
        change = f"{(cv - bv) / bv * 100:+.1f}%" if bv > 0 else "N/A"
        logger.info(f"  MIPS [{mode:<18}] {bv:>12.1f} {cv:>12.1f} {change:>10}")

    bl = extract_latency(baseline)
    cl = extract_latency(current)
    if bl and cl:
        for key in ("p50_us", "p99_us"):
            bv = bl.get(key, 0)
            cv = cl.get(key, 0)
            change = f"{(cv - bv) / bv * 100:+.1f}%" if bv > 0 else "N/A"
            logger.info(f"  {key:<30} {bv:>12.1f} {cv:>12.1f} {change:>10}")
        stalls_b = bl.get("stalls", 0)
        stalls_c = cl.get("stalls", 0)
        logger.info(f"  {'stalls':<30} {stalls_b:>12} {stalls_c:>12}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--save-baseline", action="store_true", help="Copy last_results.json → baseline.json")
    group.add_argument(
        "--check", action="store_true", help="Compare current results against baseline; exit 1 on regression"
    )
    group.add_argument("--show", action="store_true", help="Print baseline vs current side-by-side")
    args = parser.parse_args()

    if args.save_baseline:
        if not Path(RESULTS_PATH).exists():
            logger.error(f"ERROR: {RESULTS_PATH} not found — run bench.py first")
            sys.exit(1)
        import shutil

        shutil.copy2(RESULTS_PATH, BASELINE_PATH)
        current = load_json(RESULTS_PATH)
        logger.info(f"Baseline saved to {BASELINE_PATH}")
        mips = extract_mips(current)
        for mode, val in mips.items():
            logger.info(f"  {mode}: {val:.1f} MIPS")
        lat = extract_latency(current)
        if lat:
            logger.info(
                f"  P50={lat.get('p50_us', 0):.0f} µs  P99={lat.get('p99_us', 0):.0f} µs  stalls={lat.get('stalls', 0)}"
            )
        return

    if not Path(BASELINE_PATH).exists():
        logger.error(f"ERROR: {BASELINE_PATH} not found — run with --save-baseline first")
        sys.exit(1)
    if not Path(RESULTS_PATH).exists():
        logger.error(f"ERROR: {RESULTS_PATH} not found — run bench.py first")
        sys.exit(1)

    baseline = load_json(BASELINE_PATH)
    current = load_json(RESULTS_PATH)

    if args.show:
        print_comparison(baseline, current)
        return

    # --check
    print_comparison(baseline, current)
    failures = check_regression(baseline, current)
    if failures:
        logger.info("")
        for msg in failures:
            logger.info(f"REGRESSION: {msg}")
        sys.exit(1)
    else:
        logger.info("\nNo regressions detected.")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    main()
