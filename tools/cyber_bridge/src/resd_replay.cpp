/*
 * resd_replay — RESD-file-driven TimeAuthority for virtmcu.
 *
 * Parses a Renode Sensor Data (RESD) file, then acts as the Zenoh
 * TimeAuthority for a single QEMU node:
 *   1. Sends sim/clock/advance/{node_id} queries (1ms quanta by default).
 *   2. After each quantum reply, publishes every sensor channel found in the
 *      RESD file to sim/sensor/{node_id}/{sensor_name}.
 *   3. Terminates cleanly once virtual time exceeds the last sample timestamp
 *      across all sensor channels.
 *
 * Sensor Zenoh topic: sim/sensor/{node_id}/{sensor_name}
 * Payload (little-endian):
 *   uint64_t vtime_ns   — virtual time of this reading
 *   double   values[N]  — sensor values (N depends on type)
 *
 * Wire protocol (shared with hw/zenoh/zenoh-clock.c):
 *   GET payload → ClockAdvancePayload { delta_ns, mujoco_time_ns }
 *   Reply       ← ClockReadyPayload   { current_vtime_ns, n_frames }
 *
 * Usage: resd_replay <resd_file> <node_id> [delta_ns]
 */

#include "virtmcu/resd_parser.hpp"
#include <iostream>
#include <map>
#include <string>
#include <mutex>
#include <condition_variable>
#include <zenoh.h>
#include <cstring>

using namespace virtmcu;

struct ClockAdvancePayload {
    uint64_t delta_ns;
    uint64_t mujoco_time_ns; /* replay virtual time at the START of this quantum */
} __attribute__((packed));

struct ClockReadyPayload {
    uint64_t current_vtime_ns;
    uint32_t n_frames;
} __attribute__((packed));

struct ReplyContext {
    std::mutex              mtx;
    std::condition_variable cv;
    uint64_t                current_vtime_ns = 0;
    bool                    received         = false;
    bool                    success          = false;
};

static void on_reply(z_loaned_reply_t *reply, void *context)
{
    auto *ctx_shared_ptr = static_cast<std::shared_ptr<ReplyContext> *>(context);
    ReplyContext *ctx = ctx_shared_ptr->get();
    std::lock_guard<std::mutex> lock(ctx->mtx);

    if (z_reply_is_ok(reply)) {
        const z_loaned_sample_t *sample  = z_reply_ok(reply);
        const z_loaned_bytes_t  *payload = z_sample_payload(sample);
        z_bytes_reader_t         reader  = z_bytes_get_reader(payload);
        ClockReadyPayload rep;
        if (z_bytes_reader_read(&reader,
                reinterpret_cast<uint8_t *>(&rep), sizeof(rep)) == sizeof(rep)) {
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

int main(int argc, char *argv[])
{
    if (argc < 3) {
        std::cerr << "Usage: " << argv[0]
                  << " <resd_file> <node_id> [delta_ns]\n";
        return 1;
    }

    const std::string resd_file = argv[1];
    const uint32_t    node_id   = std::stoul(argv[2]);
    const uint64_t    delta_ns  = (argc >= 4) ? std::stoull(argv[3]) : 1'000'000ULL;

    /* ── Parse RESD file ─────────────────────────────────────────────── */
    ResdParser parser(resd_file);
    if (!parser.init()) {
        std::cerr << "[RESD Replay] Failed to parse " << resd_file << "\n";
        return 1;
    }

    const auto &all_sensors    = parser.get_all_sensors();
    const uint64_t last_ts_ns  = parser.get_last_timestamp();

    if (all_sensors.empty()) {
        std::cerr << "[RESD Replay] No sensor channels found in " << resd_file << "\n";
        return 1;
    }

    std::cout << "[RESD Replay] Parsed " << all_sensors.size()
              << " sensor channel(s). Last timestamp: " << last_ts_ns << " ns\n";

    /* ── Open Zenoh session ──────────────────────────────────────────── */
    z_owned_config_t  config;
    z_config_default(&config);
    
    /* Support ZENOH_CONNECT env var for easy testing/deployment locators */
    const char* connect_env = std::getenv("ZENOH_CONNECT");
    if (connect_env) {
        zc_config_insert_json5(z_config_loan_mut(&config), "connect/endpoints", connect_env);
    }

    z_owned_session_t session;
    if (z_open(&session, z_move(config), NULL) != 0) {
        std::cerr << "[RESD Replay] Failed to open Zenoh session\n";
        return 1;
    }

    /* ── Declare one publisher per sensor channel ────────────────────── */
    std::map<std::pair<ResdSampleType, uint16_t>, z_owned_publisher_t> publishers;
    for (const auto &kv : all_sensors) {
        char topic[256];
        snprintf(topic, sizeof(topic), "sim/sensor/%u/%s",
                 node_id, kv.second->get_name().c_str());

        z_owned_keyexpr_t ke;
        z_keyexpr_from_str(&ke, topic);
        z_owned_publisher_t pub;
        if (z_declare_publisher(z_session_loan(&session), &pub,
                                z_keyexpr_loan(&ke), NULL) != 0) {
            std::cerr << "[RESD Replay] Failed to declare publisher for " << topic << "\n";
        } else {
            publishers.emplace(kv.first, std::move(pub));
            std::cout << "[RESD Replay] Publishing " << topic << "\n";
        }
        z_keyexpr_drop(z_move(ke));
    }

    std::cout << "[RESD Replay] Connected. Acting as TimeAuthority for node "
              << node_id << "\n";

    /* ── Build clock-advance topic ───────────────────────────────────── */
    char clock_topic[128];
    snprintf(clock_topic, sizeof(clock_topic), "sim/clock/advance/%u", node_id);

    uint64_t current_vtime = 0;

    /* ── Main replay loop ────────────────────────────────────────────── */
    while (current_vtime <= last_ts_ns) {
        /* Send clock advance. mujoco_time_ns = start of this quantum so
         * zenoh-clock can log the replay-time/vtime relationship. */
        ClockAdvancePayload req = {delta_ns, current_vtime};

        z_owned_keyexpr_t ke;
        z_keyexpr_from_str(&ke, clock_topic);

        z_owned_bytes_t req_bytes;
        z_bytes_copy_from_buf(&req_bytes,
                reinterpret_cast<const uint8_t *>(&req), sizeof(req));

        z_get_options_t options;
        z_get_options_default(&options);
        options.payload = z_move(req_bytes);

        /* 
         * Use a heap-allocated shared_ptr to ensure the context lives as long
         * as Zenoh might call the callback OR the drop function.
         */
        auto ctx = std::make_shared<ReplyContext>();
        auto *ctx_ptr = new std::shared_ptr<ReplyContext>(ctx);

        z_owned_closure_reply_t callback;
        z_closure_reply(&callback, on_reply, 
            [](void* p){ delete static_cast<std::shared_ptr<ReplyContext>*>(p); }, 
            ctx_ptr);

        z_get(z_session_loan(&session), z_keyexpr_loan(&ke), "",
              z_move(callback), &options);
        z_keyexpr_drop(z_move(ke));

        {
            std::unique_lock<std::mutex> lock(ctx->mtx);
            if (!ctx->cv.wait_for(lock, std::chrono::seconds(5),
                                  [&] { return ctx->received; })) {
                std::cerr << "[RESD Replay] Timeout waiting for QEMU\n";
                return 1;
            }
            if (!ctx->success) {
                std::cerr << "[RESD Replay] Error reply from QEMU\n";
                return 1;
            }
            current_vtime = ctx->current_vtime_ns;
        }

        /* ── Publish all sensor channels at the new virtual time ─────── */
        for (const auto &kv : all_sensors) {
            auto pub_it = publishers.find(kv.first);
            if (pub_it == publishers.end()) continue;

            std::vector<double> vals = kv.second->get_reading(current_vtime);
            if (vals.empty()) continue;

            /* Payload: vtime_ns (8 bytes) + double values */
            std::vector<uint8_t> buf(sizeof(uint64_t) + vals.size() * sizeof(double));
            memcpy(buf.data(), &current_vtime, sizeof(uint64_t));
            memcpy(buf.data() + sizeof(uint64_t), vals.data(),
                   vals.size() * sizeof(double));

            z_owned_bytes_t pub_bytes;
            z_bytes_copy_from_buf(&pub_bytes, buf.data(), buf.size());
            z_publisher_put(z_publisher_loan(&pub_it->second),
                            z_move(pub_bytes), NULL);
        }

        std::cout << "[RESD Replay] Advanced to vtime: " << current_vtime << " ns\n";
    }

    std::cout << "[RESD Replay] Replay complete at vtime " << current_vtime << " ns\n";

    /* ── Cleanup ─────────────────────────────────────────────────────── */
    for (auto &kv : publishers) {
        z_publisher_drop(z_move(kv.second));
    }
    z_close(z_session_loan_mut(&session), NULL);
    z_session_drop(z_move(session));
    return 0;
}
