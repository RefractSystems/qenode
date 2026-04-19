import socket
import sys


def check(sock_path):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(2.0)
    try:
        s.connect(sock_path)
        greeting = s.recv(4096)
        if not greeting:
            return False
        s.sendall(b'{"execute": "qmp_capabilities"}\n')
        s.recv(4096)
        s.sendall(b'{"execute": "query-status"}\n')
        s.recv(4096)
        return True
    except Exception as e:
        print(e)
        return False

if check(sys.argv[1]):
    sys.exit(0)
sys.exit(1)
