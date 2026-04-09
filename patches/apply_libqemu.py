#!/usr/bin/env python3
"""
apply_libqemu.py — Inject the libqemu clock-socket extension into a QEMU source tree.

This adds a -clocksock <path> option to qemu-system-* that opens a Unix socket,
accepts a connection from a NodeAgent, and advances virtual time on demand.

Usage:
    python3 apply_libqemu.py <qemu-source-dir>

Idempotent: safe to run multiple times.

Tested against: QEMU 11.0.0-rc2 (commit ~April 2026).

Internal APIs used (verify if rebasing to a new QEMU version):
  - timers_state.qemu_icount_bias  (accel/tcg/icount-common.c)
  - icount_get()                   (include/exec/icount.h)
  - bql_lock() / bql_unlock()      (include/system/cpus.h)
  - qemu_clock_run_all_timers()    (include/qemu/timer.h)
  - info_report()                  (include/qemu/error-report.h)
"""

import os
import sys
import textwrap


def write_if_changed(path, content):
    """Write content to path only if it differs. Returns True if written."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    if os.path.exists(path):
        with open(path) as f:
            if f.read() == content:
                return False
    with open(path, "w") as f:
        f.write(content)
    print(f"  wrote {os.path.relpath(path)}")
    return True


def patch_file(path, marker, insertion, after=True):
    """Insert `insertion` once into `path`, positioned after/before `marker`."""
    with open(path) as f:
        content = f.read()
    if marker not in content:
        print(f"  WARNING: marker not found in {os.path.relpath(path)}: {marker!r}")
        return False
    if insertion.strip() in content:
        return False  # already applied
    if after:
        content = content.replace(marker, marker + insertion, 1)
    else:
        content = content.replace(marker, insertion + marker, 1)
    with open(path, "w") as f:
        f.write(content)
    print(f"  patched {os.path.relpath(path)}")
    return True


def main():
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <qemu-source-dir>")
        sys.exit(1)

    qemu = os.path.abspath(sys.argv[1])
    if not os.path.isdir(qemu):
        print(f"ERROR: {qemu} is not a directory")
        sys.exit(1)

    print(f"Applying libqemu clock-socket extension to: {qemu}")

    # ── 1. include/libqemu/libqemu.h ─────────────────────────────────────────
    write_if_changed(
        os.path.join(qemu, "include", "libqemu", "libqemu.h"),
        textwrap.dedent("""\
        /*
         * libqemu — external virtual clock control API.
         *
         * Activate by passing -clocksock <path> to qemu-system-*.
         * A NodeAgent connects to the Unix socket and sends ClockAdvance
         * messages to advance QEMU's virtual time quantum by quantum.
         *
         * Wire protocol (little-endian, packed structs):
         *   Host → QEMU:  ClockAdvance { uint64 delta_ns; uint64 mujoco_time_ns; }
         *   QEMU → Host:  ClockReady   { uint64 current_vtime_ns; uint32 n_frames; }
         *   (followed by n_frames × EthFrameHeader + payload, reserved for Phase 7)
         */
        #ifndef LIBQEMU_H
        #define LIBQEMU_H

        #include <stdint.h>

        #ifdef __cplusplus
        extern "C" {
        #endif

        /* Start the clock server thread listening on socket_path. */
        void libqemu_clock_server_init(const char *socket_path);

        /* Advance QEMU virtual clock by delta_ns nanoseconds. */
        void libqemu_clock_advance(int64_t delta_ns);

        /* Return current QEMU virtual time in nanoseconds. */
        int64_t libqemu_clock_get_ns(void);

        #ifdef __cplusplus
        }
        #endif

        #endif /* LIBQEMU_H */
        """),
    )

    # ── 2. libqemu/libqemu.c ─────────────────────────────────────────────────
    write_if_changed(
        os.path.join(qemu, "libqemu", "libqemu.c"),
        textwrap.dedent("""\
        /*
         * libqemu/libqemu.c — external virtual clock control server.
         *
         * SPDX-License-Identifier: GPL-2.0-or-later
         */

        #include "qemu/osdep.h"
        #include "qemu/error-report.h"
        #include "qemu/main-loop.h"
        #include "qemu/seqlock.h"
        #include "qemu/thread.h"
        #include "qemu/timer.h"
        #include "system/cpus.h"
        #include "system/cpu-timers.h"
        #include "system/cpu-timers-internal.h"
        #include "exec/icount.h"
        #include "libqemu/libqemu.h"

        #include <sys/socket.h>
        #include <sys/un.h>
        #include <pthread.h>
        #include <errno.h>

        /* Wire protocol — must match node_agent exactly. */
        typedef struct __attribute__((packed)) {
            uint64_t delta_ns;
            uint64_t mujoco_time_ns;
        } ClockAdvancePayload;

        typedef struct __attribute__((packed)) {
            uint64_t current_vtime_ns;
            uint32_t n_frames;           /* reserved: always 0 until Phase 7 */
        } ClockReadyPayload;

        /* ── Public API ──────────────────────────────────────────────────── */

        void libqemu_clock_advance(int64_t delta_ns)
        {
            int64_t current = qatomic_read(&timers_state.qemu_icount_bias);
            qatomic_set(&timers_state.qemu_icount_bias, current + delta_ns);
            qemu_clock_run_all_timers();
        }

        int64_t libqemu_clock_get_ns(void)
        {
            return icount_get();
        }

        /* ── I/O helpers ─────────────────────────────────────────────────── */

        static int read_exact(int fd, void *buf, size_t len)
        {
            size_t done = 0;
            while (done < len) {
                ssize_t n = read(fd, (uint8_t *)buf + done, len - done);
                if (n <= 0) {
                    return -1;
                }
                done += n;
            }
            return 0;
        }

        static int write_exact(int fd, const void *buf, size_t len)
        {
            size_t done = 0;
            while (done < len) {
                ssize_t n = write(fd, (const uint8_t *)buf + done, len - done);
                if (n <= 0) {
                    return -1;
                }
                done += n;
            }
            return 0;
        }

        /* ── Server thread ───────────────────────────────────────────────── */

        static void *clock_server_thread(void *arg)
        {
            const char *socket_path = (const char *)arg;
            int srv, fd;
            struct sockaddr_un addr = {0};

            srv = socket(AF_UNIX, SOCK_STREAM, 0);
            if (srv < 0) {
                error_report("libqemu: socket(): %s", strerror(errno));
                return NULL;
            }

            addr.sun_family = AF_UNIX;
            strncpy(addr.sun_path, socket_path, sizeof(addr.sun_path) - 1);
            unlink(socket_path);

            if (bind(srv, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
                error_report("libqemu: bind(%s): %s", socket_path, strerror(errno));
                close(srv);
                return NULL;
            }
            if (listen(srv, 1) < 0) {
                error_report("libqemu: listen(): %s", strerror(errno));
                close(srv);
                return NULL;
            }

            info_report("libqemu: waiting for node agent on %s", socket_path);
            fd = accept(srv, NULL, NULL);
            if (fd < 0) {
                error_report("libqemu: accept(): %s", strerror(errno));
                close(srv);
                return NULL;
            }
            info_report("libqemu: node agent connected");

            for (;;) {
                ClockAdvancePayload cmd;
                if (read_exact(fd, &cmd, sizeof(cmd)) < 0) {
                    break;
                }

                bql_lock();
                libqemu_clock_advance((int64_t)cmd.delta_ns);
                int64_t vtime = libqemu_clock_get_ns();
                bql_unlock();

                ClockReadyPayload ready = {
                    .current_vtime_ns = (uint64_t)vtime,
                    .n_frames         = 0,
                };
                if (write_exact(fd, &ready, sizeof(ready)) < 0) {
                    break;
                }
            }

            info_report("libqemu: node agent disconnected");
            close(fd);
            close(srv);
            return NULL;
        }

        void libqemu_clock_server_init(const char *socket_path)
        {
            pthread_t t;
            pthread_create(&t, NULL, clock_server_thread, g_strdup(socket_path));
            pthread_detach(t);
        }
        """),
    )

    # ── 3. libqemu/meson.build ───────────────────────────────────────────────
    write_if_changed(
        os.path.join(qemu, "libqemu", "meson.build"),
        textwrap.dedent("""\
        # libqemu — compiled into system_ss so it is always available when the
        # binary is built with --target-list=*-softmmu.
        system_ss.add(files('libqemu.c'))
        """),
    )

    # ── 4. Hook libqemu/meson.build into the top-level build ─────────────────
    top_meson = os.path.join(qemu, "meson.build")
    patch_file(
        top_meson,
        "if have_system\n  subdir('hw')",
        "\nif have_system\n  subdir('libqemu')\nendif\n",
        after=False,
    )

    # ── 5. Add -clocksock option to qemu-options.hx ──────────────────────────
    hx_path = os.path.join(qemu, "qemu-options.hx")
    patch_file(
        hx_path,
        'DEF("qtest",',
        textwrap.dedent("""\
        DEF("clocksock", HAS_ARG, QEMU_OPTION_clocksock,
            "-clocksock path\\n"
            "            Unix socket path for external virtual clock control.\\n"
            "            Requires -icount. A NodeAgent connects here to advance\\n"
            "            virtual time one physics quantum at a time.\\n",
            QEMU_ARCH_ALL)
        SRST
        ``-clocksock path``
            Open a Unix domain socket at *path* and accept a single connection
            from a NodeAgent (see tools/node_agent/).  The agent sends
            ``ClockAdvance{delta_ns}`` messages to step the virtual clock.
            Requires ``-icount shift=0,align=off,sleep=off``.
        ERST

        """),
        after=False,
    )

    # ── 6. Wire option into system/vl.c ──────────────────────────────────────
    vl_path = os.path.join(qemu, "system", "vl.c")

    # Add #include
    patch_file(
        vl_path,
        '#include "qapi/qobject-input-visitor.h"\n',
        '#include "libqemu/libqemu.h"\n',
        after=True,
    )

    # Handle the option in the switch statement
    patch_file(
        vl_path,
        "case QEMU_OPTION_qtest:\n",
        textwrap.dedent("""\
        case QEMU_OPTION_clocksock:
                    libqemu_clock_server_init(optarg);
                    break;
                """),
        after=False,
    )

    print("\nDone. Verify with: grep -r 'clocksock' <qemu-src>/")


if __name__ == "__main__":
    main()
