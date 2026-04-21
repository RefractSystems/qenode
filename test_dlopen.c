#include <dlfcn.h>
#include <stdio.h>

int main() {
    void *handle = dlopen("/workspace/third_party/qemu/build-virtmcu/install/lib/aarch64-linux-gnu/qemu/hw-virtmcu-rust-dummy.so", RTLD_NOW | RTLD_GLOBAL);
    if (!handle) {
        printf("Error: %s\n", dlerror());
        return 1;
    }
    printf("Loaded successfully\n");
    return 0;
}
