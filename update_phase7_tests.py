#!/usr/bin/env python3
import os
import glob

def patch_test_file(filepath):
    print(f"Patching {filepath}...")
    with open(filepath, 'r') as f:
        content = f.read()

    # 1. Add the mock router startup before QEMU launches
    if "python3 -u \"$WORKSPACE_DIR/tests/zenoh_router_mock.py\"" not in content:
        # Find where QEMU is launched and insert router
        if "Launch QEMU" in content or "echo \"=== suspend mode ===\"" in content:
            # For smoke_test.sh
            content = content.replace(
                "echo \"=== suspend mode ===\"", 
                "python3 -u \"$WORKSPACE_DIR/tests/zenoh_router_mock.py\" &\nROUTER_PID_1=$!\nsleep 1\necho \"=== suspend mode ===\""
            )
            content = content.replace(
                "wait \"$QEMU_PID\" 2>/dev/null || true",
                "wait \"$QEMU_PID\" 2>/dev/null || true\nkill -9 \"$ROUTER_PID_1\" 2>/dev/null || true",
                1 # Only replace the first instance (suspend mode)
            )
            
            content = content.replace(
                "echo \"=== icount mode ===\"", 
                "python3 -u \"$WORKSPACE_DIR/tests/zenoh_router_mock.py\" &\nROUTER_PID_2=$!\nsleep 1\necho \"=== icount mode ===\""
            )
            content = content.replace(
                "wait \"$QEMU_PID\" 2>/dev/null || true\n\necho \"=== Phase 7",
                "wait \"$QEMU_PID\" 2>/dev/null || true\nkill -9 \"$ROUTER_PID_2\" 2>/dev/null || true\n\necho \"=== Phase 7"
            )
            
            # For netdev_test.sh and determinism_test.sh
            content = content.replace(
                "# ── Launch QEMU ──────────────────────────────────────────────────────────────",
                "# ── Launch Router & QEMU ─────────────────────────────────────────────────────\npython3 -u \"$WORKSPACE_DIR/tests/zenoh_router_mock.py\" &\nROUTER_PID=$!\nsleep 1"
            )
            content = content.replace(
                "trap cleanup EXIT",
                "cleanup() {\n    [[ -n \"$QEMU_PID\" ]] && kill -9 \"$QEMU_PID\" 2>/dev/null || true\n    [[ -n \"$ROUTER_PID\" ]] && kill -9 \"$ROUTER_PID\" 2>/dev/null || true\n    rm -rf \"$TMPDIR_LOCAL\"\n}\ntrap cleanup EXIT"
            )

    # 2. Inject the router property into the QEMU flags
    content = content.replace("-device zenoh-clock,mode=suspend,node=0", "-device zenoh-clock,mode=suspend,node=0,router=tcp/127.0.0.1:7447")
    content = content.replace("-device zenoh-clock,mode=icount,node=0", "-device zenoh-clock,mode=icount,node=0,router=tcp/127.0.0.1:7447")
    content = content.replace("-device zenoh-clock,mode=icount,node=1", "-device zenoh-clock,mode=icount,node=1,router=tcp/127.0.0.1:7447")
    content = content.replace("-netdev zenoh,node=1,id=n1", "-netdev zenoh,node=1,id=n1,router=tcp/127.0.0.1:7447")

    with open(filepath, 'w') as f:
        f.write(content)
    print("Success.")

if __name__ == "__main__":
    files = glob.glob("/Users/marcin/src/virtmcu/test/phase7/*_test.sh")
    for f in files:
        patch_test_file(f)
    print("Finished patching Phase 7 tests to strictly use TCP routing.")
