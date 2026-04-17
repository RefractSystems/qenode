# virtmcu QEMU container image.
#
# Multi-stage build:
#   base      — debian:${DEBIAN_CODENAME}-slim + vscode user, zsh, oh-my-zsh, common tools
#   toolchain — FROM base + build deps, ARM toolchain binary, Python pin, CMake, FlatBuffers
#   devenv    — FROM toolchain + Rust + Node.js + Claude Code + Gemini CLI (devcontainer only)
#   builder   — FROM toolchain: clones QEMU, builds zenoh-c, compiles everything
#   runtime   — FROM base (lean): only runtime libs + Python + the installed QEMU binaries
#
# All versions are sourced from the VERSIONS file. Use 'make sync-versions' after
# editing VERSIONS; 'make check-versions' (or CI lint) enforces consistency.
#
# Build args (see VERSIONS for canonical values):
#   DEBIAN_CODENAME  Debian release codename (e.g. trixie)
#   QEMU_REPO        upstream QEMU git URL
#   QEMU_REF         git ref to check out (tag, branch, or SHA)
#   ZENOH_C_REF      zenoh-c git tag to build (no 'v' prefix, e.g. 1.9.0)
#   CMAKE_VERSION    CMake version installed via uv
#   ARM_TOOLCHAIN_VERSION  ARM GNU Toolchain version from developer.arm.com
#   PYTHON_VERSION   Python version pinned via uv (independent of OS default)
#   NODE_VERSION     Node.js major version for devenv

ARG QEMU_REPO=https://gitlab.com/qemu-project/qemu.git
ARG QEMU_REF=v11.0.0-rc3
ARG ZENOH_C_REF=1.9.0
ARG CMAKE_VERSION=4.3.1
ARG RUST_VERSION=1.95.0
ARG FLATBUFFERS_VERSION=25.12.19
ARG DEBIAN_CODENAME=trixie
ARG NODE_VERSION=24
ARG PYTHON_VERSION=3.13
ARG ARM_TOOLCHAIN_VERSION=13.3.rel1
ARG TARGETARCH

# ── Stage 0: rust-builder ─────────────────────────────────────────────────────
# Dedicated Rust toolchain stage for multi-stage parity.
FROM rust:${RUST_VERSION}-slim AS rust-builder

# ── Stage 1: base ─────────────────────────────────────────────────────────────
# OS foundation replacing MCR. Layers ordered slowest-to-fastest changing.
ARG DEBIAN_CODENAME
FROM debian:${DEBIAN_CODENAME}-slim AS base

ENV DEBIAN_FRONTEND=noninteractive \
    TERM=xterm-256color

RUN apt-get update && apt-get install -y --no-install-recommends \
    locales tzdata ca-certificates \
    && echo "en_US.UTF-8 UTF-8" > /etc/locale.gen \
    && locale-gen en_US.UTF-8 \
    && update-locale LANG=en_US.UTF-8 \
    && ln -sf /usr/share/zoneinfo/UTC /etc/localtime \
    && rm -rf /var/lib/apt/lists/*

ENV LANG=en_US.UTF-8 \
    LANGUAGE=en_US:en \
    LC_ALL=en_US.UTF-8

# GitHub CLI apt source (independent layer for caching)
RUN apt-get update && apt-get install -y --no-install-recommends curl gnupg apt-transport-https \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
       | gpg --dearmor -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] \
       https://cli.github.com/packages stable main" \
       > /etc/apt/sources.list.d/github-cli.list \
    && rm -rf /var/lib/apt/lists/*

# Common tools (MCR parity list)
RUN apt-get update && apt-get install -y --no-install-recommends \
    git gh wget openssh-client gnupg2 lsb-release bash-completion \
    procps lsof htop net-tools psmisc unzip nano vim-tiny less jq sudo zsh \
    && rm -rf /var/lib/apt/lists/*

# vscode user
ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=1000
RUN groupadd --gid ${USER_GID} ${USERNAME} \
    && useradd \
       --uid ${USER_UID} \
       --gid ${USER_GID} \
       --shell /usr/bin/zsh \
       --create-home \
       ${USERNAME} \
    && echo "${USERNAME} ALL=(ALL) NOPASSWD:ALL" \
       > /etc/sudoers.d/${USERNAME} \
    && chmod 0440 /etc/sudoers.d/${USERNAME}

# oh-my-zsh (UX parity)
USER vscode
RUN sh -c "$(curl -fsSL \
    --connect-timeout 30 --max-time 120 \
    https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)" \
    "" --unattended
# hadolint ignore=DL3002
USER root

# uv binary
COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /usr/bin/

# ── Stage 2: toolchain ────────────────────────────────────────────────────────
# FROM base, zero duplication. Build-specific tooling.
FROM base AS toolchain

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential meson ninja-build python3 python3-venv pkg-config \
    libglib2.0-dev libpixman-1-dev flex bison device-tree-compiler \
    libslirp-dev libstdc++6 b4 gdb gdb-multiarch strace \
    lcov gcovr patchelf \
    gcc-riscv64-linux-gnu binutils-riscv64-linux-gnu \
    && rm -rf /var/lib/apt/lists/*

# ARM GNU Toolchain (binary release, version-pinned)
ARG ARM_TOOLCHAIN_VERSION
ARG TARGETARCH
RUN case "${TARGETARCH}" in \
      amd64) _HOST=x86_64 ;; \
      arm64) _HOST=aarch64 ;; \
      *) echo "Unsupported TARGETARCH: ${TARGETARCH}" && exit 1 ;; \
    esac \
    && _URL="https://developer.arm.com/-/media/Files/downloads/gnu/${ARM_TOOLCHAIN_VERSION}/binrel" \
    && _FILE="arm-gnu-toolchain-${ARM_TOOLCHAIN_VERSION}-${_HOST}-arm-none-eabi" \
    && curl -fsSL "${_URL}/${_FILE}.tar.xz" | tar -xJ -C /opt \
    && ln -sf /opt/arm-gnu-toolchain-*/bin/* /usr/local/bin/

# Python (pinned via uv, independent of OS default)
ARG PYTHON_VERSION
RUN uv python install ${PYTHON_VERSION}

# Install FlatBuffers compiler from GitHub release
ARG FLATBUFFERS_VERSION
RUN curl -L -o /tmp/flatc.zip "https://github.com/google/flatbuffers/releases/download/v${FLATBUFFERS_VERSION}/Linux.flatc.binary.clang++-18.zip" \
    && unzip /tmp/flatc.zip -d /usr/local/bin \
    && chmod +x /usr/local/bin/flatc \
    && rm /tmp/flatc.zip

# Install specific CMake version
ARG CMAKE_VERSION
RUN uv pip install --system --no-cache-dir cmake==${CMAKE_VERSION} --break-system-packages


# ── Stage: flatcc-builder ───────────────────────────────────────────────────
# Build flatcc once to be shared across stages.
FROM toolchain AS flatcc-builder
# hadolint ignore=DL3003
RUN git clone --depth=1 https://github.com/dvidelabs/flatcc.git /build/flatcc \
    && cd /build/flatcc \
    && CFLAGS="-fPIC" ./scripts/build.sh

# ── Stage 3: devenv ───────────────────────────────────────────────────────────
# Devcontainer image. Inherits from toolchain (base + build deps + ARM toolchain).
FROM toolchain AS devenv

LABEL org.opencontainers.image.source=https://github.com/refractsystems/virtmcu
LABEL org.opencontainers.image.description="virtmcu development environment — toolchain + AI tools"
LABEL org.opencontainers.image.licenses=MIT

# Node.js — Direct binary download (NodeSource doesn't support trixie yet)
ARG NODE_VERSION
ARG TARGETARCH
RUN case "${TARGETARCH}" in \
      amd64) _HOST=x64 ;; \
      arm64) _HOST=arm64 ;; \
      *) echo "Unsupported TARGETARCH: ${TARGETARCH}" && exit 1 ;; \
    esac \
    && _URL="https://nodejs.org/dist/latest-v${NODE_VERSION}.x" \
    && _FILE="node-v${NODE_VERSION}.x-linux-${_HOST}" \
    && curl -fsSL "${_URL}/SHASUMS256.txt" | grep "linux-${_HOST}.tar.xz" > /tmp/shasum \
    && _FULLFILE=$(awk '{print $2}' /tmp/shasum) \
    && curl -fsSL "${_URL}/${_FULLFILE}" -o "/tmp/${_FULLFILE}" \
    && echo "$(awk '{print $1}' /tmp/shasum)  /tmp/${_FULLFILE}" | sha256sum -c - \
    && tar -xJ -f "/tmp/${_FULLFILE}" -C /usr/local --strip-components=1 \
    && rm "/tmp/${_FULLFILE}" /tmp/shasum

COPY --from=flatcc-builder /build/flatcc/bin/flatcc /usr/local/bin/
COPY --from=flatcc-builder /build/flatcc/include/flatcc /usr/local/include/flatcc
COPY --from=flatcc-builder /build/flatcc/lib/libflatcc* /usr/local/lib/

# Copy Rust toolchain from rust-builder
COPY --from=rust-builder /usr/local/rustup /usr/local/rustup
COPY --from=rust-builder /usr/local/cargo /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:${PATH}

# Claude Code (as vscode user)
USER vscode
RUN curl -fsSL https://claude.ai/install.sh | bash
ENV PATH="/home/vscode/.local/bin:${PATH}"
USER root

# Gemini CLI (requires node/npm, version-pinned for reproducibility)
ARG GEMINI_CLI_VERSION=0.38.1
RUN npm install -g @google/gemini-cli@${GEMINI_CLI_VERSION}

WORKDIR /workspace
# devenv runs as vscode — the expected interactive user
USER vscode

# ── Stage 4: builder ──────────────────────────────────────────────────────────
# FROM toolchain (not devenv) — CI builds never require the AI tools in devenv.
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
# ── Stage 5: runtime ──────────────────────────────────────────────────────────
# Lean: only runtime libs + Python + the installed QEMU binaries.
# FROM base (not toolchain) — drops ~400 MB of build tooling from the published image.
FROM base AS runtime

LABEL org.opencontainers.image.source=https://github.com/refractsystems/virtmcu
LABEL org.opencontainers.image.description="virtmcu runtime image — QEMU + Python simulation tools"
LABEL org.opencontainers.image.licenses=MIT

ARG TARGETARCH
# We emulate ARM microcontrollers regardless of the host architecture
ENV ARCH=arm
ENV DEBIAN_FRONTEND=noninteractive

# Install runtime libraries for QEMU and Python
RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 libglib2.0-0t64 libpixman-1-0 libslirp0 libfdt1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /opt/virtmcu /opt/virtmcu

COPY --from=builder /build/zenoh-c/lib/libzenohc.so /opt/virtmcu/lib/

# Install Python dependencies using uv (uv is in base)
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
