import sys
from pathlib import Path
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parent / ".."))
from tools.fake_adapter import recvall, start_server
from tools.vproto import MMIO_REQ_WRITE, VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION, MmioReq, VirtmcuHandshake


def test_recvall():
    mock_conn = MagicMock()
    # Test successful receive
    mock_conn.recv.side_effect = [b"hello", b" world"]
    assert recvall(mock_conn, 11) == b"hello world"

    # Test disconnect
    mock_conn.recv.side_effect = [b"123", b""]
    assert recvall(mock_conn, 5) is None


@patch("tools.fake_adapter.socket.socket")
def test_start_server_hs_fail(mock_socket_cls, capsys, tmp_path):
    mock_server = MagicMock()
    mock_socket_cls.return_value = mock_server
    mock_conn = MagicMock()
    mock_server.accept.return_value = (mock_conn, None)

    # Simulate connection closed before full handshake
    mock_conn.recv.return_value = b""

    sock_path = str(tmp_path / "fake_mmio.sock")
    start_server(sock_path)

    out, _ = capsys.readouterr()
    assert "Failed to receive handshake" in out


@patch("tools.fake_adapter.socket.socket")
def test_start_server_hs_mismatch(mock_socket_cls, capsys, tmp_path):
    mock_server = MagicMock()
    mock_socket_cls.return_value = mock_server
    mock_conn = MagicMock()
    mock_server.accept.return_value = (mock_conn, None)

    bad_hs = VirtmcuHandshake(magic=0, version=0).pack()
    mock_conn.recv.side_effect = [bad_hs]

    sock_path = str(tmp_path / "fake_mmio.sock")
    start_server(sock_path)

    out, _ = capsys.readouterr()
    assert "Handshake mismatch" in out


@patch("tools.fake_adapter.socket.socket")
@patch("pathlib.Path.unlink")
@patch("pathlib.Path.exists")
def test_start_server_success(mock_exists, mock_unlink, mock_socket_cls, capsys, tmp_path):
    mock_exists.return_value = True

    mock_server = MagicMock()
    mock_socket_cls.return_value = mock_server
    mock_conn = MagicMock()
    mock_server.accept.return_value = (mock_conn, None)

    valid_hs = VirtmcuHandshake(magic=VIRTMCU_PROTO_MAGIC, version=VIRTMCU_PROTO_VERSION).pack()
    valid_req = MmioReq(
        type=MMIO_REQ_WRITE, size=4, reserved1=0, reserved2=0, vtime_ns=1000, addr=0x1000, data=0xABCD
    ).pack()

    # Sequence:
    # 1. recvall gets valid_hs (8 bytes)
    # 2. recvall gets valid_req (32 bytes)
    # 3. recvall gets b"" indicating connection closed
    mock_conn.recv.side_effect = [valid_hs, valid_req, b""]

    sock_path = str(tmp_path / "fake_mmio.sock")
    start_server(sock_path)

    mock_exists.assert_called_with()
    mock_unlink.assert_called_with()
    mock_server.bind.assert_called_with(sock_path)
    mock_server.listen.assert_called_with(1)

    # Check that it sent a handshake and then a response to the MMIO req
    assert mock_conn.sendall.call_count == 2

    out, _ = capsys.readouterr()
    assert "REQ: type=1, size=4, vtime=1000, addr=0x1000, data=0xabcd" in out
