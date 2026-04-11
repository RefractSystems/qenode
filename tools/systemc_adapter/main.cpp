#include <systemc>
#include <tlm>
#include <tlm_utils/simple_target_socket.h>
#include <tlm_utils/simple_initiator_socket.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include <iostream>
#include <thread>
#include <mutex>
#include <condition_variable>
#include <queue>
#include <zenoh.h>

/* Wire protocol shared with hw/misc/mmio-socket-bridge.c */
#include "../../hw/misc/virtmcu_proto.h"

using namespace sc_core;
using namespace sc_dt;
using namespace std;

// Forward declaration
class QemuAdapter;

// 1. Simple Register File SystemC Module
SC_MODULE(RegisterFile) {
    tlm_utils::simple_target_socket<RegisterFile> socket;
    uint32_t regs[256];
    QemuAdapter* adapter;

    SC_CTOR(RegisterFile) : socket("socket"), adapter(nullptr) {
        socket.register_b_transport(this, &RegisterFile::b_transport);
        for (int i = 0; i < 256; i++) regs[i] = 0;
    }

    void b_transport(tlm::tlm_generic_payload& trans, sc_time& delay);
};

/*
 * 2. Multi-threaded QEMU to TLM Adapter
 */

class AsyncEvent : public sc_prim_channel {
    sc_event e;
public:
    AsyncEvent() : sc_prim_channel(sc_gen_unique_name("safe_event")) {}
    void notify_from_os_thread() {
        async_request_update();
    }
    void update() override {
        e.notify(SC_ZERO_TIME);
    }
    const sc_event& default_event() const {
        return e;
    }
};

class StopEvent : public sc_prim_channel {
public:
    StopEvent() : sc_prim_channel(sc_gen_unique_name("stop_event")) {}
    void notify_from_os_thread() { async_request_update(); }
    void update() override { sc_stop(); }
};

SC_MODULE(QemuAdapter) {
    tlm_utils::simple_initiator_socket<QemuAdapter> socket;
    std::string socket_path;

    std::thread io_thread;
    int client_fd;
    bool running;

    std::mutex mtx;
    std::mutex socket_mtx;
    std::condition_variable cv;
    std::queue<mmio_req> req_queue;

    bool has_resp;
    sysc_msg resp_msg;

    AsyncEvent safe_event;
    StopEvent stop_event;

    SC_HAS_PROCESS(QemuAdapter);

    QemuAdapter(sc_module_name name, std::string path) : 
        sc_module(name), socket("socket"), socket_path(path), 
        client_fd(-1), running(true), has_resp(false) 
    {
        SC_THREAD(systemc_thread);
        SC_THREAD(keep_alive_thread);
    }

    void keep_alive_thread() {
        while (running) {
            wait(1, SC_SEC);
        }
    }

    void trigger_irq(uint32_t irq_num, bool level) {
        sysc_msg msg;
        msg.type = level ? SYSC_MSG_IRQ_SET : SYSC_MSG_IRQ_CLEAR;
        msg.irq_num = irq_num;
        msg.data = 0;
        send_msg(msg);
    }

    void send_msg(const sysc_msg& msg) {
        std::lock_guard<std::mutex> lock(socket_mtx);
        if (client_fd >= 0) {
            writen_sync(client_fd, &msg, sizeof(msg));
        }
    }

    bool writen_sync(int fd, const void* buf, size_t len) {
        const char* p = static_cast<const char*>(buf);
        while (len > 0) {
            ssize_t n = ::write(fd, p, len);
            if (n <= 0) {
                if (n < 0 && errno == EINTR) continue;
                return false;
            }
            p += n; len -= n;
        }
        return true;
    }

    void end_of_elaboration() override {
        io_thread = std::thread(&QemuAdapter::socket_thread, this);
    }

    ~QemuAdapter() {
        running = false;
        safe_event.notify_from_os_thread(); // Wake up systemc if waiting
        if (client_fd >= 0) {
            shutdown(client_fd, SHUT_RDWR);
            close(client_fd);
        }
        if (io_thread.joinable()) io_thread.join();
    }

    void systemc_thread() {
        while (running) {
            wait(safe_event.default_event());
            if (!running) break;
            
            while (true) {
                mmio_req req;
                {
                    std::lock_guard<std::mutex> lock(mtx);
                    if (req_queue.empty()) break;
                    req = req_queue.front();
                    req_queue.pop();
                }

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

                if (req.type == MMIO_REQ_READ) {
                    trans.set_command(tlm::TLM_READ_COMMAND);
                } else {
                    trans.set_command(tlm::TLM_WRITE_COMMAND);
                }

                socket->b_transport(trans, delay);
                wait(delay);

                sysc_msg resp = {0};
                resp.type = SYSC_MSG_RESP;
                if (req.type == MMIO_REQ_READ && trans.is_response_ok()) {
                    resp.data = data_buf;
                } else {
                    resp.data = 0;
                }

                {
                    std::lock_guard<std::mutex> lock(mtx);
                    resp_msg = resp;
                    has_resp = true;
                    cv.notify_one();
                }
            }
        }
    }

    void socket_thread() {
        int server_fd = ::socket(AF_UNIX, SOCK_STREAM, 0);
        if (server_fd < 0) return;

        struct sockaddr_un addr;
        memset(&addr, 0, sizeof(addr));
        addr.sun_family = AF_UNIX;
        strncpy(addr.sun_path, socket_path.c_str(), sizeof(addr.sun_path) - 1);

        unlink(socket_path.c_str());
        if (bind(server_fd, (struct sockaddr*)&addr, sizeof(addr)) < 0) return;
        if (listen(server_fd, 1) < 0) return;

        cout << "[SystemC] Listening on " << socket_path << "..." << endl;

        client_fd = accept(server_fd, NULL, NULL);
        if (client_fd < 0) return;

        cout << "[SystemC] QEMU connected." << endl;

        auto readn = [](int fd, void* buf, size_t len) -> bool {
            char* p = static_cast<char*>(buf);
            while (len > 0) {
                ssize_t n = ::read(fd, p, len);
                if (n <= 0) {
                    if (n < 0 && errno == EINTR) continue;
                    return false;
                }
                p += n; len -= n;
            }
            return true;
        };

        while (running) {
            mmio_req req;
            if (!readn(client_fd, &req, sizeof(req))) break;

            {
                std::lock_guard<std::mutex> lock(mtx);
                req_queue.push(req);
                has_resp = false;
            }
            
            safe_event.notify_from_os_thread();

            // Wait for response from SystemC thread
            sysc_msg resp;
            {
                std::unique_lock<std::mutex> lock(mtx);
                cv.wait(lock, [this]() { return has_resp || !running; });
                if (!running) break;
                resp = resp_msg;
                has_resp = false;
            }

            send_msg(resp);
        }

        cout << "[SystemC] OS thread exiting." << endl;
        running = false;
        safe_event.notify_from_os_thread();
        close(client_fd);
        client_fd = -1;
        close(server_fd);
        unlink(socket_path.c_str());
        
        stop_event.notify_from_os_thread();
    }
};

void RegisterFile::b_transport(tlm::tlm_generic_payload& trans, sc_time& delay) {
    tlm::tlm_command cmd = trans.get_command();
    uint64_t         adr = trans.get_address() / 4;
    unsigned char*   ptr = trans.get_data_ptr();
    unsigned int     len = trans.get_data_length();

    uint64_t words_needed = (len + 3) / 4;
    if (adr + words_needed > 256) {
        trans.set_response_status(tlm::TLM_ADDRESS_ERROR_RESPONSE);
        return;
    }

    if (cmd == tlm::TLM_READ_COMMAND) {
        memcpy(ptr, &regs[adr], len);
    } else if (cmd == tlm::TLM_WRITE_COMMAND) {
        uint32_t val;
        memcpy(&val, ptr, len);
        regs[adr] = val;
        cout << "[SystemC] Wrote " << hex << val << " to reg " << dec << adr << endl;
        
        // Trigger IRQ 0 if writing to reg 255
        if (adr == 255 && adapter) {
            adapter->trigger_irq(0, val != 0);
        }
    }
    trans.set_response_status(tlm::TLM_OK_RESPONSE);
}


// --- Educational CAN-lite Model ---
struct CanFrame {
    uint32_t id;
    uint32_t data;
};

class SharedMedium;

class CanController : public sc_module {
public:
    tlm_utils::simple_target_socket<CanController> socket;
    QemuAdapter* adapter;
    SharedMedium* bus;

    uint32_t tx_id, tx_data;
    uint32_t rx_id, rx_data;
    uint32_t status; // bit 0: rx_pending, bit 1: tx_ready

    sc_event rx_event;

    SC_HAS_PROCESS(CanController);
    CanController(sc_module_name name) : sc_module(name), socket("socket"), adapter(nullptr), bus(nullptr) {
        socket.register_b_transport(this, &CanController::b_transport);
        tx_id = 0; tx_data = 0; rx_id = 0; rx_data = 0;
        status = 2; // tx_ready
        SC_METHOD(on_rx);
        dont_initialize();
        sensitive << rx_event;
    }

    void b_transport(tlm::tlm_generic_payload& trans, sc_time& delay);
    void receive_frame(CanFrame frame);
    void on_rx();
};

class SharedMedium : public sc_module {
public:
    CanController* controller;
    std::string node_id;
    z_owned_session_t session;
    z_owned_publisher_t pub;
    z_owned_subscriber_t sub;

    std::queue<CanFrame> rx_queue;
    std::mutex rx_mtx;
    AsyncEvent rx_async_event;

    SC_HAS_PROCESS(SharedMedium);
    SharedMedium(sc_module_name name, std::string node) : sc_module(name), node_id(node), controller(nullptr) {
        z_owned_config_t config;
        z_config_default(&config);
        z_open(&session, z_move(config), NULL);
        
        char topic_tx[128];
        snprintf(topic_tx, sizeof(topic_tx), "sim/systemc/frame/%s/tx", node_id.c_str());
        z_owned_keyexpr_t kexpr_tx;
        z_keyexpr_from_str(&kexpr_tx, topic_tx);
        z_declare_publisher(z_session_loan(&session), &pub, z_keyexpr_loan(&kexpr_tx), NULL);
        z_keyexpr_drop(z_move(kexpr_tx));

        char topic_rx[128];
        snprintf(topic_rx, sizeof(topic_rx), "sim/systemc/frame/%s/rx", node_id.c_str());
        z_owned_closure_sample_t callback;
        z_closure_sample(&callback, on_zenoh_rx, NULL, this);
        z_owned_keyexpr_t kexpr_rx;
        z_keyexpr_from_str(&kexpr_rx, topic_rx);
        z_declare_subscriber(z_session_loan(&session), &sub, z_keyexpr_loan(&kexpr_rx), z_move(callback), NULL);
        z_keyexpr_drop(z_move(kexpr_rx));

        SC_THREAD(process_rx);
    }

    ~SharedMedium() {
        z_publisher_drop(z_move(pub));
        z_subscriber_drop(z_move(sub));
        z_close(z_session_loan_mut(&session), NULL);
        z_session_drop(z_move(session));
    }

    static void on_zenoh_rx(z_loaned_sample_t *sample, void *context) {
        SharedMedium* self = static_cast<SharedMedium*>(context);
        const z_loaned_bytes_t *payload = z_sample_payload(sample);
        if (!payload) return;

        z_bytes_reader_t reader = z_bytes_get_reader(payload);
        uint8_t buf[20];
        if (z_bytes_reader_read(&reader, buf, 20) == 20) {
            uint32_t can_id, can_data;
            memcpy(&can_id, buf + 12, 4);
            memcpy(&can_data, buf + 16, 4);

            CanFrame frame = {can_id, can_data};
            {
                std::lock_guard<std::mutex> lock(self->rx_mtx);
                self->rx_queue.push(frame);
            }
            self->rx_async_event.notify_from_os_thread();
        }
    }

    void process_rx() {
        while (true) {
            wait(rx_async_event.default_event());
            while (true) {
                CanFrame frame;
                {
                    std::lock_guard<std::mutex> lock(rx_mtx);
                    if (rx_queue.empty()) break;
                    frame = rx_queue.front();
                    rx_queue.pop();
                }
                // Simulate arbitration / delivery delay
                wait(sc_time(1, SC_MS));
                if (controller) {
                    controller->receive_frame(frame);
                }
            }
        }
    }

    void transmit(CanFrame frame) {
        uint8_t buf[20] = {0};
        uint32_t size = 8;
        memcpy(buf + 8, &size, 4);
        memcpy(buf + 12, &frame.id, 4);
        memcpy(buf + 16, &frame.data, 4);

        z_owned_bytes_t payload;
        z_bytes_copy_from_buf(&payload, buf, sizeof(buf));
        z_publisher_put(z_publisher_loan(&pub), z_move(payload), NULL);
    }
};

void CanController::receive_frame(CanFrame frame) {
    rx_id = frame.id;
    rx_data = frame.data;
    status |= 1; // rx_pending
    rx_event.notify(SC_ZERO_TIME);
}

void CanController::on_rx() {
    if (adapter) {
        adapter->trigger_irq(0, true);
    }
}

void CanController::b_transport(tlm::tlm_generic_payload& trans, sc_time& delay) {
    tlm::tlm_command cmd = trans.get_command();
    uint64_t         adr = trans.get_address();
    unsigned char*   ptr = trans.get_data_ptr();
    unsigned int     len = trans.get_data_length();

    if (cmd == tlm::TLM_READ_COMMAND) {
        uint32_t val = 0;
        if (adr == 0x00) val = tx_id;
        else if (adr == 0x04) val = tx_data;
        else if (adr == 0x0C) val = status;
        else if (adr == 0x10) val = rx_id;
        else if (adr == 0x14) val = rx_data;
        memcpy(ptr, &val, len);
        cout << "[SystemC CAN] Read " << hex << val << " from reg " << dec << adr << endl;
    } else if (cmd == tlm::TLM_WRITE_COMMAND) {
        uint32_t val;
        memcpy(&val, ptr, len);
        cout << "[SystemC CAN] Wrote " << hex << val << " to reg " << dec << adr << endl;

        if (adr == 0x00) tx_id = val;
        else if (adr == 0x04) tx_data = val;
        else if (adr == 0x08) {
            if (val == 1 && bus) {
                CanFrame frame = {tx_id, tx_data};
                bus->transmit(frame);
            }
        } else if (adr == 0x18) {
            status &= ~1;
            if (adapter) {
                adapter->trigger_irq(0, false);
            }
        }
    }
    trans.set_response_status(tlm::TLM_OK_RESPONSE);
}

int sc_main(int argc, char* argv[]) {
    if (argc < 2) {
        cerr << "Usage: " << argv[0] << " <socket_path> [node_id]" << endl;
        return 1;
    }

    std::string socket_path = argv[1];
    std::string node_id = (argc > 2) ? argv[2] : "";

    QemuAdapter adapter("adapter", socket_path);

    RegisterFile* regfile = nullptr;
    CanController* can = nullptr;
    SharedMedium* bus = nullptr;

    if (node_id.empty()) {
        regfile = new RegisterFile("regfile");
        adapter.socket.bind(regfile->socket);
        regfile->adapter = &adapter;
    } else {
        can = new CanController("can");
        bus = new SharedMedium("bus", node_id);
        can->bus = bus;
        bus->controller = can;
        adapter.socket.bind(can->socket);
        can->adapter = &adapter;
    }

    while (adapter.running) {
        sc_start(100, SC_MS);
    }
    
    delete regfile;
    delete can;
    delete bus;
    return 0;
}
