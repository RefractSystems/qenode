#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-parameter"
#pragma GCC diagnostic ignored "-Wsign-compare"

#include "qemu/osdep.h"
#include "qom/object.h"
#include "hw/core/qdev.h"
#include "hw/core/sysbus.h"
#include "chardev/char.h"
#include "net/net.h"
#include "net/can_emu.h"
#include "net/can_host.h"
#include "system/memory.h"
#include "hw/core/cpu.h"
#include "qemu/thread.h"

#pragma GCC diagnostic pop
