"""
Contract tests for the SimTopic registry.

This file is the ONE place in the codebase permitted to compare `SimTopic.*`
output against literal Zenoh topic strings — it pins the schema that the Rust
coordinator's subscriber wildcards depend on. The lint rule in
`scripts/lint_simulation_usage.py` exempts this file by name.

If you change a literal here, you MUST also update the matching wildcard in
`tools/deterministic_coordinator/src/main.rs` and the Rust topic enum once it
exists. A divergence will silently break parallel-test message routing.
"""

from __future__ import annotations

from tools.testing.virtmcu_test_suite.topics import SimTopic


class TestSimTopicContract:
    def test_chardev_rx_topic_node0(self) -> None:
        assert SimTopic.chardev_rx(0) == "sim/chardev/0/rx"

    def test_chardev_tx_topic_node0(self) -> None:
        assert SimTopic.chardev_tx(0) == "sim/chardev/0/tx"

    def test_chardev_rx_tx_distinct(self) -> None:
        assert SimTopic.chardev_rx(0) != SimTopic.chardev_tx(0)

    def test_chardev_multi_node_isolation(self) -> None:
        assert SimTopic.chardev_rx(0) != SimTopic.chardev_rx(1)

    def test_clock_advance_topic(self) -> None:
        assert SimTopic.clock_advance(0) == "sim/clock/advance/0"
        assert SimTopic.clock_advance(3) == "sim/clock/advance/3"

    def test_clock_heartbeat_topic(self) -> None:
        assert SimTopic.clock_heartbeat(0) == "sim/clock/heartbeat/0"

    def test_clock_liveliness_topic(self) -> None:
        assert SimTopic.clock_liveliness(0) == "sim/clock/liveliness/0"

    def test_clock_vtime_topic(self) -> None:
        assert SimTopic.clock_vtime(0) == "sim/clock/vtime/0"

    def test_netdev_rx_topic(self) -> None:
        assert SimTopic.netdev_rx(0) == "sim/netdev/0/rx"

    def test_coord_alive_singleton(self) -> None:
        assert SimTopic.COORD_ALIVE == "sim/coord/alive"

    def test_router_check_singleton(self) -> None:
        assert SimTopic.ROUTER_CHECK == "sim/router/check"

    def test_network_control_singleton(self) -> None:
        assert SimTopic.NETWORK_CONTROL == "sim/network/control"

    def test_coord_per_node_topics(self) -> None:
        assert SimTopic.coord_tx(2) == "sim/coord/2/tx"
        assert SimTopic.coord_rx(2) == "sim/coord/2/rx"
        assert SimTopic.coord_done(2) == "sim/coord/2/done"

    def test_eth_topics(self) -> None:
        assert SimTopic.eth_tx(0) == "sim/eth/frame/0/tx"
        assert SimTopic.eth_rx(1) == "sim/eth/frame/1/rx"

    def test_uart_namespace_split(self) -> None:
        # Guest-facing peripheral lives under virtmcu/uart/...
        assert SimTopic.uart_tx(0) == "virtmcu/uart/0/tx"
        assert SimTopic.uart_rx(0) == "virtmcu/uart/0/rx"
        # Test-coordinator simulated UART lives under sim/uart/...
        assert SimTopic.sim_uart_tx(0) == "sim/uart/0/tx"
        assert SimTopic.sim_uart_rx(0) == "sim/uart/0/rx"

    def test_wildcard_subscribers(self) -> None:
        # These wildcards must match the Rust coordinator's `legacy_tx_topics`
        # in tools/deterministic_coordinator/src/main.rs.
        assert SimTopic.COORD_DONE_WILDCARD == "sim/coord/*/done"
        assert SimTopic.COORD_TX_WILDCARD == "sim/coord/*/tx"
        assert SimTopic.ETH_FRAME_TX_WILDCARD == "sim/eth/frame/*/tx"

    def test_topic_no_trailing_slash(self) -> None:
        for topic in [
            SimTopic.chardev_rx(0),
            SimTopic.chardev_tx(0),
            SimTopic.clock_advance(0),
            SimTopic.coord_tx(0),
            SimTopic.eth_rx(0),
        ]:
            assert not topic.endswith("/"), f"topic has trailing slash: {topic}"

    def test_node_id_int_or_str_accepted(self) -> None:
        for node in [0, 1, 15, 255]:
            topic = SimTopic.chardev_rx(node)
            assert f"/{node}/" in topic
        # str inputs (e.g. unique-id prefixes) also accepted
        assert SimTopic.chardev_rx("alpha") == "sim/chardev/alpha/rx"
