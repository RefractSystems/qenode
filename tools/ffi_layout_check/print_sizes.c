#include "chardev/char-fe.h"
#include "chardev/char.h"
#include "hw/core/qdev.h"
#include "hw/core/sysbus.h"
#include "hw/ssi/ssi.h"
#include "qemu/osdep.h"
#include "qom/object.h"
#include "system/memory.h"
#include <stddef.h>
#include <stdio.h>

int main() {
  printf("Object size: %zu, align: %zu\n", sizeof(Object), _Alignof(Object));
  printf("ObjectClass size: %zu, align: %zu\n", sizeof(ObjectClass),
         _Alignof(ObjectClass));
  printf("DeviceState size: %zu, align: %zu\n", sizeof(DeviceState),
         _Alignof(DeviceState));
  printf("SysBusDevice size: %zu, align: %zu\n", sizeof(SysBusDevice),
         _Alignof(SysBusDevice));
  printf("MemoryRegion size: %zu, align: %zu\n", sizeof(MemoryRegion),
         _Alignof(MemoryRegion));
  printf("Chardev size: %zu, align: %zu\n", sizeof(Chardev), _Alignof(Chardev));
  printf("ChardevClass size: %zu, align: %zu\n", sizeof(ChardevClass),
         _Alignof(ChardevClass));
  printf("CharFrontend size: %zu, align: %zu\n", sizeof(CharFrontend),
         _Alignof(CharFrontend));
  printf("SSIPeripheral size: %zu, align: %zu\n", sizeof(SSIPeripheral),
         _Alignof(SSIPeripheral));
  printf("SSIPeripheralClass size: %zu, align: %zu\n",
         sizeof(SSIPeripheralClass), _Alignof(SSIPeripheralClass));

  return 0;
}
