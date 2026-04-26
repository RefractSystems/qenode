#!/usr/bin/env python3
"""
jitter_proxy.py — Zenoh clock-advance jitter injection proxy for Phase 16.4.

This proxy sits between the benchmark harness (TimeAuthority) and QEMU's
zenoh-clock device.  It forwards every clock-advance queryable reply with a
random additional delay in [0, MAX_JITTER_US) microseconds, injected via a
simple sleep.

The purpose is to prove that virtmcu's virtual-time gating correctly absorbs
host-side Zenoh jitter: despite the extra wall-clock delay, all firmware runs
must produce byte-perfect identical exit_vtime_ns values.

Usage:
    python3 jitter_proxy.py <upstream_router> <proxy_listen_port> [max_jitter_us]

    upstream_router: URL of the actual Zenoh router used by QEMU
                     e.g. tcp/127.0.0.1:7448
    proxy_listen_port: TCP port this proxy listens on (used by bench harness)
                     e.g. 7449
    max_jitter_us: max random delay in microseconds (default: 200)
"""

import logging
import random
import sys
import threading
import time

import zenoh

logging.basicConfig(level=logging.INFO, format="%(levelname)s: [%(name)s] %(message)s")
logger = logging.getLogger("jitter_proxy")

# Maximum random jitter added per forwarded reply (microseconds).
DEFAULT_MAX_JITTER_US = 200

# Topic pattern for clock-advance queries.
CLOCK_ADVANCE_PREFIX = "sim/clock/advance/"

class JitterProxy:
    """
    Subscribes to clock-advance queryables on the upstream router and
    re-publishes replies with injected jitter on the proxy router.
    """

    def __init__(self, upstream_url: str, proxy_port: int, max_jitter_us: int):
        self.upstream_url = upstream_url
        self.proxy_port = proxy_port
        self.max_jitter_us = max_jitter_us
        self._rng = random.Random()
        self._lock = threading.Lock()
        self.injected_delays_us: list[float] = []
        self._stop_event = threading.Event()
        self._in_flight = 0

    def stop(self) -> None:
        """Gracefully signal the proxy to stop."""
        self._stop_event.set()

    def run(self) -> None:
        proxy_listen = f"tcp/127.0.0.1:{self.proxy_port}"
        logger.info(f"upstream={self.upstream_url} listen={proxy_listen} max_jitter={self.max_jitter_us} µs")

        # Back-end session: acts as the router for QEMU.
        backend_cfg = zenoh.Config()
        backend_cfg.insert_json5("listen/endpoints", f'["{proxy_listen}"]')
        backend_cfg.insert_json5("scouting/multicast/enabled", "false")
        backend_session = zenoh.open(backend_cfg)

        # Front-end session: connects to upstream router.
        frontend_cfg = zenoh.Config()
        frontend_cfg.insert_json5("connect/endpoints", f'["{self.upstream_url}"]')
        frontend_cfg.insert_json5("scouting/multicast/enabled", "false")
        frontend_session = zenoh.open(frontend_cfg)

        def handle_query(query):
            with self._lock:
                self._in_flight += 1
                # 50 concurrent queries is far beyond what our benchmarking harness
                # produces (1 per node). Exceeding this indicates a routing storm.
                if self._in_flight > 50:
                    self._in_flight -= 1
                    logger.error("Infinite recursion or query storm detected! Failing fast.")
                    query.reply_err(b"proxy: routing loop detected")
                    return

            try:
                topic = str(query.key_expr)
                payload = query.payload.to_bytes() if query.payload else b""

                # Forward to QEMU via the backend session.
                replies = list(backend_session.get(topic, payload=payload, timeout=5.0))

                if replies and hasattr(replies[0], "ok") and replies[0].ok is not None:
                    # Inject jitter before replying back to the TimeAuthority.
                    jitter_us = self._rng.uniform(0, self.max_jitter_us)
                    time.sleep(jitter_us / 1_000_000)

                    with self._lock:
                        self.injected_delays_us.append(jitter_us)

                    query.reply(topic, replies[0].ok.payload.to_bytes())
                else:
                    query.reply_err(b"proxy: no QEMU reply")
            finally:
                with self._lock:
                    self._in_flight -= 1

        # Declare a queryable on the frontend (upstream) session.
        queryable = frontend_session.declare_queryable(
            f"{CLOCK_ADVANCE_PREFIX}*",
            handle_query,
        )

        logger.info("ready — waiting for queries")
        try:
            while not self._stop_event.is_set():
                self._stop_event.wait(1.0)
        except KeyboardInterrupt:
            pass
        finally:
            queryable.undeclare()
            frontend_session.close()
            backend_session.close()

            # Prevent division by zero if no delays were injected
            n_delays = len(self.injected_delays_us)
            mean_delay = sum(self.injected_delays_us) / max(1, n_delays)
            logger.info(f"shutdown complete. injected {n_delays} delays, mean={mean_delay:.1f} µs")

def main() -> None:
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)

    upstream_url = sys.argv[1]
    proxy_port = int(sys.argv[2])
    max_jitter_us = int(sys.argv[3]) if len(sys.argv) > 3 else DEFAULT_MAX_JITTER_US

    proxy = JitterProxy(upstream_url, proxy_port, max_jitter_us)
    try:
        proxy.run()
    except KeyboardInterrupt:
        proxy.stop()

if __name__ == "__main__":
    main()
