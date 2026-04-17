# virtmcu QEMU container image.
#
# Multi-stage build:
#   base      — debian:${DEBIAN_CODENAME}-slim + vscode user, zsh, oh-my-zsh, common tools
#   toolchain — FROM base + build deps, ARM toolchain binary, Python pin, CMake, FlatBuffers
#   builder   — FROM toolchain: clones QEMU, builds zenoh-c, compiles everything
#   devenv    — FROM toolchain + Rust + Node.js + Claude Code + Gemini CLI + pre-built QEMU
#   runtime   — FROM base (lean): only runtime libs + Python + the installed QEMU binaries
#
# All versions are sourced from the VERSIONS file. Use 'make sync-versions' after
# editing VERSIONS; 'make check-versions' (or CI lint) enforces consistency.

ARG QEMU_REPO=https://gitlab.com/qemu-project/qemu.git
ARG QEMU_REF=v11.0.0-rc3
ARG ZENOH_C_REF=1.9.0
ARG CMAKE_VERSION=4.3.1
ARG RUST_VERSION=1.95.0
ARG BINDGEN_VERSION=0.72.1
ARG FLATBUFFERS_VERSION=25.12.19
ARG DEBIAN_CODENAME=trixie
ARG NODE_VERSION=24
ARG PYTHON_VERSION=3.13
ARG ARM_TOOLCHAIN_VERSION=13.3.rel1
ARG TARGETARCH

# ── Stage 0: rust-builder ─────────────────────────────────────────────────────
FROM rust:${RUST_VERSION}-slim AS rust-builder
ARG BINDGEN_VERSION
RUN rustup component add rustfmt \
    && cargo install bindgen-cli --version ${BINDGEN_VERSION}

# ── Stage 1: base ─────────────────────────────────────────────────────────────
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

RUN apt-get update && apt-get install -y --no-install-recommends curl gnupg apt-transport-https \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
       | gpg --dearmor -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] \
       https://cli.github.com/packages stable main" \
       > /etc/apt/sources.list.d/github-cli.list \
    && rm -rf /var/lib/apt/lists/*

RUN apt-get update && apt-get install -y --no-install-recommends \
    git gh wget openssh-client gnupg2 lsb-release bash-completion \
    procps lsof htop net-tools psmisc unzip nano vim-tiny less jq sudo zsh \
    && rm -rf /var/lib/apt/lists/*

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

USER vscode
RUN sh -c "$(curl -fsSL \
    --connect-timeout 30 --max-time 120 \
    https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)" \
    "" --unattended
USER root

COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /usr/bin/

# ── Stage 2: toolchain ────────────────────────────────────────────────────────
FROM base AS toolchain

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential meson ninja-build python3 python3-venv pkg-config \
    libglib2.0-dev libpixman-1-dev flex bison device-tree-compiler \
    libslirp-dev libclang-dev libstdc++6 b4 gdb gdb-multiarch strace \
    lcov gcovr patchelf \
    gcc-riscv64-linux-gnu binutils-riscv64-linux-gnu \
    && rm -rf /var/lib/apt/lists/*

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

ARG PYTHON_VERSION
RUN uv python install ${PYTHON_VERSION}

ARG FLATBUFFERS_VERSION
RUN curl -L -o /tmp/flatc.zip "https://github.com/google/flatbuffers/releases/download/v${FLATBUFFERS_VERSION}/Linux.flatc.binary.clang++-18.zip" \
    && unzip /tmp/flatc.zip -d /usr/local/bin \
    && chmod +x /usr/local/bin/flatc \
    && rm /tmp/flatc.zip

ARG CMAKE_VERSION
RUN uv pip install --system --no-cache-dir cmake==${CMAKE_VERSION} --break-system-packages

# ── Stage: flatcc-builder ───────────────────────────────────────────────────
FROM toolchain AS flatcc-builder
RUN git clone --depth=1 https://github.com/dvidelabs/flatcc.git /build/flatcc \
    && cd /build/flatcc \
    && CFLAGS="-fPIC" ./scripts/build.sh

# ── Stage 4: builder ──────────────────────────────────────────────────────────
FROM toolchain AS builder

ARG QEMU_REPO
ARG QEMU_REF
ARG ZENOH_C_REF
ARG TARGETARCH

COPY --from=flatcc-builder /build/flatcc/bin/flatcc /usr/local/bin/
COPY --from=flatcc-builder /build/flatcc/include/flatcc /usr/local/include/flatcc
COPY --from=flatcc-builder /build/flatcc/lib/libflatcc* /usr/local/lib/

COPY --from=rust-builder /usr/local/rustup /usr/local/rustup
COPY --from=rust-builder /usr/local/cargo /usr/local/cargo
RUN chown -R vscode:vscode /usr/local/rustup /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:${PATH}

ENV ARCH=arm
WORKDIR /build

RUN git clone --depth=1 --branch "${QEMU_REF}" "${QEMU_REPO}" qemu

RUN git clone --depth=1 --branch "${ZENOH_C_REF}" \
    https://github.com/eclipse-zenoh/zenoh-c.git /build/zenoh-c-src \
    && cmake -S /build/zenoh-c-src -B /build/zenoh-c-build \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX=/build/zenoh-c \
    -DZENOHC_BUILD_WITH_SHARED_MEMORY=OFF \
    && cmake --build /build/zenoh-c-build -j"$(nproc)" \
    && cmake --install /build/zenoh-c-build \
    && rm -rf /build/zenoh-c-src /build/zenoh-c-build

COPY patches/ /build/patches/

RUN cd qemu \
    && git config user.email "virtmcu-build@example.com" \
    && git config user.name "virtmcu" \
    && git am --3way /build/patches/arm-generic-fdt-v3.mbx \
    && sed -i 's/mc->minimum_page_bits = 12;/mc->minimum_page_bits = 12;\n\n    \/* virtmcu: allow all SysBus devices via -device; arm-generic-fdt loads devices from DTB at runtime *\/\n    machine_class_allow_dynamic_sysbus_dev(mc, "sys-bus-device");/' hw/arm/arm_generic_fdt.c

RUN cd qemu && python3 /build/patches/apply_zenoh_hook.py .
RUN cd qemu && python3 /build/patches/apply_zenoh_qapi.py .
RUN cd qemu && python3 /build/patches/apply_zenoh_netdev.py .
RUN cd qemu && python3 /build/patches/apply_zenoh_chardev.py .

COPY hw/ /build/virtmcu-hw/
RUN ln -s /build/virtmcu-hw /build/qemu/hw/virtmcu \
    && grep -q "subdir('virtmcu')" /build/qemu/hw/meson.build \
    || printf "\n# virtmcu out-of-tree peripherals\nsubdir('virtmcu')\n" \
    >> /build/qemu/hw/meson.build

RUN cd qemu \
    && mkdir -p build-virtmcu \
    && cd build-virtmcu \
    && ../configure \
    --prefix=/opt/virtmcu \
    --target-list="${ARCH}-softmmu,riscv32-softmmu,riscv64-softmmu" \
    --enable-modules \
    --enable-fdt \
    --enable-plugins \
    --enable-gcov \
    --disable-werror \
    --disable-docs \
    && make -j"$(nproc)" \
    && make install \
    && mkdir -p /opt/virtmcu/lib/qemu/plugins \
    && find contrib/plugins -name "*.so" -exec cp {} /opt/virtmcu/lib/qemu/plugins/ \; 2>/dev/null || true \
    && find /build/zenoh-c -name "libzenohc.so" -exec cp {} /opt/virtmcu/lib/ \; 2>/dev/null || true \
    && find /opt/virtmcu/bin -type f -executable -exec patchelf --set-rpath '$ORIGIN/../lib' {} + \
    && find /opt/virtmcu/lib -name "*.so" -exec patchelf --set-rpath '$ORIGIN/..' {} +

# ── Stage 3: devenv ───────────────────────────────────────────────────────────
FROM toolchain AS devenv

LABEL org.opencontainers.image.source=https://github.com/refractsystems/virtmcu
LABEL org.opencontainers.image.description="virtmcu development environment — toolchain + AI tools"
LABEL org.opencontainers.image.licenses=MIT

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

COPY --from=rust-builder /usr/local/rustup /usr/local/rustup
COPY --from=rust-builder /usr/local/cargo /usr/local/cargo
RUN chown -R vscode:vscode /usr/local/rustup /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:${PATH}

# PRE-CACHE QEMU: Copy pre-built QEMU from builder stage
COPY --from=builder /opt/virtmcu /opt/virtmcu
ENV PATH="/opt/virtmcu/bin:${PATH}"
ENV LD_LIBRARY_PATH="/opt/virtmcu/lib"
ENV QEMU_MODULE_DIR="/opt/virtmcu/lib/qemu"

USER vscode
RUN curl -fsSL https://claude.ai/install.sh | bash
ENV PATH="/home/vscode/.local/bin:${PATH}"
USER root

RUN npm install -g @google/gemini-cli

WORKDIR /workspace
USER vscode

# ── Stage 5: runtime ──────────────────────────────────────────────────────────
FROM base AS runtime

LABEL org.opencontainers.image.source=https://github.com/refractsystems/virtmcu
LABEL org.opencontainers.image.description="virtmcu runtime image — QEMU + Python simulation tools"
LABEL org.opencontainers.image.licenses=MIT

ARG TARGETARCH
ENV ARCH=arm
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 libglib2.0-0t64 libpixman-1-0 libslirp0 libfdt1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /opt/virtmcu /opt/virtmcu

COPY pyproject.toml /tmp/pyproject.toml
WORKDIR /tmp
RUN uv pip install --no-cache --system --break-system-packages -r pyproject.toml

COPY tools/ /app/tools/
WORKDIR /app

ENV PATH="/opt/virtmcu/bin:${PATH}"
ENV LD_LIBRARY_PATH="/opt/virtmcu/lib"
ENV QEMU_MODULE_DIR="/opt/virtmcu/lib/qemu"

CMD ["sh", "-c", "qemu-system-${ARCH} --version && echo 'virtmcu ready — set CMD in your world compose file'"]
