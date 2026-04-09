# qenode QEMU container image.
#
# Multi-stage build:
#   devenv   — toolchain only: all build deps, no code baked in (used by devcontainer)
#   builder  — heavy: clones QEMU, applies patches, builds (used by CI / docker-compose)
#   runtime  — lean: only runtime libs + Python + the installed QEMU binaries
#
# Build args:
#   QEMU_REPO     upstream QEMU git URL
#   QEMU_REF      git ref to check out (tag, branch, or SHA)
#   ARCH          target architecture (arm | aarch64)

ARG QEMU_REPO=https://gitlab.com/qemu-project/qemu.git
ARG QEMU_REF=v10.2.92
ARG ARCH=arm

# ── Stage 0: devenv ───────────────────────────────────────────────────────────
# Toolchain-only image for devcontainer use. The workspace is mounted at runtime
# by the devcontainer spec — nothing from the repo is baked in here.
#
# Based on the official devcontainer base (ubuntu:24.04 + vscode user + sudo).
# https://github.com/devcontainers/images/tree/main/src/base-ubuntu
FROM mcr.microsoft.com/devcontainers/base:ubuntu-24.04 AS devenv

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        git \
        meson \
        ninja-build \
        python3 \
        python3-pip \
        python3-venv \
        pkg-config \
        libglib2.0-dev \
        libpixman-1-dev \
        flex \
        bison \
        device-tree-compiler \
        libslirp-dev \
        curl \
        wget \
        ca-certificates \
        b4 \
        gdb \
        gdb-multiarch \
        gcc-arm-none-eabi \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js 22 (required for Gemini CLI v flag)
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install Claude Code and Gemini CLI (requires node/npm, installed above)
RUN npm install -g @anthropic-ai/claude-code @google/gemini-cli

WORKDIR /workspace

# ── Stage 1: builder ──────────────────────────────────────────────────────────
FROM devenv AS builder

ARG QEMU_REPO
ARG QEMU_REF
ARG ARCH

WORKDIR /build

# Copy our patch assets before the long QEMU clone so Docker cache is efficient
COPY patches/ /build/patches/

# Clone QEMU at the specified ref
RUN git clone --depth=1 --branch "${QEMU_REF}" "${QEMU_REPO}" qemu \
    && cd qemu \
    && git submodule update --init --recursive --depth=1

# Apply arm-generic-fdt v3 patch series
RUN cd qemu \
    && git config user.email "qenode-build@example.com" \
    && git config user.name "qenode" \
    && git am --3way /build/patches/arm-generic-fdt-v3.mbx

# Apply libqemu clock-socket extension
RUN cd qemu && python3 /build/patches/apply_libqemu.py .

# Copy qenode hw/ sources and integrate into QEMU's build
COPY hw/ /build/qenode-hw/
RUN ln -s /build/qenode-hw /build/qemu/hw/qenode \
    && grep -q "subdir('qenode')" /build/qemu/hw/meson.build \
       || printf "\n# qenode out-of-tree peripherals\nsubdir('qenode')\n" \
          >> /build/qemu/hw/meson.build

# Configure and build
RUN cd qemu \
    && ./configure \
        --prefix=/opt/qenode \
        --target-list="${ARCH}-softmmu" \
        --enable-modules \
        --enable-fdt \
        --disable-werror \
        --disable-docs \
    && make -j"$(nproc)" \
    && make install

# ── Stage 2: runtime ─────────────────────────────────────────────────────────
FROM ubuntu:24.04 AS runtime

ARG ARCH
ENV ARCH=${ARCH}
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
        python3 \
        python3-pip \
        libglib2.0-0 \
        libpixman-1-0 \
        libslirp0 \
        iproute2 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /opt/qenode /opt/qenode

# Install Python dependencies
COPY requirements.txt /tmp/requirements.txt
RUN pip3 install --no-cache-dir -r /tmp/requirements.txt

# Copy Python tooling
COPY tools/ /app/tools/
WORKDIR /app

# Make qemu-system-* available on PATH
ENV PATH="/opt/qenode/bin:${PATH}"
ENV QEMU_MODULE_DIR="/opt/qenode/lib/qemu"

# Default: print version and wait (worlds override CMD)
CMD ["sh", "-c", "qemu-system-${ARCH} --version && echo 'qenode ready — set CMD in your world compose file'"]
