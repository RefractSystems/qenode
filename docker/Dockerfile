# virtmcu QEMU container image.
#
# Multi-stage build:
#   base      — debian:${DEBIAN_CODENAME}-slim + vscode user, zsh, oh-my-zsh, common tools
#   toolchain — FROM base + build deps, ARM toolchain, Python pin, CMake, FlatBuffers (lean)
#   sim-tools — FROM toolchain + flatcc + zenoh-c (simulation specific)
#   builder   — FROM sim-tools: clones QEMU, compiles everything
#   devenv    — FROM toolchain + Rust + Node.js + AI tools + pre-built QEMU
#   runtime   — FROM base (lean): only runtime libs + Python + the installed QEMU binaries

ARG QEMU_REPO=https://gitlab.com/qemu-project/qemu.git
ARG QEMU_REF=v11.0.0
ARG ZENOH_C_REF=1.9.0
ARG CMAKE_VERSION=4.3.1
ARG RUST_VERSION=1.95.0
ARG BINDGEN_VERSION=0.72.1
ARG FLATBUFFERS_VERSION=25.12.19
ARG FLATCC_VERSION=0.6.1
ARG HADOLINT_VERSION=2.14.0
ARG ACTIONLINT_VERSION=1.7.12
ARG DEBIAN_CODENAME=trixie
ARG NODE_VERSION=24
ARG PYTHON_VERSION=3.13
ARG ARM_TOOLCHAIN_VERSION=13.3.rel1
ARG TARGETARCH

# ── Stage 0: rust-builder ─────────────────────────────────────────────────────
FROM public.ecr.aws/docker/library/rust:${RUST_VERSION}-slim AS rust-builder
ARG BINDGEN_VERSION
ENV CARGO_HOME=/usr/local/cargo
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
# hadolint ignore=DL3008
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
    && curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash \
    && rustup toolchain install nightly --profile minimal --component miri \
    && rustup component add rustfmt clippy rust-analyzer \
    && cargo binstall -y bindgen-cli@${BINDGEN_VERSION} \
    && cargo binstall -y cargo-audit \
    && cargo binstall -y cargo-deny \
    && cargo binstall -y cargo-machete \
    && rm -rf /var/lib/apt/lists/* \
    && rm -rf /usr/local/rustup/toolchains/*1.93.0*

# ── Stage 1: base ─────────────────────────────────────────────────────────────
ARG DEBIAN_CODENAME
FROM public.ecr.aws/debian/debian:${DEBIAN_CODENAME}-slim AS base

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

ENV DEBIAN_FRONTEND=noninteractive \
    TERM=xterm-256color

# hadolint ignore=DL3008
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

# hadolint ignore=DL3008
RUN apt-get update && apt-get install -y --no-install-recommends curl gpg gnupg apt-transport-https \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
       | gpg --dearmor -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] \
       https://cli.github.com/packages stable main" \
       > /etc/apt/sources.list.d/github-cli.list \
    && rm -rf /var/lib/apt/lists/*

# hadolint ignore=DL3008
RUN apt-get update && apt-get install -y --no-install-recommends \
    git gh wget openssh-client gnupg2 lsb-release bash-completion \
    procps lsof htop net-tools psmisc unzip nano vim-tiny less jq sudo zsh \
    shellcheck \
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

# Install oh-my-zsh as root but configure it for the vscode user home
RUN sh -c "$(curl -fsSL \
    --connect-timeout 30 --max-time 120 \
    https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)" \
    "" --unattended \
    && cp -R /root/.oh-my-zsh /home/vscode/ \
    && cp /root/.zshrc /home/vscode/ \
    && sed -i 's/\/root/\/home\/vscode/g' /home/vscode/.zshrc \
    && chown -R vscode:vscode /home/vscode/.oh-my-zsh /home/vscode/.zshrc

COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /usr/bin/

# ── Stage 2: toolchain ────────────────────────────────────────────────────────
FROM base AS toolchain
# hadolint ignore=DL3002
USER root
SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# hadolint ignore=DL3008
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential meson ninja-build python3 python3-venv pkg-config \
    libglib2.0-dev libpixman-1-dev flex bison device-tree-compiler \
    libslirp-dev libclang-dev libssl-dev libstdc++6 b4 gdb gdb-multiarch strace \
    lcov gcovr patchelf clang-format clang ccache dwarves \
    gcc-riscv64-linux-gnu binutils-riscv64-linux-gnu \
    lld cppcheck codespell \
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
ARG ACTIONLINT_VERSION
ARG HADOLINT_VERSION

RUN curl -s https://raw.githubusercontent.com/rhysd/actionlint/main/scripts/download-actionlint.bash | bash -s -- "${ACTIONLINT_VERSION}" \
    && mv actionlint /usr/local/bin/ \
    && case "${TARGETARCH}" in \
         amd64) HADO_ARCH="x86_64" ;; \
         arm64) HADO_ARCH="arm64" ;; \
       esac \
    && curl -sL "https://github.com/hadolint/hadolint/releases/download/v${HADOLINT_VERSION}/hadolint-Linux-${HADO_ARCH}" -o /usr/local/bin/hadolint \
    && chmod +x /usr/local/bin/hadolint

RUN uv python install ${PYTHON_VERSION}

ARG FLATBUFFERS_VERSION
RUN curl -L -o /tmp/flatc.zip "https://github.com/google/flatbuffers/releases/download/v${FLATBUFFERS_VERSION}/Linux.flatc.binary.clang++-18.zip" \
    && unzip /tmp/flatc.zip -d /usr/local/bin \
    && chmod +x /usr/local/bin/flatc \
    && rm /tmp/flatc.zip

ARG CMAKE_VERSION
ENV UV_LINK_MODE=copy
RUN --mount=type=cache,target=/root/.cache/uv uv pip install --system cmake==${CMAKE_VERSION} --break-system-packages

# ── Stage 3: flatcc-builder ───────────────────────────────────────────────────
# Built only when needed for simulation (QEMU)
FROM base AS flatcc-builder
ARG FLATCC_VERSION
# hadolint ignore=DL3008
RUN apt-get update && apt-get install -y --no-install-recommends build-essential cmake ninja-build \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build/flatcc
RUN git clone --depth=1 --branch v${FLATCC_VERSION} https://github.com/dvidelabs/flatcc.git . \
    && cmake -B build -G Ninja -Wno-dev \
       -DCMAKE_BUILD_TYPE=Release \
       -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
       -DFLATCC_TEST=OFF \
       -DFLATCC_CXX_TEST=OFF \
       -DFLATCC_INSTALL=ON \
       -DCMAKE_INSTALL_PREFIX=/usr/local \
       -DCMAKE_C_FLAGS="-w" \
    && cmake --build build --target install

# ── Stage 4: simulation-toolchain ─────────────────────────────────────────────
# Inherits from toolchain and adds sim-only requirements (flatcc, zenoh-c)
FROM toolchain AS simulation-toolchain

COPY --from=flatcc-builder /usr/local/bin/flatcc /usr/local/bin/
COPY --from=flatcc-builder /usr/local/include/flatcc /usr/local/include/flatcc
COPY --from=flatcc-builder /usr/local/lib/libflatcc* /usr/local/lib/

ARG ZENOH_C_REF
COPY --from=rust-builder /usr/local/cargo/bin/bindgen /usr/local/bin/bindgen
COPY --from=rust-builder /usr/local/cargo/bin/cargo-audit /usr/local/bin/cargo-audit
COPY --from=rust-builder /usr/local/cargo/bin/cargo-deny /usr/local/bin/cargo-deny
COPY --from=rust-builder /usr/local/cargo/bin/cargo-machete /usr/local/bin/cargo-machete
COPY --from=rust-builder /usr/local/rustup /usr/local/rustup
COPY --from=rust-builder /usr/local/cargo /usr/local/cargo
RUN chown -R vscode:vscode /usr/local/rustup /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:${PATH}

WORKDIR /build
RUN git clone --depth=1 --branch "${ZENOH_C_REF}" \
       https://github.com/eclipse-zenoh/zenoh-c.git /build/zenoh-c-src
WORKDIR /build/zenoh-c-build
RUN cmake -S /build/zenoh-c-src -B /build/zenoh-c-build \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX=/build/zenoh-c \
    -DZENOHC_BUILD_WITH_SHARED_MEMORY=OFF \
    && cmake --build /build/zenoh-c-build -j"$(nproc)" \
    && cmake --install /build/zenoh-c-build \
    && rm -rf /build/zenoh-c-src /build/zenoh-c-build

# ── Stage 5: qemu-builder ─────────────────────────────────────────────────────
FROM simulation-toolchain AS qemu-builder

ARG QEMU_REPO
ARG QEMU_REF
ARG TARGETARCH

COPY --from=simulation-toolchain /build/zenoh-c /build/zenoh-c

ENV ARCH=arm
WORKDIR /build
RUN git clone --depth=1 --branch "${QEMU_REF}" "${QEMU_REPO}" qemu

COPY patches/ /build/patches/
COPY scripts/apply-qemu-patches.sh /build/scripts/apply-qemu-patches.sh

# Apply all patches using the centralized script
RUN bash /build/scripts/apply-qemu-patches.sh /build/qemu

# Pre-configure with a dummy virtmcu directory to cache the core build.
# This ensures that changing files in hw/ only triggers a module rebuild.
RUN mkdir -p qemu/hw/virtmcu \
    && echo "hw_virtmcu_modules = {}" > qemu/hw/virtmcu/meson.build \
    && echo "modules += {'hw-virtmcu': hw_virtmcu_modules}" >> qemu/hw/virtmcu/meson.build \
    && grep -q "subdir('virtmcu')" qemu/hw/meson.build || echo "subdir('virtmcu')" >> qemu/hw/meson.build

ARG USE_CCACHE=false
ENV CCACHE_DIR=/var/cache/ccache \
    CCACHE_MAXSIZE=2G \
    CCACHE_SLOPPINESS=time_macros,include_file_mtime

WORKDIR /build/qemu/build-virtmcu
RUN if [ "${USE_CCACHE}" = "true" ]; then \
        ../configure \
        --prefix=/opt/virtmcu \
        --target-list="${ARCH}-softmmu,riscv32-softmmu,riscv64-softmmu" \
        --cc="ccache gcc" --cxx="ccache g++" \
        --enable-rust \
        --enable-modules \
        --enable-fdt \
        --enable-plugins \
        --enable-debug \
        --enable-gcov \
        --disable-werror \
        --disable-docs \
        --disable-dbus-display \
        --extra-ldflags="-fuse-ld=lld -rdynamic"; \
    else \
        ../configure \
        --prefix=/opt/virtmcu \
        --target-list="${ARCH}-softmmu,riscv32-softmmu,riscv64-softmmu" \
        --enable-rust \
        --enable-modules \
        --enable-fdt \
        --enable-plugins \
        --enable-debug \
        --enable-gcov \
        --disable-werror \
        --disable-docs \
        --disable-dbus-display \
        --extra-ldflags="-fuse-ld=lld -rdynamic"; \
    fi
RUN --mount=type=cache,target=/var/cache/ccache \
    make -j"$(nproc)"

# ── Stage 6: builder ──────────────────────────────────────────────────────────
FROM qemu-builder AS builder

COPY Cargo.toml Cargo.lock /build/virtmcu/
COPY hw/ /build/virtmcu/hw/
COPY tools/ /build/virtmcu/tools/
RUN rm -rf /build/qemu/hw/virtmcu && ln -sf /build/virtmcu/hw /build/qemu/hw/virtmcu \
    && grep -q "subdir('virtmcu')" /build/qemu/hw/meson.build || echo "subdir('virtmcu')" >> /build/qemu/hw/meson.build \
    && ln -sf /build/virtmcu/Cargo.toml /build/qemu/hw/Cargo.toml \
    && ln -sf /build/virtmcu/Cargo.lock /build/qemu/hw/Cargo.lock

WORKDIR /build/qemu/build-virtmcu
ARG VIRTMCU_USE_ASAN=0
ARG TARGETARCH
# hadolint ignore=SC2016
RUN --mount=type=cache,target=/var/cache/ccache \
    --mount=type=cache,id=cargo-target-${TARGETARCH},target=/build/qemu/build-virtmcu/hw/virtmcu/target \
    --mount=type=cache,id=cargo-registry-${TARGETARCH},target=/usr/local/cargo/registry \
    VIRTMCU_USE_ASAN=${VIRTMCU_USE_ASAN} make -j"$(nproc)" \
    && make install \
    && cargo build --release --manifest-path /build/virtmcu/Cargo.toml -p zenoh_coordinator -p cyber_bridge --target-dir /build/qemu/build-virtmcu/hw/virtmcu/target \
    && cp /build/qemu/build-virtmcu/hw/virtmcu/target/release/zenoh_coordinator /opt/virtmcu/bin/ \
    && cp /build/qemu/build-virtmcu/hw/virtmcu/target/release/mujoco_bridge /opt/virtmcu/bin/ \
    && cp /build/qemu/build-virtmcu/hw/virtmcu/target/release/resd_replay /opt/virtmcu/bin/


# hadolint ignore=SC2016
RUN mkdir -p /opt/virtmcu/lib/qemu/plugins \
    && mkdir -p /opt/virtmcu/include \
    && cp -R /build/zenoh-c/include/* /opt/virtmcu/include/ \
    && cp -R /usr/local/include/flatcc /opt/virtmcu/include/ \
    && find /opt/virtmcu/lib -name "hw-virtmcu-*.so" -exec mv {} /opt/virtmcu/lib/qemu/ \; \
    && find contrib/plugins -name "*.so" -exec cp {} /opt/virtmcu/lib/qemu/plugins/ \; 2>/dev/null || true \
    && find /build/zenoh-c -name "libzenohc.so" -exec cp {} /opt/virtmcu/lib/ \; 2>/dev/null || true \
    && find /opt/virtmcu/bin -type f -executable -exec patchelf --set-rpath '$ORIGIN/../lib' {} + \
    && find /opt/virtmcu/lib -name "*.so" -exec patchelf --set-rpath '$ORIGIN:$ORIGIN/..:$ORIGIN/../..:$ORIGIN/../../..' {} + \
    && find /opt/virtmcu/lib -name "hw-uefi-vars.so" -delete 2>/dev/null || true \
    && find /build/qemu/build-virtmcu -maxdepth 1 -name "qemu-system-*" -type f -delete \
    && ls -R /opt/virtmcu/lib

ENV PATH="/opt/virtmcu/bin:${PATH}"
ENV LD_LIBRARY_PATH="/opt/virtmcu/lib"
ENV QEMU_MODULE_DIR="/opt/virtmcu/lib/qemu"

# ── Stage 7: devenv-base ──────────────────────────────────────────────────────
FROM toolchain AS devenv-base

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

LABEL org.opencontainers.image.source=https://github.com/refractsystems/virtmcu
LABEL org.opencontainers.image.description="virtmcu development environment base — toolchain + AI tools"
LABEL org.opencontainers.image.licenses=MIT

# Pre-install Python dependencies for the simulation tools
COPY pyproject.toml /tmp/pyproject.toml
ENV UV_LINK_MODE=copy
RUN --mount=type=cache,target=/root/.cache/uv uv pip install --system --break-system-packages -r /tmp/pyproject.toml

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

COPY --from=rust-builder /usr/local/cargo/bin/bindgen /usr/local/bin/bindgen
COPY --from=rust-builder /usr/local/cargo/bin/cargo-audit /usr/local/bin/cargo-audit
COPY --from=rust-builder /usr/local/cargo/bin/cargo-deny /usr/local/bin/cargo-deny
COPY --from=rust-builder /usr/local/cargo/bin/cargo-machete /usr/local/bin/cargo-machete

COPY --from=rust-builder /usr/local/rustup /usr/local/rustup
COPY --from=rust-builder /usr/local/cargo /usr/local/cargo
RUN chown -R vscode:vscode /usr/local/rustup /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:${PATH}

WORKDIR /workspace
# devenv-base remains root by default for CI robustness.
ENV PATH="/home/vscode/.local/bin:${PATH}"

# ── Stage 8: devenv ───────────────────────────────────────────────────────────
FROM devenv-base AS devenv

LABEL org.opencontainers.image.description="virtmcu development environment — toolchain + AI tools + QEMU"

# PRE-CACHE QEMU: Copy pre-built QEMU from builder stage at the end to maximize cache
COPY --from=builder /opt/virtmcu /opt/virtmcu

# Pre-populate the Cargo registry so the first `make build` in a new devcontainer
# only needs to compile, not download hundreds of crates over the network.
COPY Cargo.toml Cargo.lock /tmp/virtmcu/
COPY hw/ /tmp/virtmcu/hw/
COPY tools/ /tmp/virtmcu/tools/
RUN cargo fetch --manifest-path /tmp/virtmcu/Cargo.toml \
    && rm -rf /tmp/virtmcu

ENV PATH="/opt/virtmcu/bin:${PATH}"
ENV LD_LIBRARY_PATH="/opt/virtmcu/lib"
ENV QEMU_MODULE_DIR="/opt/virtmcu/lib/qemu"

# Switch to vscode for interactive use
USER vscode

# ── Stage 9: runtime ──────────────────────────────────────────────────────────
FROM base AS runtime
LABEL org.opencontainers.image.source=https://github.com/refractsystems/virtmcu
LABEL org.opencontainers.image.description="virtmcu runtime image — QEMU + Python simulation tools"
LABEL org.opencontainers.image.licenses=MIT

ARG TARGETARCH
ENV ARCH=arm
ENV DEBIAN_FRONTEND=noninteractive

# hadolint ignore=DL3008
RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 libglib2.0-0t64 libpixman-1-0 libslirp0 libfdt1 \
    && rm -rf /var/lib/apt/lists/*

COPY pyproject.toml /tmp/pyproject.toml
WORKDIR /tmp
ENV UV_LINK_MODE=copy
RUN --mount=type=cache,target=/root/.cache/uv uv pip install --system --break-system-packages -r /tmp/pyproject.toml

COPY tools/ /app/tools/
RUN chown -R vscode:vscode /app/tools

COPY --from=builder /opt/virtmcu /opt/virtmcu

WORKDIR /app
# Keep runtime as root by default for system-level simulation.
# USER vscode

ENV PATH="/opt/virtmcu/bin:${PATH}"
ENV LD_LIBRARY_PATH="/opt/virtmcu/lib"
ENV QEMU_MODULE_DIR="/opt/virtmcu/lib/qemu"

CMD ["sh", "-c", "qemu-system-${ARCH} --version && echo 'virtmcu ready — set CMD in your world compose file'"]
