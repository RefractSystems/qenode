#include "virtmcu/resd_parser.hpp"
#include <iostream>
#include <thread>
#include <chrono>
#include <mutex>
#include <condition_variable>
#include <zenoh.h>
#include <cstring>

using namespace virtmcu;

struct ClockAdvancePayload {
    uint64_t delta_ns;
    uint64_t mujoco_time_ns;
} __attribute__((packed));

struct ClockReadyPayload {
    uint64_t current_vtime_ns;
    uint32_t n_frames;
} __attribute__((packed));

struct ReplyContext {
    std::mutex mtx;
    std::condition_variable cv;
    uint64_t current_vtime_ns;
    bool received;
    bool success;
};

void on_reply(z_loaned_reply_t *reply, void *context) {
    auto* ctx = static_cast<ReplyContext*>(context);
    std::lock_guard<std::mutex> lock(ctx->mtx);
    
    if (z_reply_is_ok(reply)) {
        const z_loaned_sample_t* sample = z_reply_ok(reply);
        const z_loaned_bytes_t *payload = z_sample_payload(sample);
        z_bytes_reader_t reader = z_bytes_get_reader(payload);
        ClockReadyPayload rep;
        if (z_bytes_reader_read(&reader, reinterpret_cast<uint8_t*>(&rep), sizeof(rep)) == sizeof(rep)) {
            ctx->current_vtime_ns = rep.current_vtime_ns;
            ctx->success = true;
        } else {
            ctx->success = false;
        }
    } else {
        ctx->success = false;
    }
    
    ctx->received = true;
    ctx->cv.notify_one();
}

int main(int argc, char* argv[]) {
    if (argc < 3) {
        std::cerr << "Usage: " << argv[0] << " <resd_file> <node_id>" << std::endl;
        return 1;
    }

    std::string resd_file = argv[1];
    uint32_t node_id = std::stoul(argv[2]);

    ResdParser parser(resd_file);
    if (!parser.init()) {
        std::cerr << "Failed to initialize RESD parser" << std::endl;
        return 1;
    }

    auto imu = parser.get_sensor(ResdSampleType::ACCELERATION, 0);

    z_owned_config_t config;
    z_config_default(&config);
    z_owned_session_t session;
    if (z_open(&session, z_move(config), NULL) != 0) {
        std::cerr << "Failed to open Zenoh session" << std::endl;
        return 1;
    }

    char topic[128];
    snprintf(topic, sizeof(topic), "sim/clock/advance/%u", node_id);
    
    char sensor_topic[128];
    snprintf(sensor_topic, sizeof(sensor_topic), "sim/sensor/%u/imu0", node_id);
    z_owned_keyexpr_t sensor_ke;
    z_keyexpr_from_str(&sensor_ke, sensor_topic);
    z_owned_publisher_t sensor_pub;
    z_declare_publisher(z_session_loan(&session), &sensor_pub, z_keyexpr_loan(&sensor_ke), NULL);
    z_keyexpr_drop(z_move(sensor_ke));

    std::cout << "[RESD Replay] Connected. Acting as TimeAuthority for node " << node_id << std::endl;

    uint64_t current_vtime = 0;
    uint64_t delta_ns = 1000000; // 1 ms

    while (true) {
        ClockAdvancePayload req = {delta_ns, 0};
        
        z_owned_keyexpr_t ke;
        z_keyexpr_from_str(&ke, topic);
        
        z_owned_bytes_t req_bytes;
        z_bytes_copy_from_buf(&req_bytes, reinterpret_cast<const uint8_t*>(&req), sizeof(req));

        z_get_options_t options;
        z_get_options_default(&options);
        options.payload = z_move(req_bytes);

        ReplyContext ctx;
        ctx.received = false;
        ctx.success = false;

        z_owned_closure_reply_t callback;
        z_closure_reply(&callback, on_reply, NULL, &ctx);

        z_get(z_session_loan(&session), z_keyexpr_loan(&ke), "", z_move(callback), &options);
        z_keyexpr_drop(z_move(ke));

        std::unique_lock<std::mutex> lock(ctx.mtx);
        if (ctx.cv.wait_for(lock, std::chrono::seconds(5), [&ctx]{ return ctx.received; })) {
            if (!ctx.success) {
                std::cerr << "[RESD Replay] Error received from Zenoh node" << std::endl;
                break;
            }
            
            current_vtime = ctx.current_vtime_ns;

            auto reading = imu->get_reading(current_vtime);
            if (reading.size() >= 3) {
                struct {
                    uint64_t vtime_ns;
                    double x, y, z;
                } __attribute__((packed)) imu_data = {current_vtime, reading[0], reading[1], reading[2]};

                z_owned_bytes_t pub_bytes;
                z_bytes_copy_from_buf(&pub_bytes, reinterpret_cast<const uint8_t*>(&imu_data), sizeof(imu_data));
                z_publisher_put(z_publisher_loan(&sensor_pub), z_move(pub_bytes), NULL);
            }

            std::cout << "[RESD Replay] Advanced to vtime: " << current_vtime << " ns" << std::endl;
        } else {
            std::cerr << "[RESD Replay] Timeout waiting for QEMU" << std::endl;
            break;
        }

        std::this_thread::sleep_for(std::chrono::milliseconds(1));
    }

    z_publisher_drop(z_move(sensor_pub));
    z_close(z_session_loan_mut(&session), NULL);
    z_session_drop(z_move(session));
    return 0;
}
