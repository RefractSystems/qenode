#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <iostream>
#include <vector>
#include <cstring>
#include "../../hw/misc/virtmcu_proto.h"

using namespace std;

int main(int argc, char* argv[]) {
    if (argc < 2) {
        cerr << "Usage: " << argv[0] << " <socket_path>" << endl;
        return 1;
    }

    string socket_path = argv[1];
    int server_fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (server_fd < 0) { perror("socket"); return 1; }

    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, socket_path.c_str(), sizeof(addr.sun_path) - 1);

    unlink(socket_path.c_str());
    if (bind(server_fd, (struct sockaddr*)&addr, sizeof(addr)) < 0) { perror("bind"); return 1; }
    if (listen(server_fd, 1) < 0) { perror("listen"); return 1; }

    cout << "Stress adapter listening on " << socket_path << "..." << endl;

    int client_fd = accept(server_fd, NULL, NULL);
    if (client_fd < 0) { perror("accept"); return 1; }

    virtmcu_handshake hs_in;
    if (read(client_fd, &hs_in, sizeof(hs_in)) != sizeof(hs_in)) { cerr << "Handshake read failed" << endl; return 1; }
    virtmcu_handshake hs_out = { VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION };
    write(client_fd, &hs_out, sizeof(hs_out));

    cout << "Client connected, starting stress loop..." << endl;

    while (true) {
        mmio_req req;
        ssize_t n = read(client_fd, &req, sizeof(req));
        if (n <= 0) break;
        if (n != sizeof(req)) {
            // Partial read, handle it
            char* p = (char*)&req + n;
            size_t remaining = sizeof(req) - n;
            while (remaining > 0) {
                n = read(client_fd, p, remaining);
                if (n <= 0) goto end;
                p += n; remaining -= n;
            }
        }

        sysc_msg resp;
        resp.type = SYSC_MSG_RESP;
        resp.irq_num = 0;
        resp.data = req.data; // Echo back for simplicity

        if (write(client_fd, &resp, sizeof(resp)) != sizeof(resp)) break;
    }

end:
    cout << "Stress adapter exiting." << endl;
    close(client_fd);
    close(server_fd);
    unlink(socket_path.c_str());
    return 0;
}
