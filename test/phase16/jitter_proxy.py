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
import random
import socket
import sys
import threading
import time

import zenoh

# Maximum random jitter added per forwarded reply (microseconds).
DEFAULT_MAX_JITTER_US = 200

# Topic pattern for clock-advance queries.
CLOCK_ADVANCE_PREFIX = "sim/clock/advance/"


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class JitterProxy:
    """
    Subscribes to clock-advance queryables on the upstream router and
    re-publishes replies with injected jitter on the proxy router.

    Implementation note: Zenoh queryables cannot be trivially proxied because
    the query originator (QEMU) connects to the proxy router, while the actual
    responder (bench.py TimeAuthority) connects to the upstream router.

    Instead this proxy acts as a *forwarding queryable*: it registers a
    queryable on the proxy router, forwards incoming queries to the upstream
    router via session.get(), injects a sleep, then replies to the original
    query.
    """

    def __init__(self, upstream_url: str, proxy_port: int, max_jitter_us: int):
        self.upstream_url = upstream_url
        self.proxy_port = proxy_port
        self.max_jitter_us = max_jitter_us
        self._rng = random.Random()
        self._lock = threading.Lock()
        self.injected_delays_us: list[float] = []

    def _make_session(self, listen_endpoint: str | None = None) -> zenoh.Session:
        config = zenoh.Config()
        config.insert_json5("connect/endpoints", f'["{self.upstream_url}"]')
        config.insert_json5("scouting/multicast/enabled", "false")
        if listen_endpoint:
            config.insert_json5("listen/endpoints", f'["{listen_endpoint}"]')
        return zenoh.open(config)

    def run(self) -> None:
        proxy_listen = f"tcp/127.0.0.1:{self.proxy_port}"
        print(f"[jitter_proxy] upstream={self.upstream_url}  listen={proxy_listen}  "
              f"max_jitter={self.max_jitter_us} µs")

        # Upstream session: used to forward queries to the actual TimeAuthority.
        upstream = self._make_session()

        # Proxy session: QEMU connects here; we intercept its queries.
        proxy_cfg = zenoh.Config()
        proxy_cfg.insert_json5("listen/endpoints", f'["{proxy_listen}"]')
        proxy_cfg.insert_json5("connect/endpoints", f'["{self.upstream_url}"]')
        proxy_cfg.insert_json5("scouting/multicast/enabled", "false")
        proxy_session = zenoh.open(proxy_cfg)

        def handle_query(query):
            topic = str(query.key_expr)
            payload = query.payload.to_bytes() if query.payload else b""

            # Inject jitter before forwarding.
            jitter_us = self._rng.uniform(0, self.max_jitter_us)
            time.sleep(jitter_us / 1_000_000)

            with self._lock:
                self.injected_delays_us.append(jitter_us)

            # Forward to real TimeAuthority on upstream session.
            replies = list(upstream.get(topic, payload=payload, timeout=30.0))
            if replies and hasattr(replies[0], "ok") and replies[0].ok is not None:
                query.reply(topic, replies[0].ok.payload.to_bytes())
            else:
                query.reply_err(b"proxy: no upstream reply")

        # Declare a queryable on all clock-advance topics.
        queryable = proxy_session.declare_queryable(
            f"{CLOCK_ADVANCE_PREFIX}*",
            handle_query,
        )

        print("[jitter_proxy] ready — press Ctrl+C to stop")
        try:
            while True:
                time.sleep(1)
        except KeyboardInterrupt:
            pass
        finally:
            queryable.undeclare()
            proxy_session.close()
            upstream.close()
            print(f"[jitter_proxy] injected {len(self.injected_delays_us)} delays, "
                  f"mean={sum(self.injected_delays_us)/max(1,len(self.injected_delays_us)):.1f} µs")


def main() -> None:
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(1)

    upstream_url = sys.argv[1]
    proxy_port = int(sys.argv[2])
    max_jitter_us = int(sys.argv[3]) if len(sys.argv) > 3 else DEFAULT_MAX_JITTER_US

    proxy = JitterProxy(upstream_url, proxy_port, max_jitter_us)
    proxy.run()


if __name__ == "__main__":
    main()
