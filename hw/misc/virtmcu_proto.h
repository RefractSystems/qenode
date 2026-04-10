/*
 * virtmcu mmio-socket-bridge wire protocol.
 *
 * Shared between the QEMU QOM device (hw/misc/mmio-socket-bridge.c) and the
 * SystemC adapter (tools/systemc_adapter/main.cpp).  Keep both sides in sync
 * by including this header rather than duplicating the structs.
 *
 * Protocol (little-endian, native host byte order — both sides assumed x86_64):
 *
 *   Request  (QEMU → adapter, sizeof = 24 bytes):
 *     uint8_t  type;       0 = read, 1 = write
 *     uint8_t  size;       access width in bytes: 1, 2, 4, or 8
 *     uint16_t reserved1;  must be zero
 *     uint32_t reserved2;  must be zero
 *     uint64_t addr;       byte offset within the mapped region
 *     uint64_t data;       write value (ignored for reads)
 *
 *   Response (adapter → QEMU, sizeof = 8 bytes):
 *     uint64_t data;       read value (ignored for writes)
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#ifndef VIRTMCU_PROTO_H
#define VIRTMCU_PROTO_H

#include <stdint.h>

struct mmio_req {
    uint8_t  type;
    uint8_t  size;
    uint16_t reserved1;
    uint32_t reserved2;
    uint64_t addr;
    uint64_t data;
} __attribute__((packed));

struct mmio_resp {
    uint64_t data;
} __attribute__((packed));

#define MMIO_REQ_READ  0
#define MMIO_REQ_WRITE 1

#endif /* VIRTMCU_PROTO_H */
