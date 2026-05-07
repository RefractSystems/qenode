# virtmcu QEMU container image.
#
# Multi-stage build:
#   base      — debian:${DEBIAN_CODENAME}-slim + vscode user, zsh, oh-my-zsh, common tools
#   toolchain — FROM base + build deps, ARM toolchain, Python pin, CMake, FlatBuffers (lean)
#   sim-tools — FROM toolchain + flatcc + zenoh-c (simulation specific)
#   builder   — FROM sim-tools: clones QEMU, compiles everything
#   devenv    — FROM toolchain + Rust + Node.js + AI tools + pre-built QEMU

ARG QEMU_REPO=https://gitlab.com/qemu-project/qemu.git
ARG QEMU_REF=v11.0.0
ARG ZENOH_C_REF=1.9.0
ARG CMAKE_VERSION=4.3.1
ARG MESON_VERSION=1.8.0
ARG RUST_VERSION=1.95.0
ARG BINDGEN_VERSION=0.72.1
ARG CARGO_BINSTALL_VERSION=1.10.4
ARG UV_VERSION=0.11.10
ARG FLATBUFFERS_VERSION=25.12.19
ARG FLATCC_VERSION=0.6.1
ARG HADOLINT_VERSION=2.14.0
ARG ACTIONLINT_VERSION=1.7.12
ARG MDBOOK_VERSION=0.4.52
ARG MDBOOK_MERMAID_VERSION=0.14.0
ARG MDBOOK_PDF_VERSION=0.1.10
ARG DEBIAN_CODENAME=trixie
ARG NODE_VERSION=24
ARG PYTHON_VERSION=3.13
ARG ARM_TOOLCHAIN_VERSION=13.3.rel1
ARG TARGETARCH

# Alias stage for uv to bypass COPY --from variable expansion limits
FROM ghcr.io/astral-sh/uv:${UV_VERSION} AS uv-bin

# ── Stage 0: rust-builder ─────────────────────────────────────────────────────
FROM public.ecr.aws/docker/library/rust:${RUST_VERSION}-slim AS rust-builder
ARG BINDGEN_VERSION
ARG MDBOOK_VERSION
ARG MDBOOK_MERMAID_VERSION
ARG MDBOOK_PDF_VERSION
ARG RUST_VERSION
ARG CARGO_BINSTALL_VERSION
ENV CARGO_HOME=/usr/local/cargo
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
# hadolint ignore=DL3008
RUN --mount=type=secret,id=GITHUB_TOKEN \
    apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
    && curl -fsSL -o /tmp/binstall.tgz "https://github.com/cargo-bins/cargo-binstall/releases/download/v${CARGO_BINSTALL_VERSION}/cargo-binstall-$(uname -m)-unknown-linux-musl.tgz" \
    && tar -xzf /tmp/binstall.tgz -C /usr/local/cargo/bin \
    && rm /tmp/binstall.tgz \
    && rustup toolchain install nightly --profile minimal --component miri \
    && rustup component add rustfmt clippy rust-analyzer \
    && GITHUB_TOKEN=$(cat /run/secrets/GITHUB_TOKEN 2>/dev/null || echo "") && export GITHUB_TOKEN \
    && cargo binstall -y bindgen-cli@"${BINDGEN_VERSION}" \
    && cargo binstall -y cargo-audit \
    && cargo binstall -y cargo-deny \
    && cargo binstall -y cargo-machete \
    && cargo binstall -y mdbook@"${MDBOOK_VERSION}" \
    && cargo binstall -y mdbook-mermaid@"${MDBOOK_MERMAID_VERSION}" \
    && curl -sSL "https://github.com/HollowMan6/mdbook-pdf/releases/download/v${MDBOOK_PDF_VERSION}/mdbook-pdf-v${MDBOOK_PDF_VERSION}-$(uname -m)-unknown-linux-gnu" -o /usr/local/cargo/bin/mdbook-pdf \
    && chmod +x /usr/local/cargo/bin/mdbook-pdf \
    && rm -rf /var/lib/apt/lists/*

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

COPY --from=uv-bin /uv /uvx /usr/bin/

# ── Stage 2: toolchain ────────────────────────────────────────────────────────
FROM base AS toolchain
# hadolint ignore=DL3002
USER root
SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# hadolint ignore=DL3008
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential meson ninja-build python3 python3-venv python3-dev pkg-config \
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

RUN curl -fsSL --retry 3 "https://raw.githubusercontent.com/rhysd/actionlint/main/scripts/download-actionlint.bash" | bash -s -- "${ACTIONLINT_VERSION}" \
    && mv actionlint /usr/local/bin/ \
    && case "${TARGETARCH}" in \
         amd64) HADO_ARCH="x86_64" ;; \
         arm64) HADO_ARCH="arm64" ;; \
       esac \
    && curl -fsSL --retry 3 "https://github.com/hadolint/hadolint/releases/download/v${HADOLINT_VERSION}/hadolint-Linux-${HADO_ARCH}" -o /usr/local/bin/hadolint \
    && chmod +x /usr/local/bin/hadolint

RUN uv python install ${PYTHON_VERSION}

ARG FLATBUFFERS_VERSION
RUN curl -fsSL --retry 3 -o /tmp/flatc.zip "https://github.com/google/flatbuffers/releases/download/v${FLATBUFFERS_VERSION}/Linux.flatc.binary.clang++-18.zip" \
    && unzip /tmp/flatc.zip -d /usr/local/bin \
    && chmod +x /usr/local/bin/flatc \
    && rm /tmp/flatc.zip

ARG CMAKE_VERSION
ARG MESON_VERSION
ENV UV_LINK_MODE=copy
RUN --mount=type=cache,target=/root/.cache/uv uv pip install --system cmake==${CMAKE_VERSION} meson==${MESON_VERSION} --break-system-packages

# ── Stage 3: devenv-base ──────────────────────────────────────────────────────
FROM toolchain AS devenv-base

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

LABEL org.opencontainers.image.source=https://github.com/refractsystems/virtmcu
LABEL org.opencontainers.image.description="virtmcu development environment base — toolchain + AI tools"
LABEL org.opencontainers.image.licenses=MIT

# Install Chromium for mdbook-pdf headless rendering
# hadolint ignore=DL3008
RUN apt-get update && apt-get install -y --no-install-recommends chromium \
    && rm -rf /var/lib/apt/lists/*

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
COPY --from=rust-builder /usr/local/cargo/bin/mdbook /usr/local/bin/mdbook
COPY --from=rust-builder /usr/local/cargo/bin/mdbook-mermaid /usr/local/bin/mdbook-mermaid
COPY --from=rust-builder /usr/local/cargo/bin/mdbook-pdf /usr/local/bin/mdbook-pdf

COPY --from=rust-builder /usr/local/rustup /usr/local/rustup
COPY --from=rust-builder /usr/local/cargo /usr/local/cargo
RUN chown -R vscode:vscode /usr/local/rustup /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:${PATH}

WORKDIR /workspace
# devenv-base remains root by default for CI robustness.
ENV PATH="/home/vscode/.local/bin:${PATH}"

# ── Stage 4: flatcc-builder ───────────────────────────────────────────────────
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

# ── Stage 5: qemu-builder ─────────────────────────────────────────────────────
FROM devenv-base AS qemu-builder

COPY --from=flatcc-builder /usr/local/bin/flatcc /usr/local/bin/
COPY --from=flatcc-builder /usr/local/include/flatcc /usr/local/include/flatcc
COPY --from=flatcc-builder /usr/local/lib/libflatcc* /usr/local/lib/

ARG ZENOH_C_REF
WORKDIR /build
RUN git clone --depth=1 --branch "${ZENOH_C_REF}" \
       https://github.com/eclipse-zenoh/zenoh-c.git /build/zenoh-c-src \
    && cmake -S /build/zenoh-c-src -B /build/zenoh-c-build \
       -DCMAKE_BUILD_TYPE=Release \
       -DCMAKE_INSTALL_PREFIX=/build/zenoh-c \
       -DZENOHC_BUILD_WITH_SHARED_MEMORY=OFF \
    && cmake --build /build/zenoh-c-build -j"$(nproc)" \
    && cmake --install /build/zenoh-c-build \
    && rm -rf /build/zenoh-c-src /build/zenoh-c-build

ARG QEMU_REPO
ARG QEMU_REF
ARG TARGETARCH


ENV ARCH=arm
WORKDIR /build
RUN git clone --depth=1 --branch "${QEMU_REF}" "${QEMU_REPO}" qemu

COPY patches/ /build/patches/
COPY BUILD_DEPS /build/
RUN mkdir -p /build/scripts
COPY scripts/apply-qemu-patches.sh scripts/common.sh /build/scripts/

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
        --prefix="$(pwd)/install" \
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
        --prefix="$(pwd)/install" \
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
    && ln -sf /build/virtmcu/Cargo.lock /build/qemu/hw/Cargo.lock \
    && touch /build/virtmcu/hw/meson.build /build/virtmcu/hw/rust/build.sh /build/virtmcu/hw/gen_trigger.sh

WORKDIR /build/qemu/build-virtmcu
ARG VIRTMCU_USE_ASAN=0
ARG TARGETARCH
# hadolint ignore=SC2016
RUN --mount=type=cache,target=/var/cache/ccache \
    --mount=type=cache,id=cargo-target-${TARGETARCH},target=/build/qemu/build-virtmcu/hw/virtmcu/target \
    --mount=type=cache,id=cargo-registry-${TARGETARCH},target=/usr/local/cargo/registry \
    VIRTMCU_USE_ASAN=${VIRTMCU_USE_ASAN} make -j"$(nproc)" \
    && make install \
    && cargo build --release --manifest-path /build/virtmcu/Cargo.toml -p deterministic_coordinator -p cyber_bridge --target-dir /build/qemu/build-virtmcu/hw/virtmcu/target \
    && cp /build/qemu/build-virtmcu/hw/virtmcu/target/release/deterministic_coordinator /usr/local/bin/ \
    && cp /build/qemu/build-virtmcu/hw/virtmcu/target/release/mujoco_bridge /usr/local/bin/ \
    && cp /build/qemu/build-virtmcu/hw/virtmcu/target/release/resd_replay /usr/local/bin/


# hadolint ignore=SC2016
RUN mkdir -p /usr/local/lib/qemu/plugins \
    && mkdir -p /usr/local/include \
    && cp -R /build/zenoh-c/include/* /usr/local/include/ \
    && cp -R /usr/local/include/flatcc /usr/local/include/ \
    && find contrib/plugins -name "*.so" -exec cp {} /usr/local/lib/qemu/plugins/ \; 2>/dev/null || true \
    && find /build/zenoh-c -name "libzenohc.so" -exec cp {} /usr/local/lib/ \; 2>/dev/null || true \
    && find /usr/local/bin -type f -executable -exec patchelf --set-rpath '$ORIGIN/../lib' {} + \
    && find /usr/local/lib -name "*.so" -exec patchelf --set-rpath '$ORIGIN:$ORIGIN/..:$ORIGIN/../..:$ORIGIN/../../..' {} + \
    && find /usr/local/lib -name "hw-uefi-vars.so" -delete 2>/dev/null || true \
    && find /build/qemu/build-virtmcu -maxdepth 1 -name "qemu-system-*" -type f -delete \
    && ls -R /usr/local/lib


# ── Stage 8: devenv ───────────────────────────────────────────────────────────
FROM devenv-base AS devenv

LABEL org.opencontainers.image.description="virtmcu development environment — toolchain + AI tools + QEMU"

# PRE-CACHE QEMU: Copy pre-built QEMU from builder stage at the end to maximize cache

# Pre-populate the Cargo registry so the first `make build` in a new devcontainer
# only needs to compile, not download hundreds of crates over the network.
COPY Cargo.toml Cargo.lock /tmp/virtmcu/
COPY hw/ /tmp/virtmcu/hw/
COPY tools/ /tmp/virtmcu/tools/
RUN cargo fetch --manifest-path /tmp/virtmcu/Cargo.toml \
    && rm -rf /tmp/virtmcu \
    && chown -R vscode:vscode /usr/local/cargo/registry \
    && mkdir -p /workspace/target \
    && chown -R vscode:vscode /workspace/target


# Switch to vscode for interactive use
USER vscode

