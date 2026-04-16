/*
 * virtmcu mmio-socket-bridge wire protocol.
 *
 * Shared between the QEMU QOM device (hw/misc/mmio-socket-bridge.c) and the
 * SystemC adapter (tools/systemc_adapter/main.cpp).  Keep both sides in sync
 * by including this header rather than duplicating the structs.
 *
 * Protocol (little-endian, native host byte order — both sides assumed x86_64):
 *
 *   Request  (QEMU → adapter, sizeof = 32 bytes):
 *     uint8_t  type;       0 = read, 1 = write
 *     uint8_t  size;       access width in bytes: 1, 2, 4, or 8
 *     uint16_t reserved1;  must be zero
 *     uint32_t reserved2;  must be zero
 *     uint64_t vtime_ns;   QEMU virtual time in nanoseconds
 *     uint64_t addr;       byte offset within the mapped region (NOT absolute address)
 *     uint64_t data;       write value (ignored for reads)
 *
 *   Message (adapter → QEMU, sizeof = 16 bytes):
 *     uint32_t type;       0 = RESP, 1 = IRQ_SET, 2 = IRQ_CLEAR
 *     uint32_t irq_num;    IRQ index (ignored for RESP)
 *     uint64_t data;       read value (ignored for writes and IRQs)
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#ifndef VIRTMCU_PROTO_H
#define VIRTMCU_PROTO_H

#include <stdint.h>

#define VIRTMCU_PROTO_MAGIC   0x564D4355 /* "VMCU" */
#define VIRTMCU_PROTO_VERSION 1

struct virtmcu_handshake {
    uint32_t magic;
    uint32_t version;
} __attribute__((packed));

#define MMIO_REQ_READ  0
#define MMIO_REQ_WRITE 1

struct mmio_req {
    uint8_t  type;
    uint8_t  size;
    uint16_t reserved1;
    uint32_t reserved2;
    uint64_t vtime_ns;
    uint64_t addr;
    uint64_t data;
} __attribute__((packed));

#define SYSC_MSG_RESP      0
#define SYSC_MSG_IRQ_SET   1
#define SYSC_MSG_IRQ_CLEAR 2

struct sysc_msg {
    uint32_t type;
    uint32_t irq_num;
    uint64_t data;
} __attribute__((packed));

#endif /* VIRTMCU_PROTO_H */
