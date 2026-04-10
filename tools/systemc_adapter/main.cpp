#include <systemc>
#include <tlm>
#include <tlm_utils/simple_target_socket.h>
#include <tlm_utils/simple_initiator_socket.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <iostream>

using namespace sc_core;
using namespace sc_dt;
using namespace std;

// 1. Simple Register File SystemC Module
SC_MODULE(RegisterFile) {
    tlm_utils::simple_target_socket<RegisterFile> socket;
    uint32_t regs[256];

    SC_CTOR(RegisterFile) : socket("socket") {
        socket.register_b_transport(this, &RegisterFile::b_transport);
        for (int i = 0; i < 256; i++) regs[i] = 0;
    }

    void b_transport(tlm::tlm_generic_payload& trans, sc_time& delay) {
        tlm::tlm_command cmd = trans.get_command();
        uint64_t         adr = trans.get_address() / 4;
        unsigned char*   ptr = trans.get_data_ptr();
        unsigned int     len = trans.get_data_length();

        if (adr >= 256) {
            trans.set_response_status(tlm::TLM_ADDRESS_ERROR_RESPONSE);
            return;
        }

        if (cmd == tlm::TLM_READ_COMMAND) {
            memcpy(ptr, &regs[adr], len);
            cout << "[SystemC] Read " << hex << *(uint32_t*)ptr << " from reg " << dec << adr << endl;
        } else if (cmd == tlm::TLM_WRITE_COMMAND) {
            memcpy(&regs[adr], ptr, len);
            cout << "[SystemC] Wrote " << hex << *(uint32_t*)ptr << " to reg " << dec << adr << endl;
        }

        trans.set_response_status(tlm::TLM_OK_RESPONSE);
    }
};

// 2. QEMU to TLM Adapter
struct mmio_req {
    uint8_t type;
    uint8_t size;
    uint16_t reserved1;
    uint32_t reserved2;
    uint64_t addr;
    uint64_t data;
} __attribute__((packed));

struct mmio_resp {
    uint64_t data;
} __attribute__((packed));

SC_MODULE(QemuAdapter) {
    tlm_utils::simple_initiator_socket<QemuAdapter> socket;
    std::string socket_path;

    SC_HAS_PROCESS(QemuAdapter);

    QemuAdapter(sc_module_name name, std::string path) : sc_module(name), socket("socket"), socket_path(path) {
        SC_THREAD(run);
    }

    void run() {
        int server_fd = ::socket(AF_UNIX, SOCK_STREAM, 0);
        if (server_fd < 0) {
            perror("socket");
            return;
        }

        struct sockaddr_un addr;
        memset(&addr, 0, sizeof(addr));
        addr.sun_family = AF_UNIX;
        strncpy(addr.sun_path, socket_path.c_str(), sizeof(addr.sun_path) - 1);

        unlink(socket_path.c_str());
        if (bind(server_fd, (struct sockaddr*)&addr, sizeof(addr)) < 0) {
            perror("bind");
            return;
        }

        if (listen(server_fd, 1) < 0) {
            perror("listen");
            return;
        }

        cout << "[SystemC] Listening on " << socket_path << "..." << endl;

        int client_fd = accept(server_fd, NULL, NULL);
        if (client_fd < 0) {
            perror("accept");
            return;
        }

        cout << "[SystemC] QEMU connected." << endl;

        while (true) {
            mmio_req req;
            ssize_t n = read(client_fd, &req, sizeof(req));
            if (n <= 0) break; // QEMU disconnected

            tlm::tlm_generic_payload trans;
            sc_time delay = sc_time(10, SC_NS);
            
            trans.set_address(req.addr);
            trans.set_data_length(req.size);
            trans.set_streaming_width(req.size);
            trans.set_byte_enable_ptr(0);
            trans.set_dmi_allowed(false);
            trans.set_response_status(tlm::TLM_INCOMPLETE_RESPONSE);

            uint64_t data_buf = req.data;
            trans.set_data_ptr((unsigned char*)&data_buf);

            if (req.type == 0) {
                trans.set_command(tlm::TLM_READ_COMMAND);
            } else {
                trans.set_command(tlm::TLM_WRITE_COMMAND);
            }

            // Do the blocking TLM call
            socket->b_transport(trans, delay);

            // Wait for the simulated delay
            wait(delay);

            mmio_resp resp;
            if (req.type == 0 && trans.is_response_ok()) {
                resp.data = data_buf;
            } else {
                resp.data = 0; // Or some status
            }

            if (write(client_fd, &resp, sizeof(resp)) <= 0) {
                break;
            }
        }

        close(client_fd);
        close(server_fd);
        unlink(socket_path.c_str());
        sc_stop();
    }
};

int sc_main(int argc, char* argv[]) {
    if (argc < 2) {
        cerr << "Usage: " << argv[0] << " <socket_path>" << endl;
        return 1;
    }

    RegisterFile regfile("regfile");
    QemuAdapter adapter("adapter", argv[1]);

    adapter.socket.bind(regfile.socket);

    sc_start();
    return 0;
}
