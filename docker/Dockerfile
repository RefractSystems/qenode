# virtmcu QEMU container image.
#
# Multi-stage build:
#   toolchain — ubuntu:24.04 + all build deps; base for builder (no MCR dependency)
#   devenv    — MCR devcontainer base + build tools + AI CLIs (devcontainer only)
#   builder   — FROM toolchain: clones QEMU, builds zenoh-c, compiles everything
#   runtime   — lean: only runtime libs + Python + the installed QEMU binaries
#
# Build args:
#   QEMU_REPO     upstream QEMU git URL
#   QEMU_REF      git ref to check out (tag, branch, or SHA)
#   ZENOH_C_REF   zenoh-c git tag to build (no 'v' prefix, e.g. 1.9.0)
#   CMAKE_VERSION minimum cmake version required
#   ARCH          target architecture (arm | aarch64)

ARG QEMU_REPO=https://gitlab.com/qemu-project/qemu.git
ARG QEMU_REF=v11.0.0-rc3
ARG ZENOH_C_REF=1.9.0
ARG CMAKE_VERSION=4.3.1
ARG RUST_VERSION=1.95.0
ARG FLATBUFFERS_VERSION=25.12.19
ARG TARGETARCH

# ── Stage 0: rust-builder ─────────────────────────────────────────────────────
# Dedicated Rust toolchain stage for multi-stage parity.
FROM rust:${RUST_VERSION}-slim AS rust-builder

# ── Stage 0: toolchain ────────────────────────────────────────────────────────
# Plain ubuntu with all build dependencies. Used directly by the builder stage
# so that CI never has to pull from MCR (which is rate-limited on GHA runners).
FROM ubuntu:24.04 AS toolchain

ENV DEBIAN_FRONTEND=noninteractive

RUN sed -i 's/archive.ubuntu.com/azure.archive.ubuntu.com/g' /etc/apt/sources.list.d/ubuntu.sources || true \
    && sed -i 's/security.ubuntu.com/azure.archive.ubuntu.com/g' /etc/apt/sources.list.d/ubuntu.sources || true

RUN apt-get -y -o Acquire::ForceIPv4=true update && apt-get install -y -o Acquire::ForceIPv4=true --no-install-recommends \
    ca-certificates \
    gpg \
    curl \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | gpg --dearmor -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | tee /etc/apt/sources.list.d/github-cli.list > /dev/null \
    && apt-get -y -o Acquire::ForceIPv4=true update && apt-get install -y -o Acquire::ForceIPv4=true --no-install-recommends -o Acquire::Retries=3 \
    build-essential \
    git \
    gh \
    meson \
    ninja-build \
    python3 \
    python3-venv \
    pkg-config \
    libglib2.0-dev \
    libpixman-1-dev \
    flex \
    bison \
    device-tree-compiler \
    libslirp-dev \
    curl \
    ca-certificates \
    b4 \
    gdb \
    gdb-multiarch \
    gcc-arm-none-eabi \
    gcc-riscv64-linux-gnu \
    binutils-riscv64-linux-gnu \
    jq \
    lcov \
    gcovr \
    patchelf \
    unzip \
    && rm -rf /var/lib/apt/lists/*

# Install FlatBuffers compiler from GitHub release
ARG FLATBUFFERS_VERSION
RUN curl -L -o /tmp/flatc.zip "https://github.com/google/flatbuffers/releases/download/v${FLATBUFFERS_VERSION}/Linux.flatc.binary.clang++-18.zip" \
    && unzip /tmp/flatc.zip -d /usr/local/bin \
    && chmod +x /usr/local/bin/flatc \
    && rm /tmp/flatc.zip

# Install uv for fast Python package management
COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /usr/bin/

# Install specific CMake version
ARG CMAKE_VERSION=4.3.1
RUN uv pip install --system --no-cache-dir cmake==${CMAKE_VERSION} --break-system-packages


# ── Stage: flatcc-builder ───────────────────────────────────────────────────
# Build flatcc once to be shared across stages.
FROM toolchain AS flatcc-builder
# hadolint ignore=DL3003
RUN git clone --depth=1 https://github.com/dvidelabs/flatcc.git /build/flatcc \
    && cd /build/flatcc \
    && CFLAGS="-fPIC" ./scripts/build.sh

# ── Stage 1: devenv ───────────────────────────────────────────────────────────
# Devcontainer image only. MCR base provides the vscode user + sudo setup.
# The workspace is mounted at runtime — nothing from the repo is baked in here.
#
# https://github.com/devcontainers/images/tree/main/src/base-ubuntu
FROM mcr.microsoft.com/devcontainers/base:ubuntu-24.04 AS devenv

LABEL org.opencontainers.image.source=https://github.com/refractsystems/virtmcu
LABEL org.opencontainers.image.description="virtmcu development environment — toolchain + AI tools"
LABEL org.opencontainers.image.licenses=MIT

ENV DEBIAN_FRONTEND=noninteractive

RUN sed -i 's/archive.ubuntu.com/azure.archive.ubuntu.com/g' /etc/apt/sources.list.d/ubuntu.sources || true \
    && sed -i 's/security.ubuntu.com/azure.archive.ubuntu.com/g' /etc/apt/sources.list.d/ubuntu.sources || true

RUN apt-get -y -o Acquire::ForceIPv4=true update && apt-get install -y -o Acquire::ForceIPv4=true --no-install-recommends \
    ca-certificates \
    gpg \
    curl \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | gpg --dearmor -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | tee /etc/apt/sources.list.d/github-cli.list > /dev/null \
    && apt-get -y -o Acquire::ForceIPv4=true update && apt-get install -y -o Acquire::ForceIPv4=true --no-install-recommends -o Acquire::Retries=3 \
    build-essential \
    git \
    gh \
    meson \
    ninja-build \
    python3 \
    python3-venv \
    pkg-config \
    libglib2.0-dev \
    libpixman-1-dev \
    flex \
    bison \
    device-tree-compiler \
    libslirp-dev \
    curl \
    ca-certificates \
    b4 \
    gdb \
    gdb-multiarch \
    gcc-arm-none-eabi \
    gcc-riscv64-linux-gnu \
    binutils-riscv64-linux-gnu \
    strace \
    jq \
    unzip \
    && rm -rf /var/lib/apt/lists/*

# Install FlatBuffers compiler from GitHub release
ARG FLATBUFFERS_VERSION
RUN curl -L -o /tmp/flatc.zip "https://github.com/google/flatbuffers/releases/download/v${FLATBUFFERS_VERSION}/Linux.flatc.binary.clang++-18.zip" \
    && unzip /tmp/flatc.zip -d /usr/local/bin \
    && chmod +x /usr/local/bin/flatc \
    && rm /tmp/flatc.zip

# Install uv for fast Python package management
COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /usr/bin/

# Install specific CMake version
ARG CMAKE_VERSION=4.3.1
RUN uv pip install --system --no-cache-dir cmake==${CMAKE_VERSION} --break-system-packages


COPY --from=flatcc-builder /build/flatcc/bin/flatcc /usr/local/bin/
COPY --from=flatcc-builder /build/flatcc/include/flatcc /usr/local/include/flatcc
COPY --from=flatcc-builder /build/flatcc/lib/libflatcc* /usr/local/lib/

# Copy Rust toolchain from rust-builder
COPY --from=rust-builder /usr/local/rustup /usr/local/rustup
COPY --from=rust-builder /usr/local/cargo /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:${PATH}

# Install Node.js 24 (required for Gemini CLI)
RUN curl -fsSL https://deb.nodesource.com/setup_24.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install Claude Code via native installer (as vscode user so auto-update works)
# npm install -g is deprecated since Feb 2026; native installer puts binary in ~/.local/bin
USER vscode
RUN curl -fsSL https://claude.ai/install.sh | bash
ENV PATH="/home/vscode/.local/bin:${PATH}"
# hadolint ignore=DL3002
USER root

# Install Gemini CLI (requires node/npm, installed above)
# hadolint ignore=DL3016
RUN npm install -g @google/gemini-cli

WORKDIR /workspace

# ── Stage 2: builder ──────────────────────────────────────────────────────────
# FROM toolchain (not devenv) so CI never pulls from MCR.
FROM toolchain AS builder

ARG QEMU_REPO
ARG QEMU_REF
ARG ZENOH_C_REF
ARG TARGETARCH

COPY --from=flatcc-builder /build/flatcc/bin/flatcc /usr/local/bin/
COPY --from=flatcc-builder /build/flatcc/include/flatcc /usr/local/include/flatcc
COPY --from=flatcc-builder /build/flatcc/lib/libflatcc* /usr/local/lib/

# Copy Rust toolchain from rust-builder
COPY --from=rust-builder /usr/local/rustup /usr/local/rustup
COPY --from=rust-builder /usr/local/cargo /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:${PATH}

# We emulate ARM microcontrollers regardless of the host architecture
ENV ARCH=arm

WORKDIR /build

# Clone QEMU before COPYing local files so that patch/hw changes do not bust
# this expensive layer. Cache key: QEMU_REF arg.
# hadolint ignore=DL3003
RUN git clone --depth=1 --branch "${QEMU_REF}" "${QEMU_REPO}" qemu

# Build zenoh-c from source and install to /build/zenoh-c.
# Placed after the QEMU clone (both are network-only) so neither busts the
# other. Cache key: ZENOH_C_REF arg.
# meson.build resolves meson.project_source_root()/../zenoh-c → /build/zenoh-c ✓
RUN git clone --depth=1 --branch "${ZENOH_C_REF}" \
    https://github.com/eclipse-zenoh/zenoh-c.git /build/zenoh-c-src \
    && cmake -S /build/zenoh-c-src -B /build/zenoh-c-build \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX=/build/zenoh-c \
    -DZENOHC_BUILD_WITH_SHARED_MEMORY=OFF \
    && cmake --build /build/zenoh-c-build -j"$(nproc)" \
    && cmake --install /build/zenoh-c-build \
    && rm -rf /build/zenoh-c-src /build/zenoh-c-build

# COPY local files after all network steps so that changes to patches/ or hw/
# only invalidate layers that actually depend on them.
COPY patches/ /build/patches/

# Apply arm-generic-fdt v3 patch series
# hadolint ignore=DL3003
RUN cd qemu \
    && git config user.email "virtmcu-build@example.com" \
    && git config user.name "virtmcu" \
    && git am --3way /build/patches/arm-generic-fdt-v3.mbx \
    && sed -i 's/mc->minimum_page_bits = 12;/mc->minimum_page_bits = 12;\n\n    \/* virtmcu: allow all SysBus devices via -device; arm-generic-fdt loads devices from DTB at runtime *\/\n    machine_class_allow_dynamic_sysbus_dev(mc, "sys-bus-device");/' hw/arm/arm_generic_fdt.c

# Inject virtmcu_tcg_quantum_hook into cpu-exec.c (required by hw/zenoh/)
# hadolint ignore=DL3003
RUN cd qemu && python3 /build/patches/apply_zenoh_hook.py .

# Inject Zenoh netdev/chardev types into QEMU's QAPI schemas (net.json, char.json).
# These generate NET_CLIENT_DRIVER_ZENOH, NetdevZenohOptions, etc. used by hw/zenoh/.
# hadolint ignore=DL3003
RUN cd qemu && python3 /build/patches/apply_zenoh_qapi.py .

# Register backends in net/net.c and chardev/char.c
# hadolint ignore=DL3003
RUN cd qemu && python3 /build/patches/apply_zenoh_netdev.py .
# hadolint ignore=DL3003
RUN cd qemu && python3 /build/patches/apply_zenoh_chardev.py .

# Copy virtmcu hw/ sources and integrate into QEMU's build
COPY hw/ /build/virtmcu-hw/
RUN ln -s /build/virtmcu-hw /build/qemu/hw/virtmcu \
    && grep -q "subdir('virtmcu')" /build/qemu/hw/meson.build \
    || printf "\n# virtmcu out-of-tree peripherals\nsubdir('virtmcu')\n" \
    >> /build/qemu/hw/meson.build

# Configure and build
# hadolint ignore=DL3003,SC2016
RUN cd qemu \
    && ./configure \
    --prefix=/opt/virtmcu \
    --target-list="${ARCH}-softmmu,riscv32-softmmu,riscv64-softmmu" \
    --enable-modules \
    --enable-fdt \
    --enable-gcov \
    --disable-werror \
    --disable-docs \
    && make -j"$(nproc)" \
    && make -j"$(nproc)" plugins \
    && make install \
    && cp /build/zenoh-c/lib/libzenohc.so /opt/virtmcu/lib/ \
    && find /opt/virtmcu/bin -type f -executable -exec patchelf --set-rpath '$ORIGIN/../lib' {} + \
    && find /opt/virtmcu/lib -name "*.so" -exec patchelf --set-rpath '$ORIGIN/..' {} +

# Make the installed QEMU binary and modules available inside this stage
# (used by CI integration jobs that run smoke tests within the builder container).
ENV PATH="/opt/virtmcu/bin:${PATH}"
ENV QEMU_MODULE_DIR="/opt/virtmcu/lib/qemu"
# ── Stage 3: runtime ──────────────────────────────────────────────────────────
# Lean: only runtime libs + Python + the installed QEMU binaries.
# Use toolchain as base to ensure parity with builder environment.
FROM toolchain AS runtime

LABEL org.opencontainers.image.source=https://github.com/refractsystems/virtmcu
LABEL org.opencontainers.image.description="virtmcu runtime image — QEMU + Python simulation tools"
LABEL org.opencontainers.image.licenses=MIT

ARG TARGETARCH
# We emulate ARM microcontrollers regardless of the host architecture
ENV ARCH=arm
ENV DEBIAN_FRONTEND=noninteractive

COPY --from=builder /opt/virtmcu /opt/virtmcu

COPY --from=builder /build/zenoh-c/lib/libzenohc.so /opt/virtmcu/lib/

# Install Python dependencies using uv
COPY pyproject.toml /tmp/pyproject.toml
WORKDIR /tmp
RUN uv pip install --no-cache --system --break-system-packages -r pyproject.toml

# Copy Python tooling
COPY tools/ /app/tools/
WORKDIR /app

# Make qemu-system-* available on PATH
ENV PATH="/opt/virtmcu/bin:${PATH}"
ENV LD_LIBRARY_PATH="/opt/virtmcu/lib"
ENV QEMU_MODULE_DIR="/opt/virtmcu/lib/qemu"

# Default: print version and wait (worlds override CMD)
CMD ["sh", "-c", "qemu-system-${ARCH} --version && echo 'virtmcu ready — set CMD in your world compose file'"]
