#!/usr/bin/env python3
"""
virtmcu_extcap.py - Wireshark Extcap Plugin for VirtMCU Zenoh Capture.

This script implements the Wireshark extcap interface to allow live capturing
of VirtMCU simulation traffic directly from Zenoh topics.

It can also be used as a standalone PCAP dumper:
  python3 virtmcu_extcap.py --capture --fifo simulation.pcap
"""

import argparse
import asyncio
import os
import sys
from typing import Any

import zenoh

from tools import vproto

# Protocol mapping for DLT_USER0 (aligned with MessageLog.rs and dissector.lua)
PROTO_MAP = {
    0: 1,  # Ethernet
    1: 2,  # UART
    2: 7,  # SPI
    3: 4,  # CAN-FD
    4: 5,  # FlexRay
    5: 6,  # LIN
    6: 3,  # IEEE 802.15.4
    7: 8,  # RF-HCI
    8: 255,  # Control/Test Infra
}


def list_interfaces() -> None:
    print(
        "interface {value=virtmcu0}{display=VirtMCU Zenoh Capture}"
    )  # PRINT_EXCEPTION: Wireshark extcap protocol requirement


def list_dlts() -> None:
    print(
        "dlt {number=147}{name=DLT_USER0}{display=VirtMCU Custom Link Type}"
    )  # PRINT_EXCEPTION: Wireshark extcap protocol requirement


def list_config() -> None:
    default_session = os.environ.get("ZENOH_ROUTER", "")
    print(
        f"arg {{number=0}}{{call=--session}}{{display=Zenoh Session}}{{type=string}}{{default={default_session}}}{{tooltip=Zenoh router endpoint}}"
    )  # PRINT_EXCEPTION: Wireshark extcap protocol requirement
    print(
        "arg {number=1}{call=--topic}{display=Zenoh Topic}{type=string}{default=sim/coord/**/rx}{tooltip=Zenoh topic to subscribe to}"
    )  # PRINT_EXCEPTION: Wireshark extcap protocol requirement
    print(
        "arg {number=2}{call=--legacy}{display=Use Legacy Topics}{type=boolflag}{default=false}{tooltip=Subscribe to sim/comm/** for raw traffic}"
    )  # PRINT_EXCEPTION: Wireshark extcap protocol requirement


class PcapDumper:
    def __init__(self, fifo_path: str) -> None:
        self.fifo_path = fifo_path
        self.fifo: Any = None

    def open(self) -> None:
        if self.fifo_path == "-":
            self.fifo = sys.stdout.buffer
        else:
            self.fifo = open(self.fifo_path, "wb")

        # PCAP Global Header
        self.fifo.write((0xA1B2C3D4).to_bytes(4, "little"))
        self.fifo.write((2).to_bytes(2, "little"))
        self.fifo.write((4).to_bytes(2, "little"))
        self.fifo.write((0).to_bytes(4, "little", signed=True))
        self.fifo.write((0).to_bytes(4, "little"))
        self.fifo.write((65535).to_bytes(4, "little"))
        self.fifo.write((147).to_bytes(4, "little"))
        self.fifo.flush()

    def write_packet(self, vtime_ns: int, src: int, dst: int, protocol: int, payload: bytes) -> None:
        ts_sec = vtime_ns // 1_000_000_000
        ts_usec = (vtime_ns % 1_000_000_000) // 1000

        pcap_proto = PROTO_MAP.get(protocol, 255)

        # DLT_USER0 Header: src(4) + dst(4) + proto(2)
        header = src.to_bytes(4, "little") + dst.to_bytes(4, "little") + pcap_proto.to_bytes(2, "little")
        full_payload = header + payload

        incl_len = len(full_payload)
        orig_len = incl_len

        # Packet Header
        self.fifo.write(ts_sec.to_bytes(4, "little"))
        self.fifo.write(ts_usec.to_bytes(4, "little"))
        self.fifo.write(incl_len.to_bytes(4, "little"))
        self.fifo.write(orig_len.to_bytes(4, "little"))

        self.fifo.write(full_payload)
        self.fifo.flush()

    def close(self) -> None:
        if self.fifo and self.fifo_path != "-":
            self.fifo.close()


async def capture_loop(fifo_path: str, session_url: str, topic_pattern: str, use_legacy: bool) -> None:
    dumper = PcapDumper(fifo_path)
    dumper.open()

    conf = zenoh.Config()
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("mode", '"client"')
    if session_url:
        conf.insert_json5("connect/endpoints", f'["{session_url}"]')

    session = zenoh.open(conf)  # ZENOH_OPEN_EXCEPTION: manual config for Wireshark integration

    def on_sample(sample: zenoh.Sample) -> None:
        try:
            topic = sample.key_expr
            data = sample.payload.to_bytes()

            # 1. Try decoding as CoordMessage (Unified)
            try:
                # We use a heuristic: if topic matches sim/coord/**/rx
                if "sim/coord/" in str(topic):
                    msg = vproto.CoordMessage.unpack(data)
                    dumper.write_packet(
                        msg.delivery_vtime_ns, msg.src_node_id, msg.dst_node_id, msg.protocol, msg.payload
                    )
                    return
            except Exception:  # noqa: BLE001, S110, S110
                pass

            # 2. Try decoding as Legacy ZenohFrameHeader
            try:
                if len(data) >= vproto.SIZE_ZENOH_FRAME_HEADER:
                    header = vproto.ZenohFrameHeader.unpack(data[: vproto.SIZE_ZENOH_FRAME_HEADER])
                    payload = data[vproto.SIZE_ZENOH_FRAME_HEADER : vproto.SIZE_ZENOH_FRAME_HEADER + header.size]

                    # Extract node ID from topic sim/comm/<proto>/<node>/rx
                    parts = str(topic).split("/")
                    node_id = 0
                    proto_id = 8  # Default to Control

                    for i, part in enumerate(parts):
                        if part in ["eth", "uart", "can", "lin", "spi"]:
                            if i + 1 < len(parts):
                                try:
                                    node_id = int(parts[i + 1])
                                except ValueError:
                                    pass

                            if part == "eth":
                                proto_id = 0
                            elif part == "uart":
                                proto_id = 1
                            elif part == "can":
                                proto_id = 3
                            elif part == "lin":
                                proto_id = 5
                            elif part == "spi":
                                proto_id = 2
                            break
                        elif "rf" in part:
                            proto_id = 6  # Rf802154
                            break

                    dumper.write_packet(header.delivery_vtime_ns, 0, node_id, proto_id, payload)
            except Exception:  # noqa: BLE001, S110, S110
                pass

        except Exception:  # noqa: BLE001, S110
            pass

    sub = session.declare_subscriber(topic_pattern, on_sample)

    try:
        while True:
            await asyncio.sleep(1)  # SLEEP_EXCEPTION: Live capture event loop yield
    except asyncio.CancelledError:
        pass
    finally:
        sub.undeclare()  # type: ignore[no-untyped-call]
        session.close()  # type: ignore[no-untyped-call]
        dumper.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="VirtMCU Wireshark Extcap Plugin")
    parser.add_argument("--extcap-interfaces", action="store_true")
    parser.add_argument("--extcap-dlts", action="store_true")
    parser.add_argument("--extcap-config", action="store_true")
    parser.add_argument("--extcap-interface")
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--fifo")

    # Wireshark config args
    parser.add_argument("--session", default=os.environ.get("ZENOH_ROUTER"))
    parser.add_argument("--topic", default="sim/coord/**/rx")
    parser.add_argument("--legacy", action="store_true")

    args = parser.parse_args()

    if args.extcap_interfaces:
        list_interfaces()
    elif args.extcap_dlts:
        list_dlts()
    elif args.extcap_config:
        list_config()
    elif args.capture:
        if not args.fifo:
            print("Error: --fifo is required for capture", file=sys.stderr)  # PRINT_EXCEPTION: CLI error reporting
            sys.exit(1)

        if not args.session:
            print(
                "Error: Zenoh session endpoint must be specified via --session or ZENOH_ROUTER environment variable",
                file=sys.stderr,
            )  # PRINT_EXCEPTION: CLI error reporting
            sys.exit(1)

        topic = args.topic
        if args.legacy and topic == "sim/coord/**/rx":
            topic = "sim/comm/**"

        try:
            asyncio.run(capture_loop(args.fifo, args.session, topic, args.legacy))
        except KeyboardInterrupt:
            pass
    else:
        parser.print_help()
