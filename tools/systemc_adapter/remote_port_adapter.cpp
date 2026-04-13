#include <systemc.h>
#include <tlm.h>
#include <tlm_utils/simple_target_socket.h>
#include <tlm_utils/simple_initiator_socket.h>
#include <iostream>

using namespace sc_core;
using namespace tlm;
using namespace tlm_utils;

#include "libremote-port/remote-port-tlm.h"
#include "libremote-port/remote-port-tlm-memory-master.h"
#include "libremote-port/remote-port-tlm-wires.h"

class RegisterFile : public sc_module {
public:
    tlm_utils::simple_target_socket<RegisterFile> socket;
    uint32_t mem[1024];

    SC_HAS_PROCESS(RegisterFile);
    RegisterFile(sc_module_name name) : sc_module(name), socket("socket") {
        for (int i = 0; i < 1024; i++) mem[i] = 0;
        socket.register_b_transport(this, &RegisterFile::b_transport);
    }

    void b_transport(tlm::tlm_generic_payload& trans, sc_time& delay) {
        tlm::tlm_command cmd = trans.get_command();
        sc_dt::uint64 addr = trans.get_address();
        unsigned char* ptr = trans.get_data_ptr();
        unsigned int len = trans.get_data_length();

        if (addr + len > 4096) {
            trans.set_response_status(tlm::TLM_ADDRESS_ERROR_RESPONSE);
            return;
        }

        if (cmd == tlm::TLM_READ_COMMAND) {
            memcpy(ptr, ((uint8_t*)mem) + addr, len);
            std::cout << "[rp_adapter] READ from addr=0x" << std::hex << addr << " len=" << std::dec << len << std::endl;
        } else if (cmd == tlm::TLM_WRITE_COMMAND) {
            memcpy(((uint8_t*)mem) + addr, ptr, len);
            std::cout << "[rp_adapter] WRITE to addr=0x" << std::hex << addr << " val=0x";
            for(unsigned int i = 0; i < len; i++) {
                std::cout << std::hex << (int)ptr[i];
            }
            std::cout << std::dec << " len=" << len << std::endl;
        }
        
        trans.set_response_status(tlm::TLM_OK_RESPONSE);
    }
};

class ResetDriver : public sc_module {
public:
    sc_out<bool> rst;
    SC_HAS_PROCESS(ResetDriver);
    ResetDriver(sc_module_name name) : sc_module(name), rst("rst") {
        SC_THREAD(drive_reset);
    }
    void drive_reset() {
        rst.write(true);
        wait(10, SC_NS);
        rst.write(false);
        std::cout << "[ResetDriver] Reset de-asserted at 10 ns\n";
    }
};

int sc_main(int argc, char* argv[]) {
    if (argc < 2) {
        std::cerr << "Usage: " << argv[0] << " <socket-path>\n";
        return 1;
    }
    
    remoteport_tlm rp("rp_server", -1, argv[1]);
    remoteport_tlm_memory_master rp_mem("rp_mem");
    RegisterFile regfile("regfile");
    sc_signal<bool> rst_sig("rst_sig");
    ResetDriver rst_drv("rst_drv");
    
    rst_drv.rst(rst_sig);
    rp.rst(rst_sig);
    rp.register_dev(0, &rp_mem);
    rp_mem.sk.bind(regfile.socket);
    
    std::cout << "Starting Remote Port TLM-2.0 Adapter on " << argv[1] << std::endl;
    
    sc_start();
    
    return 0;
}
