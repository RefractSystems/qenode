"""
AUTO-GENERATED from topics.toml. DO NOT EDIT MANUALLY.
"""
from __future__ import annotations

from typing import Final


class SimTopic:
    """Enterprise topic registry for the VirtMCU simulation."""

    # Singleton control-plane topics
    COORD_ALIVE: Final[str] = "sim/coord/alive"
    ROUTER_CHECK: Final[str] = "sim/router/check"
    NETWORK_CONTROL: Final[str] = "sim/network/control"

    # Wildcard subscriber patterns
    COORD_DONE_WILDCARD: Final[str] = "sim/coord/*/done"
    COORD_RX_WILDCARD: Final[str] = "sim/coord/*/rx"
    COORD_TX_WILDCARD: Final[str] = "sim/coord/*/tx"
    ETH_FRAME_RX_WILDCARD: Final[str] = "sim/eth/frame/*/rx"
    ETH_FRAME_TX_WILDCARD: Final[str] = "sim/eth/frame/*/tx"
    SIM_UART_TX_WILDCARD: Final[str] = "sim/uart/*/tx"
    SIM_UART_RX_WILDCARD: Final[str] = "sim/uart/*/rx"
    CAN_TX_WILDCARD: Final[str] = "sim/can/*/tx"
    LIN_TX_WILDCARD: Final[str] = "sim/lin/*/tx"
    SIM_SYSTEMC_TX_WILDCARD: Final[str] = "sim/systemc/frame/*/tx"
    SPI_TX_WILDCARD: Final[str] = "sim/spi/*/*/tx"
    RF_IEEE802154_TX_WILDCARD: Final[str] = "sim/rf/ieee802154/*/tx"
    RF_HCI_TX_WILDCARD: Final[str] = "sim/rf/hci/*/tx"
    CHARDEV_TX_WILDCARD: Final[str] = "sim/chardev/*/tx"
    NETDEV_TX_WILDCARD: Final[str] = "sim/netdev/*/tx"
    VIRTM_UART_TX_WILDCARD: Final[str] = "virtmcu/uart/*/tx"
    VIRTM_UART_PORT_TX_WILDCARD: Final[str] = "virtmcu/uart/**/tx"

    # Templates
    @staticmethod
    def clock_advance(node_id: int | str) -> str:
        return f"sim/clock/advance/{node_id}"

    @staticmethod
    def clock_start(node_id: int | str) -> str:
        return f"sim/clock/start/{node_id}"

    @staticmethod
    def clock_vtime(node_id: int | str) -> str:
        return f"sim/clock/vtime/{node_id}"

    @staticmethod
    def clock_liveliness(node_id: int | str) -> str:
        return f"sim/clock/liveliness/{node_id}"

    @staticmethod
    def clock_heartbeat(node_id: int | str) -> str:
        return f"sim/clock/heartbeat/{node_id}"

    @staticmethod
    def clock_unique_prefix(unique_id: int | str) -> str:
        return f"sim/clock/{unique_id}"

    @staticmethod
    def coord_done(node_id: int | str) -> str:
        return f"sim/coord/{node_id}/done"

    @staticmethod
    def coord_rx(node_id: int | str) -> str:
        return f"sim/coord/{node_id}/rx"

    @staticmethod
    def coord_tx(node_id: int | str) -> str:
        return f"sim/coord/{node_id}/tx"

    @staticmethod
    def plugin_liveliness(plugin: int | str, node_id: int | str) -> str:
        return f"sim/{plugin}/liveliness/{node_id}"

    @staticmethod
    def test_probe(suffix: int | str) -> str:
        return f"sim/test/probe/{suffix}"

    @staticmethod
    def eth_tx(node_id: int | str) -> str:
        return f"sim/eth/frame/{node_id}/tx"

    @staticmethod
    def eth_rx(node_id: int | str) -> str:
        return f"sim/eth/frame/{node_id}/rx"

    @staticmethod
    def uart_tx(node_id: int | str) -> str:
        return f"virtmcu/uart/{node_id}/tx"

    @staticmethod
    def uart_rx(node_id: int | str) -> str:
        return f"virtmcu/uart/{node_id}/rx"

    @staticmethod
    def uart_unique_prefix(unique_id: int | str) -> str:
        return f"virtmcu/uart/{unique_id}"

    @staticmethod
    def sim_uart_tx(node_id: int | str) -> str:
        return f"sim/uart/{node_id}/tx"

    @staticmethod
    def sim_uart_rx(node_id: int | str) -> str:
        return f"sim/uart/{node_id}/rx"

    @staticmethod
    def uart_port_tx(node_id: int | str, port_id: int | str) -> str:
        return f"virtmcu/uart/{node_id}/{port_id}/tx"

    @staticmethod
    def uart_port_rx(node_id: int | str, port_id: int | str) -> str:
        return f"virtmcu/uart/{node_id}/{port_id}/rx"

    @staticmethod
    def can_tx(node_id: int | str) -> str:
        return f"sim/can/{node_id}/tx"

    @staticmethod
    def can_rx(node_id: int | str) -> str:
        return f"sim/can/{node_id}/rx"

    @staticmethod
    def lin_tx(node_id: int | str) -> str:
        return f"sim/lin/{node_id}/tx"

    @staticmethod
    def lin_rx(node_id: int | str) -> str:
        return f"sim/lin/{node_id}/rx"

    @staticmethod
    def lin_unique_prefix(unique_id: int | str) -> str:
        return f"sim/lin/{unique_id}"

    @staticmethod
    def flexray_unique_prefix(unique_id: int | str) -> str:
        return f"sim/flexray/{unique_id}"

    @staticmethod
    def spi_base(bus: int | str, node_id: int | str) -> str:
        return f"sim/spi/{bus}/{node_id}"

    @staticmethod
    def rf_ieee802154_tx(node_id: int | str) -> str:
        return f"sim/rf/ieee802154/{node_id}/tx"

    @staticmethod
    def rf_ieee802154_rx(node_id: int | str) -> str:
        return f"sim/rf/ieee802154/{node_id}/rx"

    @staticmethod
    def rf_hci_tx(node_id: int | str) -> str:
        return f"sim/rf/hci/{node_id}/tx"

    @staticmethod
    def rf_hci_rx(node_id: int | str) -> str:
        return f"sim/rf/hci/{node_id}/rx"

    @staticmethod
    def telemetry_trace(node_id: int | str) -> str:
        return f"sim/telemetry/trace/{node_id}"

    @staticmethod
    def chardev_rx(node_id: int | str) -> str:
        return f"sim/chardev/{node_id}/rx"

    @staticmethod
    def chardev_tx(node_id: int | str) -> str:
        return f"sim/chardev/{node_id}/tx"

    @staticmethod
    def netdev_rx(node_id: int | str) -> str:
        return f"sim/netdev/{node_id}/rx"

    @staticmethod
    def netdev_tx(node_id: int | str) -> str:
        return f"sim/netdev/{node_id}/tx"
