import socket, json
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect('qmp.sock')
s.recv(1024)
s.send(b'{"execute": "qmp_capabilities"}\n')
s.recv(1024)

def get_prop(path, prop):
    s.send(json.dumps({"execute": "qom-get", "arguments": {"path": path, "property": prop}}).encode() + b'\n')
    data = b""
    while b'\n' not in data:
        data += s.recv(4096)
    return json.loads(data.decode().strip())

print("PL011 container:", get_prop('/pl011@9000000/pl011[0]', 'container'))
