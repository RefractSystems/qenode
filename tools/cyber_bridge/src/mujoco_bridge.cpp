/*
 * mujoco_bridge — Zero-copy MuJoCo ↔ QEMU Cyber-Physical Bridge.
 *
 * This bridge runs alongside a MuJoCo process that exposes its mjData via a
 * POSIX shared memory segment, and alongside a QEMU instance running with the
 * zenoh-clock plugin (suspend mode).  It acts as the Zenoh TimeAuthority:
 *
 *   For each physics step:
 *     1. Call mj_step() (or wait for the MuJoCo process to do so).
 *     2. Send sim/clock/advance/{node_id} — QEMU firmware runs one quantum.
 *     3. Receive ClockReadyPayload → current_vtime_ns.
 *     4. Read mjData->sensordata → publish to Zenoh sensor topics so firmware
 *        can poll them via the mmio-socket-bridge SAL peripheral.
 *     5. Subscribe to sim/actuator/{node_id}/* → write to mjData->ctrl.
 *
 * Zero-copy path:
 *   The MuJoCo process (Python or C++) opens a POSIX shm segment named
 *   "/virtmcu_mujoco_{node_id}" and writes a MjSharedLayout header followed
 *   by the flat sensordata and ctrl arrays.  This bridge mmap()s the same
 *   segment, reads sensordata, and writes ctrl without any serialisation.
 *
 * VirtmcuQuantumTiming integration (Phase 7.7):
 *   The ClockAdvancePayload.mujoco_time_ns field carries the MuJoCo simulation
 *   time (in ns) at the start of each quantum.  Inside QEMU, zenoh-clock.c
 *   stores this in s->mujoco_time_ns, making it available to QEMU-internal
 *   SAL/AAL modules via virtmcu_get_quantum_timing().  External tools (like
 *   this bridge) instead read it back from the ClockReadyPayload reply and use
 *   it to align their next mj_step() call:
 *
 *       interpolated_physics_t = quantum_start_vtime_ns + fraction * delta_ns
 *
 * Shared-memory layout (MjSharedLayout):
 *   uint32_t  nsensordata   — number of sensor outputs
 *   uint32_t  nu            — number of actuators
 *   uint64_t  mujoco_time_ns — MuJoCo simulation time (ns), written by MuJoCo
 *   double    sensordata[nsensordata]
 *   double    ctrl[nu]
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "virtmcu/sal_aal.hpp"
#include <iostream>
#include <string>
#include <vector>
#include <mutex>
#include <condition_variable>
#include <atomic>
#include <thread>
#include <zenoh.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <cstring>
#include <csignal>
#include <cstdio>
#include <memory>

using namespace virtmcu;

/* ── Shared memory layout ────────────────────────────────────────────────── */

struct MjSharedLayout {
    uint32_t nsensordata;        /* number of sensor outputs */
    uint32_t nu;                 /* number of actuator inputs */
    uint64_t mujoco_time_ns;     /* MuJoCo sim time, written by MuJoCo process */
    /* Followed by:
     *   double sensordata[nsensordata]   (read by this bridge → Zenoh)
     *   double ctrl[nu]                  (written by this bridge ← Zenoh)
     */
} __attribute__((packed));

static inline double *shm_sensordata(MjSharedLayout *hdr) {
    return reinterpret_cast<double *>(hdr + 1);
}

static inline double *shm_ctrl(MjSharedLayout *hdr) {
    return shm_sensordata(hdr) + hdr->nsensordata;
}

/* ── Zenoh wire protocol (shared with hw/zenoh/zenoh-clock.c) ─────────────── */

struct ClockAdvancePayload {
    uint64_t delta_ns;
    uint64_t mujoco_time_ns; /* MuJoCo time at start of quantum → zenoh-clock */
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

/* ── Actuator subscriber: Zenoh → mjData->ctrl ────────────────────────────── */

struct ActuatorContext {
    MjSharedLayout *shm;
    int             ctrl_idx;
    std::mutex      mtx;
};

static void on_actuator(z_loaned_sample_t *sample, void *context)
{
    auto *ctx = static_cast<ActuatorContext *>(context);
    const z_loaned_bytes_t *payload = z_sample_payload(sample);
    if (!payload) return;

    z_bytes_reader_t reader = z_bytes_get_reader(payload);
    double val;
    if (z_bytes_reader_read(&reader,
            reinterpret_cast<uint8_t *>(&val), sizeof(val)) == sizeof(val)) {
        std::lock_guard<std::mutex> lock(ctx->mtx);
        if (ctx->shm && ctx->ctrl_idx < (int)ctx->shm->nu) {
            shm_ctrl(ctx->shm)[ctx->ctrl_idx] = val;
        }
    }
}

/* ── Signal handler for graceful shutdown ────────────────────────────────── */
static std::atomic<bool> g_running{true};
static void on_signal(int) { g_running = false; }

/* ── main ─────────────────────────────────────────────────────────────────── */

int main(int argc, char *argv[])
{
    if (argc < 3) {
        std::cerr << "Usage: " << argv[0]
                  << " <node_id> <nu> [nsensordata] [delta_ns]\n"
                  << "  node_id     — QEMU node index (used in Zenoh topics)\n"
                  << "  nu          — number of actuator channels\n"
                  << "  nsensordata — number of sensor outputs (default: 6)\n"
                  << "  delta_ns    — quantum size in ns (default: 1000000 = 1ms)\n";
        return 1;
    }

    const uint32_t node_id     = std::stoul(argv[1]);
    const uint32_t nu          = std::stoul(argv[2]);
    const uint32_t nsensordata = (argc >= 4) ? std::stoul(argv[3]) : 6;
    const uint64_t delta_ns    = (argc >= 5) ? std::stoull(argv[4]) : 1'000'000ULL;

    std::signal(SIGINT,  on_signal);
    std::signal(SIGTERM, on_signal);

    /* ── Open shared memory ──────────────────────────────────────────── */
    char shm_name[64];
    snprintf(shm_name, sizeof(shm_name), "/virtmcu_mujoco_%u", node_id);

    const size_t shm_size = sizeof(MjSharedLayout)
                          + (nsensordata + nu) * sizeof(double);

    int shm_fd = shm_open(shm_name, O_RDWR, 0666);
    if (shm_fd < 0) {
        /* First run: create and initialise the segment */
        shm_fd = shm_open(shm_name, O_CREAT | O_RDWR, 0666);
        if (shm_fd < 0) {
            perror("[MuJoCo Bridge] shm_open");
            return 1;
        }
        if (ftruncate(shm_fd, shm_size) < 0) {
            perror("[MuJoCo Bridge] ftruncate");
            close(shm_fd);
            return 1;
        }
        std::cout << "[MuJoCo Bridge] Created shared memory segment "
                  << shm_name << " (" << shm_size << " bytes)\n";
    }

    void *ptr = mmap(nullptr, shm_size,
                     PROT_READ | PROT_WRITE, MAP_SHARED, shm_fd, 0);
    if (ptr == MAP_FAILED) {
        perror("[MuJoCo Bridge] mmap");
        close(shm_fd);
        return 1;
    }
    close(shm_fd);

    auto *shm = static_cast<MjSharedLayout *>(ptr);
    shm->nsensordata  = nsensordata;
    shm->nu           = nu;
    shm->mujoco_time_ns = 0;

    std::cout << "[MuJoCo Bridge] Shared memory ready: "
              << nsensordata << " sensor outputs, "
              << nu << " actuator inputs\n";
    std::cout << "[MuJoCo Bridge] MuJoCo process should map " << shm_name
              << " and write sensordata / read ctrl at this layout\n";

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
        std::cerr << "[MuJoCo Bridge] Failed to open Zenoh session\n";
        munmap(ptr, shm_size);
        return 1;
    }

    /* ── Sensor publishers: mjData->sensordata → Zenoh ──────────────── */
    std::vector<z_owned_publisher_t> sensor_pubs(nsensordata);
    for (uint32_t i = 0; i < nsensordata; i++) {
        char topic[128];
        snprintf(topic, sizeof(topic), "sim/sensor/%u/sensordata_%u", node_id, i);
        z_owned_keyexpr_t ke;
        z_keyexpr_from_str(&ke, topic);
        z_declare_publisher(z_session_loan(&session), &sensor_pubs[i],
                            z_keyexpr_loan(&ke), NULL);
        z_keyexpr_drop(z_move(ke));
    }

    /* ── Actuator subscribers: Zenoh → mjData->ctrl ──────────────────── */
    std::vector<ActuatorContext>      act_ctxs(nu);
    std::vector<z_owned_subscriber_t> act_subs(nu);
    for (uint32_t i = 0; i < nu; i++) {
        act_ctxs[i].shm      = shm;
        act_ctxs[i].ctrl_idx = i;

        char topic[128];
        snprintf(topic, sizeof(topic), "sim/actuator/%u/ctrl_%u", node_id, i);
        z_owned_keyexpr_t ke;
        z_keyexpr_from_str(&ke, topic);
        z_owned_closure_sample_t cb;
        z_closure_sample(&cb, on_actuator, NULL, &act_ctxs[i]);
        z_declare_subscriber(z_session_loan(&session), &act_subs[i],
                             z_keyexpr_loan(&ke), z_move(cb), NULL);
        z_keyexpr_drop(z_move(ke));
    }

    /* ── Build clock-advance topic ───────────────────────────────────── */
    char clock_topic[128];
    snprintf(clock_topic, sizeof(clock_topic), "sim/clock/advance/%u", node_id);

    std::cout << "[MuJoCo Bridge] Ready to synchronize Zenoh Clock with MuJoCo mj_step()\n";

    uint64_t current_vtime = 0;

    /* ── Main loop ───────────────────────────────────────────────────── */
    while (g_running) {
        /*
         * Phase 7.7 VirtmcuQuantumTiming integration:
         * mujoco_time_ns in the request = current MuJoCo simulation time from
         * the shared memory header.  Inside QEMU, zenoh-clock.c stores this in
         * s->mujoco_time_ns, making it available to QEMU-internal SAL/AAL
         * modules via virtmcu_get_quantum_timing().
         */
        const uint64_t mujoco_t = shm->mujoco_time_ns;
        ClockAdvancePayload req = {delta_ns, mujoco_t};

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
                std::cerr << "[MuJoCo Bridge] Timeout waiting for QEMU\n";
                return 1;
            }
            if (!ctx->success) {
                std::cerr << "[MuJoCo Bridge] Error reply from QEMU\n";
                return 1;
            }
            current_vtime = ctx->current_vtime_ns;
        }

        /*
         * Publish sensor readings from sensordata[].
         * In a real deployment the MuJoCo process has already advanced one
         * step and written fresh sensordata before we read here.
         * Payload: vtime_ns(8) + double value(8) = 16 bytes per channel.
         */
        const double *sdata = shm_sensordata(shm);
        for (uint32_t i = 0; i < nsensordata; i++) {
            struct { uint64_t vtime_ns; double value; } __attribute__((packed))
                pkt = {current_vtime, sdata[i]};
            z_owned_bytes_t pub_bytes;
            z_bytes_copy_from_buf(&pub_bytes,
                    reinterpret_cast<const uint8_t *>(&pkt), sizeof(pkt));
            z_publisher_put(z_publisher_loan(&sensor_pubs[i]),
                            z_move(pub_bytes), NULL);
        }
    }

    /* ── Cleanup ─────────────────────────────────────────────────────── */
    for (auto &sub : act_subs)  z_subscriber_drop(z_move(sub));
    for (auto &pub : sensor_pubs) z_publisher_drop(z_move(pub));
    z_close(z_session_loan_mut(&session), NULL);
    z_session_drop(z_move(session));
    munmap(ptr, shm_size);
    shm_unlink(shm_name);
    return 0;
}
