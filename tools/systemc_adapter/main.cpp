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

                // Clock sync: advance SystemC time to match QEMU's vtime_ns
                sc_time target_time = sc_time(req.vtime_ns, SC_NS);
                if (target_time > sc_time_stamp()) {
                    wait(target_time - sc_time_stamp());
                } else if (target_time < sc_time_stamp()) {
                    cerr << "[QemuAdapter] WARNING: Time regression! target=" << req.vtime_ns << " ns, current=" << sc_time_stamp().to_double() << " ns" << endl;
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

        virtmcu_handshake hs_in;
        if (!readn(client_fd, &hs_in, sizeof(hs_in))) {
            cerr << "[SystemC] Failed to read handshake from QEMU." << endl;
            close(client_fd); client_fd = -1; return;
        }

        if (hs_in.magic != VIRTMCU_PROTO_MAGIC || hs_in.version != VIRTMCU_PROTO_VERSION) {
            cerr << "[SystemC] Handshake mismatch: expected magic 0x" << hex << VIRTMCU_PROTO_MAGIC
                 << " version " << dec << VIRTMCU_PROTO_VERSION << ", got magic 0x" << hex << hs_in.magic
                 << " version " << dec << hs_in.version << endl;
            close(client_fd); client_fd = -1; return;
        }

        virtmcu_handshake hs_out = { VIRTMCU_PROTO_MAGIC, VIRTMCU_PROTO_VERSION };
        writen_sync(client_fd, &hs_out, sizeof(hs_out));

        cout << "[SystemC] QEMU connected and handshake successful." << endl;

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
    uint64_t         adr = trans.get_address();
    unsigned char*   ptr = trans.get_data_ptr();
    unsigned int     len = trans.get_data_length();

    if (len > 4) {
        trans.set_response_status(tlm::TLM_BURST_ERROR_RESPONSE);
        return;
    }

    uint64_t reg_idx = adr / 4;
    if (reg_idx >= 256) {
        trans.set_response_status(tlm::TLM_ADDRESS_ERROR_RESPONSE);
        return;
    }

    if (cmd == tlm::TLM_READ_COMMAND) {
        memcpy(ptr, &regs[reg_idx], len);
        uint32_t val = 0;
        memcpy(&val, ptr, len);
        cout << "[SystemC] Read " << hex << val << " from reg " << dec << reg_idx << " (addr " << adr << ")" << endl;
    } else if (cmd == tlm::TLM_WRITE_COMMAND) {
        uint32_t val = 0;
        memcpy(&val, ptr, len);
        regs[reg_idx] = val;
        cout << "[SystemC] Wrote " << hex << val << " to reg " << dec << reg_idx << " (addr " << adr << ")" << endl;
        
        // Trigger IRQ 0 if writing to reg 255
        if (reg_idx == 255 && adapter) {
            adapter->trigger_irq(0, val != 0);
        }
    }
    trans.set_response_status(tlm::TLM_OK_RESPONSE);
}


// --- Educational CAN-lite Model ---

/**
 * CAN Wire Format (20 bytes):
 * Bytes 0-7:   delivery_vtime_ns (uint64_t)
 * Bytes 8-11:  size (uint32_t, always 8 for CAN-lite)
 * Bytes 12-15: can_id (uint32_t)
 * Bytes 16-19: can_data (uint32_t)
 */
struct CanWireFrame {
    uint64_t delivery_vtime_ns;
    uint32_t size;
    uint32_t can_id;
    uint32_t can_data;
} __attribute__((packed));

struct CanFrame {
    uint32_t id;
    uint32_t data;
};

struct CanInternalFrame {
    uint64_t delivery_vtime_ns;
    CanFrame frame;
};

class CanController;

class SharedMedium : public sc_module {
public:
    CanController* controller;
    std::string node_id;
    z_owned_session_t session;
    z_owned_publisher_t pub;
    z_owned_subscriber_t sub;

    std::queue<CanInternalFrame> rx_queue;
    std::mutex rx_mtx;
    AsyncEvent rx_async_event;

    std::queue<CanWireFrame> tx_queue;
    std::mutex tx_mtx;
    std::condition_variable tx_cv;
    std::thread tx_thread;
    bool running;

    SC_HAS_PROCESS(SharedMedium);
    SharedMedium(sc_module_name name, std::string node) : 
        sc_module(name), controller(nullptr), node_id(node), running(true) 
    {
        tx_thread = std::thread(&SharedMedium::zenoh_tx_thread, this);
        SC_THREAD(process_rx);
    }

    void start_of_simulation() override {
        z_owned_config_t config;
        z_config_default(&config);
        if (z_open(&session, z_move(config), NULL) != 0) {
            cerr << "[SharedMedium] Failed to open Zenoh session" << endl;
            return;
        }
        
        char topic_tx[128];
        snprintf(topic_tx, sizeof(topic_tx), "sim/systemc/frame/%s/tx", node_id.c_str());
        z_owned_keyexpr_t kexpr_tx;
        z_keyexpr_from_str(&kexpr_tx, topic_tx);
        if (z_declare_publisher(z_session_loan(&session), &pub, z_keyexpr_loan(&kexpr_tx), NULL) != 0) {
            cerr << "[SharedMedium] Failed to declare publisher" << endl;
        }
        z_keyexpr_drop(z_move(kexpr_tx));

        char topic_rx[128];
        snprintf(topic_rx, sizeof(topic_rx), "sim/systemc/frame/%s/rx", node_id.c_str());
        z_owned_closure_sample_t callback;
        z_closure_sample(&callback, on_zenoh_rx, NULL, this);
        z_owned_keyexpr_t kexpr_rx;
        z_keyexpr_from_str(&kexpr_rx, topic_rx);
        if (z_declare_subscriber(z_session_loan(&session), &sub, z_keyexpr_loan(&kexpr_rx), z_move(callback), NULL) != 0) {
            cerr << "[SharedMedium] Failed to declare subscriber" << endl;
        }
        z_keyexpr_drop(z_move(kexpr_rx));
    }

    ~SharedMedium() {
        running = false;
        tx_cv.notify_all();
        if (tx_thread.joinable()) tx_thread.join();

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
        CanWireFrame wire;
        if (z_bytes_reader_read(&reader, (uint8_t*)&wire, sizeof(wire)) == sizeof(wire)) {
            CanInternalFrame internal = {
                .delivery_vtime_ns = wire.delivery_vtime_ns,
                .frame = {wire.can_id, wire.can_data}
            };
            {
                std::lock_guard<std::mutex> lock(self->rx_mtx);
                self->rx_queue.push(internal);
            }
            cout << "[SharedMedium] Zenoh RX: ID=" << hex << wire.can_id << " vtime=" << dec << wire.delivery_vtime_ns << " ns" << endl;
            self->rx_async_event.notify_from_os_thread();
        }
    }

    void process_rx() {
        while (running) {
            if (rx_queue.empty()) {
                wait(rx_async_event.default_event());
            }
            if (!running) break;

            while (true) {
                CanInternalFrame internal;
                {
                    std::lock_guard<std::mutex> lock(rx_mtx);
                    if (rx_queue.empty()) break;
                    internal = rx_queue.front();
                    rx_queue.pop();
                }

                // Deterministic wait: wait until delivery time
                sc_time delivery_time = sc_time(internal.delivery_vtime_ns, SC_NS);
                if (delivery_time > sc_time_stamp()) {
                    cout << "[SharedMedium] Waiting for delivery at " << internal.delivery_vtime_ns << " ns (current: " << sc_time_stamp().to_double() << " ns)" << endl;
                    wait(delivery_time - sc_time_stamp());
                } else if (delivery_time < sc_time_stamp()) {
                    cerr << "[SharedMedium] LATE FRAME! delivery=" << internal.delivery_vtime_ns << " ns, current=" << sc_time_stamp().to_double() << " ns" << endl;
                }

                self_deliver(internal.frame);
            }
        }
    }

    void self_deliver(CanFrame frame);

    void transmit(CanFrame frame) {
        CanWireFrame wire = {
            .delivery_vtime_ns = (uint64_t)sc_time_stamp().to_double(),
            .size = 8,
            .can_id = frame.id,
            .can_data = frame.data
        };

        {
            std::lock_guard<std::mutex> lock(tx_mtx);
            tx_queue.push(wire);
        }
        tx_cv.notify_one();
    }

    void zenoh_tx_thread() {
        while (running) {
            CanWireFrame wire;
            {
                std::unique_lock<std::mutex> lock(tx_mtx);
                tx_cv.wait(lock, [this] { return !tx_queue.empty() || !running; });
                if (!running) break;
                wire = tx_queue.front();
                tx_queue.pop();
            }

            z_owned_bytes_t payload;
            z_bytes_copy_from_buf(&payload, (uint8_t*)&wire, sizeof(wire));
            z_publisher_put(z_publisher_loan(&pub), z_move(payload), NULL);
        }
    }
};

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
    void receive_frame(CanFrame frame) {
        rx_id = frame.id;
        rx_data = frame.data;
        status |= 1; // rx_pending
        rx_event.notify(SC_ZERO_TIME);
    }
    void on_rx() {
        if (adapter) {
            adapter->trigger_irq(0, true);
        }
    }
};

void SharedMedium::self_deliver(CanFrame frame) {
    if (controller) {
        controller->receive_frame(frame);
    }
}

void CanController::b_transport(tlm::tlm_generic_payload& trans, sc_time& delay) {
    tlm::tlm_command cmd = trans.get_command();
    uint64_t         adr = trans.get_address();
    unsigned char*   ptr = trans.get_data_ptr();
    unsigned int     len = trans.get_data_length();

    if (len > 4) {
        trans.set_response_status(tlm::TLM_BURST_ERROR_RESPONSE);
        return;
    }

    if (cmd == tlm::TLM_READ_COMMAND) {
        uint32_t val = 0;
        bool addr_ok = true;
        if (adr == 0x00) val = tx_id;
        else if (adr == 0x04) val = tx_data;
        else if (adr == 0x0C) val = status;
        else if (adr == 0x10) val = rx_id;
        else if (adr == 0x14) val = rx_data;
        else addr_ok = false;

        if (addr_ok) {
            memcpy(ptr, &val, len);
            trans.set_response_status(tlm::TLM_OK_RESPONSE);
        } else {
            trans.set_response_status(tlm::TLM_ADDRESS_ERROR_RESPONSE);
        }
    } else if (cmd == tlm::TLM_WRITE_COMMAND) {
        uint32_t val = 0;
        memcpy(&val, ptr, len);

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
        } else {
            trans.set_response_status(tlm::TLM_ADDRESS_ERROR_RESPONSE);
            return;
        }
        trans.set_response_status(tlm::TLM_OK_RESPONSE);
    }
}

int sc_main(int argc, char* argv[]) {
    sc_set_time_resolution(1, SC_NS);
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
        sc_start();
        if (adapter.running) {
            std::this_thread::sleep_for(std::chrono::milliseconds(1));
        }
    }
    
    if (regfile) delete regfile;
    if (bus)     delete bus;
    if (can)     delete can;
    return 0;
}
