import logging
from pathlib import Path

logger = logging.getLogger(__name__)


def write_pcap(path: Path, history: list[dict]) -> None:
    """
    Writes a PCAP file from the recorded history.
    Uses DLT_USER0 (147) as the link layer type.
    """
    try:
        with path.open("wb") as f:
            # Global Header (24 bytes)
            # magic_number: u32 = 0xa1b2c3d4
            # version_major: u16 = 2
            # version_minor: u16 = 4
            # thiszone: i32 = 0
            # sigfigs: u32 = 0
            # snaplen: u32 = 65535
            # network: u32 = 147 (DLT_USER0)
            f.write((0xA1B2C3D4).to_bytes(4, "little"))
            f.write((2).to_bytes(2, "little"))
            f.write((4).to_bytes(2, "little"))
            f.write((0).to_bytes(4, "little", signed=True))
            f.write((0).to_bytes(4, "little"))
            f.write((65535).to_bytes(4, "little"))
            f.write((147).to_bytes(4, "little"))

            for entry in history:
                try:
                    # Robust type parsing for corner cases
                    vtime_ns = int(entry.get("vtime_ns") or 0)
                    topic = str(entry.get("topic", "")).encode("utf-8")
                    payload_hex = str(entry.get("payload", ""))
                    try:
                        payload_bytes = bytes.fromhex(payload_hex)
                    except ValueError:
                        payload_bytes = b""
                    direction = str(entry.get("direction", "unk")).encode("utf-8")

                    src_node = 0
                    dst_node = 0
                    proto = 255

                    topic_len = len(topic)
                    dir_len = len(direction)

                    pcap_payload = (
                        src_node.to_bytes(4, "little")
                        + dst_node.to_bytes(4, "little")
                        + proto.to_bytes(2, "little")
                        + topic_len.to_bytes(2, "little")
                        + topic
                        + dir_len.to_bytes(2, "little")
                        + direction
                        + payload_bytes
                    )

                    ts_sec = vtime_ns // 1_000_000_000
                    ts_usec = (vtime_ns % 1_000_000_000) // 1000

                    orig_len = len(pcap_payload)
                    incl_len = min(orig_len, 65535)

                    # Packet Header (16 bytes)
                    f.write(ts_sec.to_bytes(4, "little"))
                    f.write(ts_usec.to_bytes(4, "little"))
                    f.write(incl_len.to_bytes(4, "little"))
                    f.write(orig_len.to_bytes(4, "little"))

                    f.write(pcap_payload[:incl_len])
                except Exception as pkt_e:
                    logger.warning(f"Failed to write PCAP packet: {pkt_e}")
                    continue
    except Exception as e:
        logger.error(f"Failed to open/write PCAP file {path}: {e}")
