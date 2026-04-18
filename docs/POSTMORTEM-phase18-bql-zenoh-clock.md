# Postmortem: Phase 18.9 — BQL/TLS Deadlock & Protocol Bugs in zenoh-clock

**Date:** 2026-04-18  
**Component:** `hw/zenoh/zenoh-clock.c`, `hw/rust/zenoh-clock/src/lib.rs`  
**Severity:** Critical — simulator hangs deterministically after the first clock query  
**Status:** Resolved

---

## Background

Phase 18.9 migrated the Zenoh clock backend from `zenoh-c` (C FFI) to a native Rust library
(`hw/rust/zenoh-clock/`) linked via `virtmcu-rust-ffi`. The Rust library exposes three C-callable
functions: `zenoh_clock_init`, `zenoh_clock_free`, and `zenoh_clock_quantum_wait`.

During testing, QEMU would accept the first Zenoh clock query but then hang permanently on the
second query. The Phase 7 smoke test timed out at the 30-second queryable-readiness check.

Three independent bugs were present simultaneously, which made isolation particularly difficult:
each fix exposed the next bug rather than restoring full function.

---

## Bug 1 — Unconditional BQL Sandwich (Deadlock)

### Symptom

QEMU hung silently after `zenoh_clock_quantum_wait` returned. No crash, no assertion, no log
output. The vCPU thread never resumed executing translation blocks (TBs).

### Root Cause: Two Hook Call Sites with Different BQL State

`virtmcu_tcg_quantum_hook` is called from two places in the patched QEMU:

| Call site | File | BQL held? |
|-----------|------|-----------|
| TB execution loop (`cpu_exec_loop`) | `accel/tcg/cpu-exec.c:947` | **No** |
| CPU idle/halt path | `system/cpus.c:472` | **Yes** |

The original code did an unconditional BQL sandwich:

```c
// WRONG
bql_unlock();
int64_t delta = zenoh_clock_quantum_wait(rust_state, now);
bql_lock();
```

When the hook fired from `cpu_exec_loop` (BQL not held), `bql_unlock()` was a no-op but
`bql_lock()` at the end acquired the lock. QEMU's mttcg loop then called `bql_lock()` again
— deadlock.

### The TLS Trap

The natural fix is `if (bql_locked()) { bql_unlock(); }`. But `bql_locked()` uses
`QEMU_DEFINE_STATIC_CO_TLS(bool, bql_locked)` in `system/cpus.c`:

```c
// cpus.c — file-static TLS
QEMU_DEFINE_STATIC_CO_TLS(bool, bql_locked)

bool bql_locked(void) { return get_bql_locked(); }
```

The `QEMU_DEFINE_STATIC_CO_TLS` macro defines a **file-scoped** coroutine-local variable.
When called from a DSO (`.so` plugin), the linker resolves `bql_locked` to the exported
symbol, but the `get_bql_locked()` accessor inside the DSO reads the DSO's own copy of the
TLS slot — which is always `false`. The DSO can never observe the real BQL state.

### Fix

Add thin wrapper functions **inside the main QEMU binary** (`system/cpus.c`) that call the
real `bql_locked()` from within the correct translation unit:

```c
// system/cpus.c
bool virtmcu_is_bql_locked(void) { return bql_locked(); }
void virtmcu_safe_bql_unlock(void) { if (bql_locked()) bql_unlock(); }
void virtmcu_safe_bql_lock(void)   { if (!bql_locked()) bql_lock(); }
```

Declare them in `include/qemu/main-loop.h` and use them in the plugin:

```c
// zenoh-clock.c — correct conditional sandwich
bool locked = virtmcu_bql_locked();   // calls virtmcu_is_bql_locked() via ffi
if (locked) { virtmcu_bql_unlock(); }
int64_t delta = zenoh_clock_quantum_wait(rust_state, now);
if (locked) { virtmcu_bql_lock(); }
```

**Lesson:** Never call macros or functions that rely on file-static or coroutine-local TLS
from a DSO. The linker will resolve the symbol but the accessor reads the wrong slot.
When you need real internal QEMU state from a plugin, inject a wrapper function into the
main binary and export it from a proper header.

---

## Bug 2 — Double BQL Lock in Timer Callback (Second Deadlock)

### Symptom

After fixing Bug 1, QEMU deadlocked again, but now always on the *first* timer expiry.
The `virtmcu_cpu_exit_all()` call inside the timer callback never returned.

### Root Cause

QEMU fires `QEMUTimer` callbacks from the **main loop** (IO thread), which holds the BQL.
The original timer callback was:

```c
// WRONG
static void zenoh_clock_timer_cb(void *opaque)
{
    bql_lock();           // BQL is ALREADY held — deadlock
    virtmcu_cpu_exit_all();
    bql_unlock();
}
```

`bql_lock()` on a non-recursive mutex when BQL is already held → deadlock.

### Fix

Remove the lock/unlock entirely. The main-loop contract guarantees BQL is held for timer
callbacks:

```c
// CORRECT
static void zenoh_clock_timer_cb(void *opaque)
{
    // Fires from the QEMU main loop with BQL already held.
    virtmcu_cpu_exit_all();
}
```

**Lesson:** Know which thread context each callback fires in before touching any lock.
QEMU has at least three distinct contexts: (1) main loop / IO thread (BQL held),
(2) vCPU threads inside `cpu_exec_loop` (BQL **not** held), (3) vCPU threads outside the
exec loop (BQL held). Always document which context a function expects in a comment.

---

## Bug 3 — Stale `quantum_done` Flag (Missed Wake-up)

### Symptom

After fixing Bugs 1 and 2, the first query succeeded but the second query returned
immediately with a **stale vtime** — the same timestamp as the first reply. The vCPU never
ran between the two queries.

### Root Cause: Protocol State Machine Bug

The Zenoh clock uses a three-flag protocol between the Zenoh callback thread and the vCPU
thread:

```
quantum_ready — set by Zenoh callback to tell vCPU a new delta arrived
quantum_done  — set by vCPU to tell Zenoh callback it has reached the boundary
delta_ns      — written by Zenoh callback, read by vCPU after quantum_ready
vtime_ns      — written by vCPU, read by Zenoh callback after quantum_done
```

The state machine in `on_clock_query` (Rust) had a subtle reset bug:

```rust
// BEFORE FIX
let already_done = backend.quantum_done.load(Ordering::Acquire);
backend.quantum_done.store(false, Ordering::Release);   // (A) cleared immediately
backend.quantum_ready.store(true, Ordering::Release);
virtmcu_cond_signal(backend.vcpu_cond);

if !already_done {
    while !backend.quantum_done.load(Ordering::Acquire) {
        virtmcu_cond_timedwait(...);
    }
    // quantum_done is now true — but it was NEVER reset here
}
// (B) quantum_done is still true at this point
```

On the **next** query, `already_done` reads `true` at `(A)` because `quantum_done` was left
set from the previous iteration. The callback skipped the wait, read a stale `vtime_ns`, and
replied immediately without ever letting the vCPU run.

### Fix

Reset `quantum_done` after the wait loop exits, once the signal has been consumed:

```rust
if !already_done {
    while !backend.quantum_done.load(Ordering::Acquire) {
        let rc = virtmcu_cond_timedwait(...);
        if rc == 0 && !backend.quantum_done.load(Ordering::Acquire) {
            error_code = 1; // STALL
            break;
        }
    }
    // Consume the signal — reset so the next query starts clean.
    backend.quantum_done.store(false, Ordering::Release);
}
```

**Lesson:** In producer-consumer flag protocols, every "signal" flag must be reset by the
**consumer**, not the producer. The producer sets the flag; the consumer reads it and clears
it. If the producer clears it eagerly (before the consumer can act), a race exists. If the
consumer never clears it, a stale read corrupts the next cycle.

Draw out every state transition before coding multi-flag protocols. Verify: "What is the
value of every flag at the start of the next cycle?"

---

## Bug 4 — CI Build Failure: Missing BQL Helper Declarations

### Symptom

Local builds and tests passed. CI failed at compile step with:

```
error: implicit declaration of function 'virtmcu_is_bql_locked'
error: implicit declaration of function 'virtmcu_safe_bql_lock'
error: implicit declaration of function 'virtmcu_safe_bql_unlock'
```

### Root Cause

The BQL helper functions added to `system/cpus.c` and declared in `main-loop.h` were applied
manually to the local QEMU tree during debugging. They were never integrated into the patch
mechanism (`apply_zenoh_hook.py`) that runs during `make setup` on a fresh clone.

The local QEMU tree (`third_party/qemu/`) is not tracked by git. CI clones a fresh QEMU
source and runs `setup-qemu.sh` → `apply_zenoh_hook.py`. The manually-applied local edits
were invisible to CI.

A partially-correct patch file (`patches/virtmcu_bql_helper.patch`) existed but was:
- Untracked (not committed)
- Not referenced by `setup-qemu.sh`
- Incomplete (missing the `main-loop.h` declarations)

### Fix

Add the injections to `apply_zenoh_hook.py` using the existing `patch_file()` mechanism:

```python
# Inject implementations into system/cpus.c
cpus_c = os.path.join(qemu, "system", "cpus.c")
patch_file(
    cpus_c,
    "void bql_unlock(void)\n{\n    g_assert(!bql_unlock_blocked);\n    qemu_mutex_unlock(&bql);\n}",
    "\nbool virtmcu_is_bql_locked(void) { return bql_locked(); }\n"
    "void virtmcu_safe_bql_unlock(void) { if (bql_locked()) bql_unlock(); }\n"
    "void virtmcu_safe_bql_lock(void) { if (!bql_locked()) bql_lock(); }\n",
    after=True,
)

# Inject declarations into include/qemu/main-loop.h
main_loop_h = os.path.join(qemu, "include", "qemu", "main-loop.h")
patch_file(
    main_loop_h,
    "bool bql_locked(void);",
    "\nbool virtmcu_is_bql_locked(void);\n"
    "void virtmcu_safe_bql_unlock(void);\n"
    "void virtmcu_safe_bql_lock(void);\n",
    after=True,
)
```

**Lesson:** Any modification to files not tracked by the project's git repo must go through
the automated patch/code-generation mechanism — never left as a manual local edit. When
debugging forces you to edit untracked files, immediately write the equivalent injection into
the patch script and delete your manual edit. "Works on my machine" failures are almost
always caused by this class of drift.

---

## Debugging Methodology: How to Attack Multi-Bug Hangs

This incident involved four bugs that masked each other. The approach that worked:

### 1. Establish a Minimal Reproducer First

Before reading any code, get the shortest possible command that reproduces the hang:

```bash
# Start router
python3 tests/zenoh_router_persistent.py &

# Start QEMU with just the clock device
scripts/run.sh --dtb /tmp/dummy.dtb -kernel /tmp/firmware.elf \
  -device zenoh-clock,mode=suspend,node=0,router=tcp/127.0.0.1:7447 \
  -nographic -monitor none &

# Single query
python3 -c "
import zenoh, struct
c = zenoh.Config()
c.insert_json5('connect/endpoints', '[\"tcp/127.0.0.1:7447\"]')
s = zenoh.open(c)
r = list(s.get('sim/clock/advance/0', payload=struct.pack('<QQ', 1000000, 0), timeout=5.0))
print(r)
"
```

A hang with no output is a different bug class from a hang with partial output.

### 2. Kill Stale Processes Before Every Test Run

When testing a system that registers Zenoh queryables, leftover processes from failed runs
will intercept queries intended for the new process. Always run:

```bash
bash scripts/cleanup-sim.sh --quiet
# or
pkill -f "qemu-system\|zenoh_router" 2>/dev/null || true
```

This burned hours before becoming habitual.

### 3. Use `strace` / `gdb` to Identify the Blocking Syscall

When a process hangs:

```bash
# Find which syscall is blocking
strace -p <qemu-pid> -e trace=futex 2>&1 | head -20
```

A `futex(FUTEX_WAIT_PRIVATE, ...)` on a mutex address tells you exactly which lock is
contended. Cross-reference the address with `gdb`:

```bash
gdb -p <qemu-pid>
(gdb) info threads
(gdb) thread apply all bt
```

Look for two threads both trying to acquire the same mutex — that confirms a deadlock and
immediately narrows the search to lock/unlock pairs.

### 4. Add Eprintln Breadcrumbs Aggressively (Then Remove Them)

In Rust, temporary `eprintln!` statements in async/threaded code are invaluable:

```rust
eprintln!("[zenoh-clock] on_clock_query START node={} delta={} already_done={}",
    backend.node_id, delta, already_done);
// ... critical section ...
eprintln!("[zenoh-clock] on_clock_query END node={} vtime={} error={}",
    backend.node_id, vtime, error_code);
```

The gap between START and END (or its absence) tells you which side of the code is blocking.
Similarly in C:

```c
fprintf(stderr, "[zenoh-clock] entering quantum_wait now=%lld\n", (long long)now);
```

Remove or gate these behind a `VIRTMCU_DEBUG` env check before merging.

### 5. Map All Lock Acquisition Points Before Touching Lock Code

Draw a table before writing or modifying any code that touches a shared lock:

| Function | Caller | BQL state on entry |
|----------|--------|--------------------|
| `zenoh_clock_timer_cb` | QEMU main loop | **Held** |
| `zenoh_clock_cpu_halt_cb` (via halt hook) | vCPU, WFI path | **Held** |
| `zenoh_clock_tcg_quantum_cb` (via TCG hook) | vCPU, exec loop | **Not held** |
| `zenoh_clock_quantum_wait` | vCPU | Depends on caller |

Without this table it is easy to assume a uniform BQL state and introduce a double-lock or
a missed unlock.

### 6. Reason About Flag Protocols Symbolically

For every flag in a multi-flag protocol, write out what value it holds at the **start of
each iteration**, not just at the point where you set or clear it:

```
Cycle N start:
  quantum_ready = false  (reset by quantum_wait on exit)
  quantum_done  = false  (reset by on_clock_query after consuming it)

During Cycle N:
  on_clock_query: reads already_done=false, sets quantum_ready=true, signals vCPU
  quantum_wait:   wakes, sets quantum_done=true, signals query_cond
  on_clock_query: reads quantum_done=true, exits loop, RESETS quantum_done=false

Cycle N+1 start:
  quantum_ready = false  ✓
  quantum_done  = false  ✓  (was reset — no stale read)
```

If the reset is missing, write "quantum_done = true" at the start of Cycle N+1 and trace
what happens. This mechanical check found Bug 3 in under five minutes once applied.

### 7. Verify CI/Local Parity Before Declaring Victory

A test passing locally means nothing if the local environment has files that CI never sees.
After any debugging session that touched untracked files, run:

```bash
git status --short
git diff --stat
```

Any modified file under `third_party/` or any untracked file that was part of the fix must
either be committed or folded into the patch mechanism before the fix is real.

The rule: **if CI cannot reproduce your local setup from `git clone` + `make setup`, your
fix is not done.**

---

## Timeline

| Step | What happened |
|------|---------------|
| Phase 18.9 start | Gemini CLI migrates zenoh-clock to Rust; integration tests hang |
| Debugging round 1 | Identify Bug 1 (TLS/BQL). Fix conditional sandwich. |
| Debugging round 2 | Identify Bug 2 (timer callback double-lock). Fix timer callback. |
| Debugging round 3 | Stale process interference masks progress; cleanup script added to workflow. |
| Debugging round 4 | Identify Bug 3 (stale quantum_done). Fix Rust flag reset. |
| Phase 7 smoke test passes | Both suspend and icount modes verified. |
| CI push | CI fails — Bug 4 (manual edits not in patch mechanism). |
| Fix Bug 4 | Inject BQL helpers via `apply_zenoh_hook.py`. CI green. |

Total calendar time: ~2 sessions across separate context windows.

---

## Checklist for Future zenoh-clock or Hook Work

- [ ] Document BQL state at every hook call site in a table comment.
- [ ] Never call QEMU macros that use file-static TLS from a DSO — use main-binary wrappers.
- [ ] Timer callbacks: remove any lock/unlock; the main loop already holds BQL.
- [ ] Multi-flag protocols: write out flag values at the *start* of each cycle, not just at set/clear points.
- [ ] After any debugging session: `git status` — no untracked fix artifacts.
- [ ] Kill stale QEMU/router processes before every integration test run.
- [ ] Run `apply_zenoh_hook.py` twice and verify the second run produces no output (idempotency check).
