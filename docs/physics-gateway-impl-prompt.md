# Implementation Prompt: virtmcu Physics Gateway Refactor

You are implementing a three-phase refactor of the virtmcu physics bridge. This document
is self-contained — read it fully before writing any code.

---

## Project Context

`virtmcu` is a deterministic multi-node firmware simulation framework. The working
directory is `/workspace`. You will touch three crates:

- `tools/cyber_bridge` — the Physical Node binary and physics bridge (Rust)
- `hw/rust/common/virtmcu-api` — shared protocol types, FlatBuffers schemas, transport
  traits (Rust, `no_std`-compatible)
- `tools/cyber_bridge/scripts/mock_physics.py` — Python mock physics engine (test only)

**Mandatory pre-flight for every code change:**
1. Does it use Dependency Injection (no hardcoded singletons)?
2. Does it crash loudly on invariant violations (`assert!`, `expect`, not `warn`)?
3. Which `make test-check` target proves it works?

Run `make test-check` after completing each phase before proceeding to the next.

**Lint gate:** `[workspace.lints.clippy] all = "deny"`. Every clippy warning is a build
failure. `#[allow(clippy::...)]` in production code is banned.

---

## Phase 1 — Immediate Bug Fixes in `tools/cyber_bridge`

### Fix 1A — TOCTOU in `ZenohActuatorSink::drain()`

**File:** `tools/cyber_bridge/src/lib.rs`

Find the `drain` method (currently around line 94). Replace:

```rust
pub fn drain(&self) -> std::collections::HashMap<u32, Vec<f64>> {
    let mut map = self.buffer.lock().unwrap();
    let current = map.clone();
    map.clear();
    current
}
```

With:

```rust
pub fn drain(&self) -> std::collections::HashMap<u32, Vec<f64>> {
    let mut map = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *map)
}
```

`std::mem::take` is atomic under the lock — no clone/clear race and no panic on a
poisoned mutex.

---

### Fix 1B — SHM RAII and Final Layout

**File:** `tools/cyber_bridge/src/physics.rs`

#### 1B-i: Define the canonical SHM header as named constants

Add these constants at the top of `physics.rs` (below the `use` statements). These are
the only authoritative source for SHM offsets. Do not use magic numbers anywhere else in
this file.

```rust
/// Final SHM header layout (24 bytes, all little-endian):
///
/// [0..4]   n_sensors:   u32  — number of sensor f64 slots
/// [4..8]   n_actuators: u32  — number of actuator/ctrl f64 slots
/// [8..12]  bridge_seq:  u32  — incremented by gateway to wake physics engine
/// [12..16] physics_seq: u32  — incremented by physics engine to wake gateway
/// [16..20] shutdown:    u32  — set to 1 by gateway to request clean exit
/// [20..24] reserved:    u32  — must be zero; reserved for future use
/// [24..]   data:              — n_sensors f64s, then n_actuators f64s
pub const SHM_OFF_N_SENSORS:   usize = 0;
pub const SHM_OFF_N_ACTUATORS: usize = 4;
pub const SHM_OFF_BRIDGE_SEQ:  usize = 8;
pub const SHM_OFF_PHYSICS_SEQ: usize = 12;
pub const SHM_OFF_SHUTDOWN:    usize = 16;
pub const SHM_OFF_RESERVED:    usize = 20;
pub const SHM_DATA_OFFSET:     usize = 24;
pub const SHM_HEADER_SIZE:     usize = 24;
```

The layout is designed so that `SHM_DATA_OFFSET` stays at 24 even after the counter
types change from u64 to u32 and the shutdown field is added. All existing hardcoded
`24` constants in Rust survive unchanged.

#### 1B-ii: Update `SharedMemPhysics` struct

Replace the struct definition with:

```rust
pub struct SharedMemPhysics {
    mmap: MmapMut,
    shm_path: std::path::PathBuf,
    node_id: u32,
    n_sensors: u32,
    n_actuators: u32,
    session: Arc<zenoh::Session>,
    topic_prefix: String,
    timeout_ms: u64,
    bridge_seq: u32,
}
```

Changes: `bridge_seq` is now `u32` (was `u64`); `shm_path: PathBuf` is added for RAII.

#### 1B-iii: Update `SharedMemPhysics::new()`

Replace the header-write block and the struct fields that reference AtomicU64. The new
header writes use `u32` for sequences and zeros the shutdown/reserved fields. Replace the
body of `new()` with:

```rust
pub fn new(
    node_id: u32,
    n_sensors: u32,
    n_actuators: u32,
    session: Arc<zenoh::Session>,
    topic_prefix: String,
    timeout_ms: u64,
) -> Result<Self> {
    let shm_path = std::path::PathBuf::from(
        format!("/dev/shm/virtmcu_physics_{node_id}")
    );
    let size = SHM_HEADER_SIZE + (n_sensors + n_actuators) as usize * 8;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&shm_path)?;

    file.set_len(size as u64)?;
    let mut mmap = unsafe { MmapMut::map_mut(&file)? };

    // Write header — all fields little-endian u32
    mmap[SHM_OFF_N_SENSORS  ..SHM_OFF_N_SENSORS   + 4].copy_from_slice(&n_sensors.to_le_bytes());
    mmap[SHM_OFF_N_ACTUATORS..SHM_OFF_N_ACTUATORS  + 4].copy_from_slice(&n_actuators.to_le_bytes());
    mmap[SHM_OFF_BRIDGE_SEQ ..SHM_OFF_BRIDGE_SEQ   + 4].copy_from_slice(&0u32.to_le_bytes());
    mmap[SHM_OFF_PHYSICS_SEQ..SHM_OFF_PHYSICS_SEQ  + 4].copy_from_slice(&0u32.to_le_bytes());
    mmap[SHM_OFF_SHUTDOWN   ..SHM_OFF_SHUTDOWN      + 4].copy_from_slice(&0u32.to_le_bytes());
    mmap[SHM_OFF_RESERVED   ..SHM_OFF_RESERVED      + 4].copy_from_slice(&0u32.to_le_bytes());

    Ok(Self {
        mmap,
        shm_path,
        node_id,
        n_sensors,
        n_actuators,
        session,
        topic_prefix,
        timeout_ms,
        bridge_seq: 0,
    })
}
```

Note: the file is now named `virtmcu_physics_{node_id}` (was `virtmcu_mujoco_{node_id}`).
Update the Python mock and any test fixtures that reference the old name.

#### 1B-iv: Implement `Drop` for `SharedMemPhysics`

Add immediately after the `impl SharedMemPhysics` block:

```rust
impl Drop for SharedMemPhysics {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.shm_path) {
            // Log but do not panic — Drop must not unwind
            eprintln!("SharedMemPhysics: failed to remove {}: {e}", self.shm_path.display());
        }
    }
}
```

#### 1B-v: Replace spin-polling with futex in `SharedMemPhysics::step()`

Add the following imports at the top of `physics.rs` (they are Linux-specific — the
futex syscall is only available on Linux, which is the only supported platform):

```rust
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU32, Ordering};
```

Replace the existing `step()` implementation. The key changes:
- Counters use `u32` casts / `AtomicU32`
- `FUTEX_WAKE` after writing actuators signals the physics engine
- `FUTEX_WAIT` replaces the sleep loop, with `EAGAIN` and `EINTR` handled as retries

```rust
impl PhysicsStep for SharedMemPhysics {
    fn step(
        &mut self,
        _delta_ns: u64,
        actuators: &HashMap<u32, Vec<f64>>,
        resp: &ClockReadyResp,
    ) -> Result<()> {
        let ctrl_offset = SHM_DATA_OFFSET + (self.n_sensors as usize) * 8;

        // 1. Write actuator (ctrl) values to SHM
        for i in 0..self.n_actuators {
            let val = actuators
                .get(&i)
                .and_then(|v| v.first())
                .copied()
                .unwrap_or(0.0);
            let offset = ctrl_offset + (i as usize) * 8;
            self.mmap[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
        }

        // 2. Increment bridge_seq and wake the physics engine via futex
        self.bridge_seq = self.bridge_seq.wrapping_add(1);
        let bridge_ptr = self.mmap.as_ptr().wrapping_add(SHM_OFF_BRIDGE_SEQ)
            as *const AtomicU32;
        unsafe { (*bridge_ptr).store(self.bridge_seq, Ordering::Release) };

        #[cfg(target_os = "linux")]
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                bridge_ptr,
                libc::FUTEX_WAKE,
                1i32,        // wake at most 1 waiter
                std::ptr::null::<libc::timespec>(),
                std::ptr::null::<u32>(),
                0i32,
            );
        }

        // 3. Wait for physics engine to increment physics_seq via futex
        let physics_ptr = self.mmap.as_ptr().wrapping_add(SHM_OFF_PHYSICS_SEQ)
            as *const AtomicU32;
        let expected = self.bridge_seq;
        let start = std::time::Instant::now();

        loop {
            let current = unsafe { (*physics_ptr).load(Ordering::Acquire) };
            if current == expected {
                break;
            }
            if start.elapsed().as_millis() > self.timeout_ms as u128 {
                return Err(anyhow::anyhow!(
                    "Physics engine timeout at vtime {}ns",
                    resp.current_vtime_ns()
                ));
            }

            #[cfg(target_os = "linux")]
            {
                let ts = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 10_000_000, // 10 ms kernel timeout
                };
                let ret = unsafe {
                    libc::syscall(
                        libc::SYS_futex,
                        physics_ptr,
                        libc::FUTEX_WAIT,
                        current,   // only sleep if *ptr still == current
                        &ts as *const libc::timespec,
                        std::ptr::null::<u32>(),
                        0i32,
                    )
                };
                if ret == -1 {
                    let err = unsafe { *libc::__errno_location() };
                    match err {
                        libc::EAGAIN | libc::EINTR => continue, // value changed or signal, retry
                        libc::ETIMEDOUT => continue,            // kernel timeout, check wall-clock
                        _ => return Err(anyhow::anyhow!("futex WAIT error: errno {err}")),
                    }
                }
            }
            #[cfg(not(target_os = "linux"))]
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // 4. Read sensor values from SHM and publish to Zenoh
        for i in 0..self.n_sensors {
            let offset = SHM_DATA_OFFSET + (i as usize) * 8;
            let val_bytes = &self.mmap[offset..offset + 8];
            let topic = format!(
                "{}/{}/sensordata_{}",
                self.topic_prefix, self.node_id, i
            );
            let payload = virtmcu_api::encode_frame(resp.current_vtime_ns(), 0, val_bytes);
            self.session
                .put(&topic, payload)
                .wait()
                .map_err(|e| anyhow::anyhow!("Zenoh publish failed: {e}"))?;
        }

        Ok(())
    }
}
```

Add `libc` to `tools/cyber_bridge/Cargo.toml`:

```toml
libc = "0.2"
```

#### 1B-vi: Update `mock_physics.py`

**File:** `tools/cyber_bridge/scripts/mock_physics.py`

Update:
1. The SHM file name from `virtmcu_mujoco_0` to `virtmcu_physics_0`.
2. The sequence counter unpacking from `<Q` (u64) to `<I` (u32).
3. The sequence counter packing similarly.
4. Add a shutdown flag check.

The header layout the Python script must use:

```python
# SHM header layout (all u32, little-endian):
# [0:4]   n_sensors
# [4:8]   n_actuators
# [8:12]  bridge_seq   (gateway → physics)
# [12:16] physics_seq  (physics → gateway)
# [16:20] shutdown     (1 = exit)
# [20:24] reserved
# [24..]  sensor f64s, then actuator f64s
SHM_NAME        = "/dev/shm/virtmcu_physics_0"
OFF_N_SENSORS   = 0
OFF_N_ACTUATORS = 4
OFF_BRIDGE_SEQ  = 8
OFF_PHYSICS_SEQ = 12
OFF_SHUTDOWN    = 16
SHM_DATA_OFFSET = 24
```

The main loop must check the shutdown flag before each step:

```python
while True:
    bridge_seq = struct.unpack_from("<I", mm, OFF_BRIDGE_SEQ)[0]  # u32
    if bridge_seq != physics_seq:
        # Check shutdown before doing any work
        shutdown = struct.unpack_from("<I", mm, OFF_SHUTDOWN)[0]
        if shutdown:
            break
        # ... physics step ...
        physics_seq = bridge_seq
        struct.pack_into("<I", mm, OFF_PHYSICS_SEQ, physics_seq)   # u32
    else:
        time.sleep(0.001)
```

---

## Phase 2 — FlatBuffers Schema and Transport Trait

### Step 2A — Add `physics.fbs`

**Create file:** `hw/rust/common/virtmcu-api/src/physics.fbs`

```fbs
namespace virtmcu.physics;

/// A single actuator command from one firmware node for one quantum.
/// Carried inside PhysicsTrigger; not used on the Zenoh actuator bus directly.
table ActuatorSample {
    /// Virtual time (ns) at which the firmware issued this command.
    delivery_vtime_ns: uint64;
    /// Actuator index as defined in the board topology.
    actuator_id:       uint32;
    /// Actuator values (length = data_size declared in board YAML).
    values:            [float64];
}

/// Sent by the Time Authority to the Physics Gateway once per quantum.
/// Contains the complete, causally-ordered set of actuator commands for
/// the completed quantum.  The gateway MUST NOT step the physics engine
/// until it receives this message.
table PhysicsTrigger {
    /// Monotonically increasing quantum counter (matches ClockAdvanceReq.quantum_number).
    quantum_number:       uint64;
    /// Virtual time (ns) at the END of the completed quantum
    /// (= absolute_vtime_ns + delta_ns from ClockAdvanceReq).
    quantum_end_vtime_ns: uint64;
    /// All actuator commands collected during this quantum, ordered by
    /// (delivery_vtime_ns, actuator_id).
    actuators:            [ActuatorSample];
}

/// Sent by the Physics Gateway back to the Time Authority after each step.
struct PhysicsDone {
    /// Must match the quantum_number in the corresponding PhysicsTrigger.
    quantum_number: uint64;
    /// 0 = OK, 1 = physics engine error (simulation will abort).
    status:         uint32;
    /// Reserved — must be zero.
    reserved:       uint32;
}

root_type PhysicsTrigger;
```

**Generate the Rust bindings.** Run from the repo root:

```bash
cd hw/rust/common/virtmcu-api/src
flatc --rust --gen-all --filename-suffix _generated physics.fbs
```

This creates `physics_generated.rs` in the same directory. Verify it exists, then add
the standard allow-block wrapper to `virtmcu-api/src/lib.rs`:

```rust
#[allow( // virtmcu-allow: allow reasoning="FlatBuffers-generated module"
    clippy::all,
    missing_docs,
    clippy::unwrap_used,
    clippy::missing_safety_doc,
    clippy::undocumented_unsafe_blocks,
    clippy::extra_unused_lifetimes
)]
pub mod physics_generated;
pub use physics_generated::virtmcu::physics as physics_proto;
```

Commit both `physics.fbs` and `physics_generated.rs`.

---

### Step 2B — Add `PhysicsTransport` Trait

**File:** `hw/rust/common/virtmcu-api/src/lib.rs`

Add after the existing `PhysicalNodeTransport` trait (around line 391):

```rust
/// Abstract transport for the Time Authority ↔ Physics Gateway handshake.
///
/// The Time Authority sends a trigger containing all actuator data for the
/// completed quantum and blocks until the gateway responds with a done signal.
/// Implementations: ZenohPhysicsTransport, UnixSocketPhysicsTransport.
pub trait PhysicsGatewayTransport: Send + Sync {
    /// Send the complete actuator bundle for one quantum to the gateway and
    /// block until the gateway signals that the physics step is complete.
    ///
    /// Returns `Err` on transport failure, timeout, or a non-OK status in the
    /// `PhysicsDone` response.  Callers must treat any `Err` as fatal.
    fn trigger_and_wait(
        &self,
        trigger_bytes: &[u8],
        timeout: core::time::Duration,
    ) -> Result<(), alloc::string::String>;
}

/// Server-side counterpart: implemented by the Physics Gateway to receive
/// triggers and send done signals.
pub trait PhysicsGatewayServer: Send + Sync {
    /// Block until a trigger arrives.  Returns the raw FlatBuffers bytes of
    /// the `PhysicsTrigger` table, or `None` on shutdown/transport close.
    fn recv_trigger(
        &self,
        timeout: core::time::Duration,
    ) -> Option<alloc::vec::Vec<u8>>;

    /// Send the done signal back to the Time Authority.
    fn send_done(
        &self,
        done: physics_proto::PhysicsDone,
    ) -> Result<(), alloc::string::String>;
}
```

Note: the transport uses raw `&[u8]` / `Vec<u8>` for the FlatBuffers payload to keep
the trait `no_std`-compatible and decoupled from the flatbuffers builder.

---

### Step 2C — Add Topic Constants

**File:** `hw/rust/common/virtmcu-api/src/topics/mod.rs`

Add to the `sim_topic` module:

```rust
/// Topic on which the Time Authority publishes physics triggers.
pub const PHYSICS_TRIGGER: &str = "sim/physics/trigger";
/// Topic on which the Physics Gateway publishes done signals.
pub const PHYSICS_DONE: &str = "sim/physics/done";
```

---

### Step 2D — Update `ZenohActuatorSink` to Retain `delivery_vtime_ns`

**File:** `tools/cyber_bridge/src/lib.rs`

The sink currently discards the `ZenohFrameHeader` and stores only `HashMap<u32, Vec<f64>>`.
Update it to store `BTreeMap<u64, HashMap<u32, Vec<f64>>>` keyed by `delivery_vtime_ns`.
This is required for Phase 3 where the TA bundles actuators per-quantum into a
`PhysicsTrigger`.

Change the buffer field type:

```rust
pub struct ZenohActuatorSink {
    buffer: Arc<std::sync::Mutex<
        std::collections::BTreeMap<u64, std::collections::HashMap<u32, Vec<f64>>>
    >>,
    _subscriber: zenoh::pubsub::Subscriber<()>,
}
```

In the subscriber callback, parse the header before extracting values:

```rust
.callback(move |sample| {
    let raw = sample.payload().to_bytes();
    if raw.len() < virtmcu_api::ZENOH_FRAME_HEADER_SIZE {
        return;
    }
    let Some((header, data_bytes)) = virtmcu_api::decode_frame(&raw) else {
        return;
    };
    let vtime = header.delivery_vtime_ns();
    let topic = sample.key_expr().as_str();
    let Some(actuator_id_str) = topic.split('/').next_back() else { return };
    let Ok(actuator_id) = actuator_id_str.parse::<u32>() else { return };

    let mut vals = Vec::new();
    for chunk in data_bytes.chunks_exact(8) {
        if let Ok(arr) = chunk.try_into() {
            vals.push(f64::from_le_bytes(arr));
        }
    }
    if vals.is_empty() { return; }

    if let Ok(mut map) = buffer_clone.lock() {
        map.entry(vtime).or_default().insert(actuator_id, vals);
    }
})
```

Update `drain()`:

```rust
pub fn drain(
    &self,
) -> std::collections::BTreeMap<u64, std::collections::HashMap<u32, Vec<f64>>> {
    let mut map = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut *map)
}
```

**Cascade — update all call sites of `drain()`:**

`time_authority.rs:196-200` currently passes the drained `HashMap` directly to
`physics.step()`. After this change the drained value is a `BTreeMap`. Update
`PhysicsStep::step()` in `physics.rs` to accept `&BTreeMap<u64, HashMap<u32, Vec<f64>>>`
and `NoOpPhysics::step()` similarly.

`SharedMemPhysics::step()` must select only the actuators that belong to the current
quantum. Use `resp.current_vtime_ns()` (= quantum end) and `delta_ns` (the quantum
duration) to build the range:

```rust
fn step(
    &mut self,
    delta_ns: u64,
    actuators: &std::collections::BTreeMap<u64, std::collections::HashMap<u32, Vec<f64>>>,
    resp: &ClockReadyResp,
) -> Result<()> {
    let quantum_end   = resp.current_vtime_ns();
    let quantum_start = quantum_end.saturating_sub(delta_ns);

    // For each actuator slot, use the LAST command issued within this quantum.
    // Multiple writes to the same actuator in one quantum: last value wins.
    let mut quantum_actuators: std::collections::HashMap<u32, Vec<f64>> =
        std::collections::HashMap::new();
    for (_vtime, id_map) in actuators.range(quantum_start..quantum_end) {
        for (&id, vals) in id_map {
            quantum_actuators.insert(id, vals.clone());
        }
    }
    // ... rest of step() uses quantum_actuators instead of actuators ...
```

---

## Phase 3 — Decoupled Physics Gateway Binary

### Step 3A — Add `virtmcu-physics-gateway` Binary

**File:** `tools/cyber_bridge/Cargo.toml`

Add:

```toml
[[bin]]
name = "virtmcu-physics-gateway"
path = "src/bin/physics_gateway.rs"
```

Add to `[dependencies]`:

```toml
async-trait = "0.1"
```

### Step 3B — Move SHM Logic into Gateway

Create `tools/cyber_bridge/src/bin/physics_gateway.rs`.

The gateway binary:
1. Opens a `PhysicsGatewayServer` transport (Unix socket or Zenoh, selected by
   `--transport [unix|zenoh]`, defaulting to `unix`).
2. Owns the SHM file (`SharedMemPhysics` moved here from the TA in Phase 3).
3. On each quantum:
   a. Calls `server.recv_trigger(timeout)` — blocks until the TA sends a `PhysicsTrigger`.
   b. Deserializes the FlatBuffers payload.
   c. Writes actuator values to SHM (using `quantum_end_vtime_ns` to select the correct
      subset from the trigger's `actuators` list — last value wins per actuator_id).
   d. Increments `bridge_seq` and calls `FUTEX_WAKE`.
   e. Waits for `physics_seq == bridge_seq` using `FUTEX_WAIT` (same loop as Phase 1).
   f. Reads sensor values from SHM.
   g. Builds and sends `PhysicsDone { quantum_number, status: 0, reserved: 0 }` back
      via `server.send_done(done)`.

CLI shape:

```
virtmcu-physics-gateway
  --transport [unix|zenoh]          # PhysicsGatewayServer implementation
  --connect <endpoint>              # socket path or Zenoh endpoint
  --node-id <u32>                   # identifies the SHM file
  --n-sensors <u32>
  --n-actuators <u32>
  --timeout-ms <u64>
```

### Step 3C — Update Time Authority to Use `PhysicsGatewayTransport`

**File:** `tools/cyber_bridge/src/bin/time_authority.rs`

When `--physics gateway` is selected (a new `PhysicsType::Gateway` variant):
1. Construct a `Box<dyn PhysicsGatewayTransport>` from `--gateway-transport [unix|zenoh]`.
2. In the main loop, after `transport.advance()` returns and actuators are drained,
   serialize the `PhysicsTrigger` FlatBuffers table from the drained
   `BTreeMap<u64, HashMap<u32, Vec<f64>>>`.
3. Call `gateway_transport.trigger_and_wait(&trigger_bytes, timeout)` and await the
   `PhysicsDone` response before issuing the next `ClockAdvanceReq`.

The in-process `PhysicsType::Shm` (using `SharedMemPhysics` directly) stays available
for single-process deployments and is not removed.

### Step 3D — Transport Implementations

**File:** `tools/cyber_bridge/src/lib.rs` (or a new `src/physics_transport.rs`)

Implement `PhysicsGatewayTransport` and `PhysicsGatewayServer` for both transports.
Follow the exact pattern of the existing `ZenohPhysicalNodeTransport` and
`UnixSocketPhysicalNodeTransport` in `virtmcu-api/src/lib.rs`.

For Unix socket: use a length-prefixed framing identical to the existing
`UnixSocketPhysicalNodeTransport` (write 8-byte LE length, then payload bytes).

### Step 3E — Shutdown Sequence

The gateway must implement the project-mandatory teardown sequence (CLAUDE.md §4):

```rust
// In the gateway's main task/thread:
// 1. On SIGINT/SIGTERM: set running = false (AtomicBool, Ordering::Release)
// 2. Write shutdown = 1u32 into SHM at SHM_OFF_SHUTDOWN (little-endian)
// 3. Increment bridge_seq and call FUTEX_WAKE to unblock the physics engine
// 4. The physics engine sees shutdown == 1 and exits WITHOUT computing a step
// 5. Wait for worker threads via drain_cond (Arc<(Mutex<()>, Condvar)>)
//    — no bounded spin loops
// 6. Drop SharedMemPhysics (triggers Drop::drop → std::fs::remove_file)
```

The physics engine (and the Python mock) must check the shutdown flag as the very first
action after waking from `FUTEX_WAIT` / spin-poll, before reading any actuator data.

---

## Co-location Constraints

Document this in a comment in `physics_gateway.rs`:

```
// The Physics Gateway and the physics engine (any implementation: reference,
// MuJoCo, Omniverse, custom) MUST run on the same host because they
// communicate via /dev/shm (Linux tmpfs — not network-transparent).
//
// In Docker Compose: mount /dev/shm as a shared bind volume into both
// containers.  Do NOT use --ipc=host.
//
// The Time Authority and QEMU cyber nodes may run on any host reachable
// by the configured Zenoh router.  Use --gateway-transport unix when the
// TA and gateway share a host (lowest latency); use zenoh for cross-host
// deployments at the cost of one Zenoh round trip per quantum.
```

---

## Verification Gates

After Phase 1:
```bash
make test-check
# Confirm: no /dev/shm/virtmcu_physics_* files left after test run
# Confirm: mock_physics.py runs without struct.error (layout consistency)
```

After Phase 2:
```bash
make test-check
# Confirm: physics_generated.rs was committed alongside physics.fbs
# Confirm: ZenohActuatorSink unit test passes with new BTreeMap return type
```

After Phase 3:
```bash
make test-check
# Start virtmcu-physics-gateway and virtmcu-physical-node,
# run mock_physics.py, confirm physics step completes without timeout
```

Full CI parity before PR:
```bash
make ci-full
```
