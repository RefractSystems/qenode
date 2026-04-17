  New stage graph

  debian:${DEBIAN_CODENAME}-slim
            │                                                                                                                                                                              
         [base]          ← replaces MCR: vscode user, zsh, oh-my-zsh, locale, GitHub CLI, uv
            │                                                                                                                                                                              
         [toolchain]     ← FROM base + build deps + ARM toolchain binary + Python pin + CMake
            │                                                                                                                                                                              
            ├── [flatcc-builder]   ← FROM toolchain (unchanged logic)
            │                                                                                                                                                                              
            ├── [devenv]           ← FROM toolchain + Rust + Node + Claude Code + Gemini
            │                                                                                                                                                                              
            ├── [builder]          ← FROM toolchain + Rust + QEMU + zenoh-c build
            │                                                                                                                                                                              
            └── [runtime]          ← FROM base (NOT toolchain) + QEMU install copy + Python deps
                                                                                                                                                                                           
  ---             
  Step 1 — VERSIONS file: five new entries                                                                                                                                                 
                                                                                                                                                                                           
  # Debian base image codename — single-line upgrade path
  DEBIAN_CODENAME=trixie                                                                                                                                                                   
                  
  # Node.js major version for NodeSource channel selection                                                                                                                                 
  NODE_VERSION=24 
                                                                                                                                                                                           
  # Python version — pinned via uv, independent of OS default                                                                                                                              
  PYTHON_VERSION=3.13                                                                                                                                                                  
                                                                                                                                                                                           
  # ARM GNU Toolchain — downloaded from developer.arm.com official releases                                                                                                                
  # Pinned independently of Debian apt to guarantee cross-compiler reproducibility.                                                                                                        
  # Binary fidelity (ADR-006) requires the toolchain to be bit-for-bit stable.                                                                                                             
  ARM_TOOLCHAIN_VERSION=13.3.rel1                                                                                                                                                          
                                                                                                                                                                                           
  Why each one matters:                                                                                                                                                                    
  - DEBIAN_CODENAME — upgrade from trixie to forky is a 1-line change, CI picks it up automatically via the existing cat VERSIONS >> $GITHUB_ENV step                                      
  - NODE_VERSION — currently a hardcoded literal 24 in the Dockerfile; must be in VERSIONS                                                                                                 
  - PYTHON_VERSION — pyproject.toml requires >=3.13 but CI hardcodes PYTHON_VERSION: "3.13" — these must all agree from one source
  - ARM_TOOLCHAIN_VERSION — using Debian's apt gcc-arm-none-eabi means a apt-get upgrade could silently change your cross-compiler; for a binary-fidelity project the toolchain must be as 
  reproducible as the firmware                                                                                                                                                             
                                                                                                                                                                                           
  ---                                                                                                                                                                                      
  Step 2 — base stage (the MCR replacement)                                                                                                                                                
                                                                                                                                                                                           
  Layers ordered slowest-to-fastest changing to maximise Docker cache reuse.
                                                                                                                                                                                           
  Layer 1: OS foundation                                                                                                                                                                   
  ARG DEBIAN_CODENAME=trixie                                                                                                                                                               
  FROM debian:${DEBIAN_CODENAME}-slim AS base                                                                                                                                              
                                             
  ENV DEBIAN_FRONTEND=noninteractive \
      LANG=en_US.UTF-8 \                                                                                                                                                                   
      LANGUAGE=en_US:en \
      LC_ALL=en_US.UTF-8 \                                                                                                                                                                 
      TERM=xterm-256color 
                                                                                                                                                                                           
  RUN apt-get update && apt-get install -y --no-install-recommends \
      locales tzdata ca-certificates \                                                                                                                                                     
      && locale-gen en_US.UTF-8 \     
      && update-locale LANG=en_US.UTF-8 \                                                                                                                                                  
      && ln -sf /usr/share/zoneinfo/UTC /etc/localtime \
      && rm -rf /var/lib/apt/lists/*                                                                                                                                                       
                                                                                                                                                                                           
  No Azure mirror sed hack — deb.debian.org is already a CDN-backed global mirror. It's gone entirely.
                                                                                                                                                                                           
  Layer 2: GitHub CLI apt source                                                                                                                                                           
                                                                                                                                                                                           
  Set up the repo in a dedicated layer so the keyring is cached independently of the package list:                                                                                         
  RUN apt-get update && apt-get install -y --no-install-recommends curl gnupg apt-transport-https \
      && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \                                                                                                        
         | gpg --dearmor -o /usr/share/keyrings/githubcli-archive-keyring.gpg \                                                                                                            
      && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] \
         https://cli.github.com/packages stable main" \                                                                                                                                    
         > /etc/apt/sources.list.d/github-cli.list \                                                                                                                                       
      && rm -rf /var/lib/apt/lists/*                                                                                                                                                       
                                                                                                                                                                                           
  Note: the GitHub CLI repo uses stable main with no Ubuntu/Debian codename — it works identically on any Debian-family release.                                                           
                                                                                                                                                                                           
  Layer 3: Common tools (MCR parity list)                                                                                                                                                  
  RUN apt-get update && apt-get install -y --no-install-recommends \                                                                                                                       
      git gh wget openssh-client gnupg2 lsb-release bash-completion \                                                                                                                      
      procps lsof htop net-tools psmisc unzip nano vim-tiny less jq sudo zsh \
      && rm -rf /var/lib/apt/lists/*                                                                                                                                                       
                                                                                                                                                                                           
  This is the exact tool set MCR's base-ubuntu image installs. Nothing more, nothing less.                                                                                                 
                                                                                                                                                                                           
  Layer 4: vscode user
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
                                              
  The shell is set here via useradd --shell, not by oh-my-zsh's install script. This is deliberate — see the risk section on chsh.                                                         
                                                                                                                                                                                           
  Layer 5: oh-my-zsh (what makes the terminal UX identical to MCR)                                                                                                                         
  USER vscode                                                                                                                                                                              
  RUN sh -c "$(curl -fsSL \                                                                                                                                                                
      --connect-timeout 30 --max-time 120 \
      https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)" \
      "" --unattended                                                                                                                                                                      
  USER root          
                                                                                                                                                                                           
  --unattended skips chsh (shell is already set) and all interactive prompts. The curl timeouts prevent a silent hang if GitHub is slow.                                                   
                                                                                                                                                                                           
  Layer 6: uv binary                                                                                                                                                                       
  COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /usr/bin/                                                                                                                               
                  
  ---                                                                                                                                                                                      
  Step 3 — toolchain stage: FROM base, zero duplication
                                                                                                                                                                                           
  Everything already in base is gone. What remains is build-specific:
                                                                                                                                                                                           
  Build tooling (apt):                                                                                                                                                                     
  FROM base AS toolchain                                                                                                                                                                   
  RUN apt-get update && apt-get install -y --no-install-recommends \                                                                                                                       
      build-essential meson ninja-build python3 python3-venv pkg-config \
      libglib2.0-dev libpixman-1-dev flex bison device-tree-compiler \                                                                                                                     
      libslirp-dev libstdc++6 b4 gdb gdb-multiarch strace \           
      lcov gcovr patchelf \                                                                                                                                                                
      && rm -rf /var/lib/apt/lists/*                                                                                                                                                       
                                                                                                                                                                                           
  libstdc++6 is explicit here — the prebuilt flatc binary dynamically links against it and it is absent from debian:trixie-slim.                                                           
                                                                                                                                                                                           
  ARM GNU Toolchain (binary release, version-pinned):                                                                                                                                      
  ARG ARM_TOOLCHAIN_VERSION=13.3.rel1                                                                                                                                                      
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
                                                                                                                                                                                           
  This removes the dependency on Debian's gcc-arm-none-eabi package entirely. The cross-compiler version is now as reproducible as QEMU and Zenoh. TARGETARCH is already used by the       
  existing builder stage so the pattern is established.                                                                                                                                    
                                                                                                                                                                                           
  RISC-V toolchain (apt — version less critical for firmware fidelity):                                                                                                                    
  RUN apt-get update && apt-get install -y --no-install-recommends \
      gcc-riscv64-linux-gnu binutils-riscv64-linux-gnu \                                                                                                                                   
      && rm -rf /var/lib/apt/lists/*                                                                                                                                                       
                                    
  Python (pinned via uv, independent of OS default):                                                                                                                                       
  ARG PYTHON_VERSION=3.13                                                                                                                                                              
  RUN uv python install ${PYTHON_VERSION}
                                                                                                                                                                                           
  uv downloads Python from python-build-standalone — glibc-generic binaries, no distro dependency. All project tooling already uses uv run or uv pip, so this is the natural home for the  
  pin.                                                                                                                                                                                     
                                                                                                                                                                                           
  FlatBuffers and CMake — logic unchanged, just inherit the same pattern:                                                                                                                  
  ARG FLATBUFFERS_VERSION
  ARG CMAKE_VERSION                                                                                                                                                                        
  RUN curl -L ... flatc.zip ...
  RUN uv pip install --system cmake==${CMAKE_VERSION} ...
                                                                                                                                                                                           
  ---
  Step 4 — devenv stage: FROM toolchain, drop MCR                                                                                                                                          
                                                                                                                                                                                           
  Remove FROM mcr.microsoft.com/devcontainers/base:ubuntu-24.04. Remove the duplicated apt block. What remains is only what devenv adds on top of toolchain:
                                                                                                                                                                                           
  FROM toolchain AS devenv
                                                                                                                                                                                           
  COPY --from=flatcc-builder ...  (unchanged)                                                                                                                                              
  
  COPY --from=rust-builder ...    (unchanged)                                                                                                                                              
  ENV RUSTUP_HOME=... CARGO_HOME=... PATH=...
                                                                                                                                                                                           
  # Node.js — uses NODE_VERSION from VERSIONS                                                                                                                                              
  ARG NODE_VERSION=24                                                                                                                                                                      
  RUN curl -fsSL https://deb.nodesource.com/setup_${NODE_VERSION}.x | bash - \                                                                                                             
      && apt-get install -y --no-install-recommends nodejs \
      && rm -rf /var/lib/apt/lists/*                                                                                                                                                       
  
  # Claude Code (as vscode user)                                                                                                                                                           
  USER vscode     
  RUN curl -fsSL https://claude.ai/install.sh | bash                                                                                                                                       
  ENV PATH="/home/vscode/.local/bin:${PATH}"
  USER root                                                                                                                                                                                
  
  RUN npm install -g @google/gemini-cli                                                                                                                                                    
                  
  WORKDIR /workspace

  Labels and LABEL metadata stay as-is.                                                                                                                                                    
  
  ---                                                                                                                                                                                      
  Step 5 — runtime stage: FROM base, not toolchain
                                                                                                                                                                                           
  This is both a correctness fix and a size fix:
                                                                                                                                                                                           
  FROM base AS runtime
                                                                                                                                                                                           
  LABEL ...       

  ENV ARCH=arm DEBIAN_FRONTEND=noninteractive                                                                                                                                              
  ARG TARGETARCH
                                                                                                                                                                                           
  COPY --from=builder /opt/virtmcu /opt/virtmcu                                                                                                                                            
  COPY --from=builder /build/zenoh-c/lib/libzenohc.so /opt/virtmcu/lib/
                                                                                                                                                                                           
  COPY pyproject.toml /tmp/pyproject.toml                                                                                                                                                  
  WORKDIR /tmp
  RUN uv pip install --no-cache --system --break-system-packages -r pyproject.toml                                                                                                         
                                                                                                                                                                                           
  COPY tools/ /app/tools/
  WORKDIR /app                                                                                                                                                                             
                                                                                                                                                                                           
  ENV PATH="/opt/virtmcu/bin:${PATH}"
  ENV LD_LIBRARY_PATH="/opt/virtmcu/lib"                                                                                                                                                   
  ENV QEMU_MODULE_DIR="/opt/virtmcu/lib/qemu"
                                                                                                                                                                                           
  CMD ["sh", "-c", "qemu-system-${ARCH} --version && echo 'virtmcu ready'"]                                                                                                                
                                                                                                                                                                                           
  ---                                                                                                                                                                                      
  Step 6 — sync-versions.py extension + new check-versions.py
                                                             
  Extend sync-versions.py with four new propagation rules matching the existing pattern (regex-replace ARG defaults in Dockerfile):
                                                                                                                                                                                           
  # propagate DEBIAN_CODENAME, NODE_VERSION, PYTHON_VERSION, ARM_TOOLCHAIN_VERSION
  # → ARG DEBIAN_CODENAME= in Dockerfile                                                                                                                                                   
                  
  Also fix the existing inconsistency: pyproject.toml pins flatbuffers>=25 (floor, not exact) while requirements.txt has flatbuffers==25.12.19. The sync script updates requirements.txt   
  but not pyproject.toml. Both should be ==. Update sync-versions.py to maintain both with exact pins.
                                                                                                                                                                                           
  New scripts/check-versions.py (read-only, CI enforcer):                                                                                                                                  
  
  Checks that every version key in VERSIONS matches:                                                                                                                                       
  - The corresponding ARG default in docker/Dockerfile
  - The corresponding pin in pyproject.toml                                                                                                                                                
  - The corresponding pin in requirements.txt
                                                                                                                                                                                           
  Fails with a clear diff if anything is out of sync. This is the gate that prevents merging version drift. The rule: sync-versions.py fixes, check-versions.py enforces.
                                                                                                                                                                                           
  Add to Makefile:
  check-versions:                                                                                                                                                                          
      @python3 scripts/check-versions.py
                                                                                                                                                                                           
  ---
  Step 7 — CI pipeline updates                                                                                                                                                             
                              
  Both ci.yml and docker-publish.yml need four new build-args entries. The Load VERSIONS step already exports them into $GITHUB_ENV — only the build-args block changes:
                                                                                                                                                                                           
  build-args: |
    DEBIAN_CODENAME=${{ env.DEBIAN_CODENAME }}                                                                                                                                             
    NODE_VERSION=${{ env.NODE_VERSION }}
    PYTHON_VERSION=${{ env.PYTHON_VERSION }}                                                                                                                                               
    ARM_TOOLCHAIN_VERSION=${{ env.ARM_TOOLCHAIN_VERSION }}
    QEMU_REF=v${{ env.QEMU_VERSION }}                                                                                                                                                      
    ZENOH_C_REF=${{ env.ZENOH_VERSION }}                                                                                                                                                   
    CMAKE_VERSION=${{ env.CMAKE_VERSION }}                                                                                                                                                 
    RUST_VERSION=${{ env.RUST_VERSION }}                                                                                                                                                   
    FLATBUFFERS_VERSION=${{ env.FLATBUFFERS_VERSION }}
                                                                                                                                                                                           
  Add check-versions as a new job in the lint tier of ci.yml.                                                                                                                              
  
  ---                                                                                                                                                                                      
  Execution order 
                                                                                                                                                                                           
  0. Pre-flight checks (verify assumptions before writing code — see below)
                                                                                                                                                                                           
  1. VERSIONS: add 5 new entries
                                                                                                                                                                                           
  2. docker/Dockerfile:                                                                                                                                                                    
     a. base stage (Debian, vscode, zsh, oh-my-zsh, uv)
     b. toolchain FROM base (remove duplication, ARM binary install, Python pin)                                                                                                           
     c. devenv FROM toolchain (drop MCR line)                                                                                                                                              
     d. runtime FROM base (not toolchain)
                                                                                                                                                                                           
  3. sync-versions.py: extend for new keys; fix flatbuffers >= vs == gap                                                                                                                   
     check-versions.py: new enforcer script
                                                                                                                                                                                           
  4. Makefile: add check-versions target

  5. ci.yml + docker-publish.yml: add 4 build-args; add check-versions job                                                                                                                 
  
  6. Local validation:                                                                                                                                                                     
     docker build --target devenv -t virtmcu-devenv .
     → open devcontainer                                                                                                                                                                   
     → verify: zsh prompt + oh-my-zsh theme, sudo without password,
       claude --version, gemini --version, uv sync, make setup                                                                                                                             
                                                                                                                                                                                           
  7. CI green → merge
                                                                                                                                                                                           
  ---             
  What can go wrong — full risk table
                                                                                                                                                                                           
  Build-time risks:
                                                                                                                                                                                           
  ┌────────────────────────────────────────────────┬───────────────────────┬───────────────────────┬──────────────────────────────────────────────────────────────────────────────────┐ 
  │                      Risk                      │      Likelihood       │        Impact         │                                    Mitigation                                    │ 
  ├────────────────────────────────────────────────┼───────────────────────┼───────────────────────┼──────────────────────────────────────────────────────────────────────────────────┤ 
  │ NodeSource setup_24.x doesn't recognize trixie │ Medium                │ Hard blocker          │ Pre-flight check below; fallback: install from nodejs.org binary tarball         │ 
  │                                                │                       │                       │ (OS-agnostic)                                                                    │ 
  ├────────────────────────────────────────────────┼───────────────────────┼───────────────────────┼──────────────────────────────────────────────────────────────────────────────────┤    
  │ ARM toolchain download URL returns 404         │ Low                   │ Hard blocker          │ Pre-flight check below; ARM's URL scheme has been stable since GCC 12 rebrand    │    
  ├────────────────────────────────────────────────┼───────────────────────┼───────────────────────┼──────────────────────────────────────────────────────────────────────────────────┤    
  │ flatc binary fails: missing libstdc++.so.6     │ High without          │ Breaks toolchain      │ Mitigated: libstdc++6 explicitly in toolchain apt block                          │    
  │                                                │ mitigation            │ build                 │                                                                                  │ 
  ├────────────────────────────────────────────────┼───────────────────────┼───────────────────────┼──────────────────────────────────────────────────────────────────────────────────┤    
  │ oh-my-zsh install.sh hangs                     │ Low                   │ Blocks base build     │ Mitigated: --connect-timeout 30 --max-time 120 on curl                           │ 
  ├────────────────────────────────────────────────┼───────────────────────┼───────────────────────┼──────────────────────────────────────────────────────────────────────────────────┤    
  │ chsh fails inside oh-my-zsh installer          │ Medium                │ zsh not default shell │ Mitigated: shell set via useradd --shell before installer runs; --unattended     │ 
  │                                                │                       │                       │ skips chsh                                                                       │    
  ├────────────────────────────────────────────────┼───────────────────────┼───────────────────────┼──────────────────────────────────────────────────────────────────────────────────┤ 
  │ uv python install 3.13 fails on trixie         │ Very low              │ Breaks Python pin     │ uv uses glibc-generic python-build-standalone binaries, not distro-specific      │    
  ├────────────────────────────────────────────────┼───────────────────────┼───────────────────────┼──────────────────────────────────────────────────────────────────────────────────┤    
  │ rust:${RUST_VERSION}-slim still based on       │ None                  │ Aesthetic mismatch    │ We COPY binaries from it, not layer on top — no runtime impact                   │
  │ bookworm                                       │                       │ only                  │                                                                                  │    
  └────────────────────────────────────────────────┴───────────────────────┴───────────────────────┴──────────────────────────────────────────────────────────────────────────────────┘
                                                                                                                                                                                           
  Devcontainer UX risks:

  ┌──────────────────────────────────────────┬──────────────────────────────────────────────┬──────────────────────────────────────────────────────────────────────────────────────────┐
  │                   Risk                   │                  Root cause                  │                                        Mitigation                                        │
  ├──────────────────────────────────────────┼──────────────────────────────────────────────┼──────────────────────────────────────────────────────────────────────────────────────────┤   
  │ ~/.claude.json bind mount creates a      │ Docker creates dir for missing-file bind     │ Prepend `touch ~/.claude.json                                                            │
  │ directory                                │ mounts                                       │                                                                                          │   
  ├──────────────────────────────────────────┼──────────────────────────────────────────────┼──────────────────────────────────────────────────────────────────────────────────────────┤   
  │ VS Code terminal doesn't default to zsh  │ devcontainer spec doesn't auto-detect login  │ Add "terminal.integrated.defaultProfile.linux": "zsh" to devcontainer.json               │   
  │                                          │ shell                                        │ customizations                                                                           │   
  ├──────────────────────────────────────────┼──────────────────────────────────────────────┼──────────────────────────────────────────────────────────────────────────────────────────┤   
  │ postCreateCommand chown silently no-ops  │ Mount not present at postCreateCommand time  │ Change to sudo mkdir -p /home/vscode/.claude && sudo chown -R vscode:vscode              │
  │                                          │                                              │ /home/vscode/.claude                                                                     │   
  └──────────────────────────────────────────┴──────────────────────────────────────────────┴──────────────────────────────────────────────────────────────────────────────────────────┘
                                                                                                                                                                                           
  Version drift risks:

  ┌──────────────────────────────────────────────────────────┬───────────────────────────────────────────────┬────────────────────────────────────────────────────────────────────────┐
  │                           Risk                           │                  Root cause                   │                               Mitigation                               │
  ├──────────────────────────────────────────────────────────┼───────────────────────────────────────────────┼────────────────────────────────────────────────────────────────────────┤    
  │ flatbuffers>=25 in pyproject.toml vs ==25.12.19 in       │ Pre-existing inconsistency                    │ Fix in this PR; sync-versions.py updates both                          │
  │ requirements.txt                                         │                                               │                                                                        │    
  ├──────────────────────────────────────────────────────────┼───────────────────────────────────────────────┼────────────────────────────────────────────────────────────────────────┤    
  │ Someone bumps VERSIONS without running sync-versions     │ Manual step today                             │ check-versions.py in CI lint catches before merge                      │    
  ├──────────────────────────────────────────────────────────┼───────────────────────────────────────────────┼────────────────────────────────────────────────────────────────────────┤    
  │ ARM toolchain URL format changes on next major version   │ ARM renamed toolchain once (GCC 11 → 12);     │ Pin URL pattern in VERSIONS comment; human-review ARM changelog when   │    
  │                                                          │ stable since                                  │ bumping version                                                        │
  └──────────────────────────────────────────────────────────┴───────────────────────────────────────────────┴────────────────────────────────────────────────────────────────────────┘    
                  
  ---
  Assumptions to verify before writing any code
                                               
  These are ordered by "if wrong, the approach changes":
                                                                                                                                                                                           
  1. NodeSource recognizes trixie — the single highest-risk item:                                                                                                                          
  curl -fsSL https://deb.nodesource.com/setup_24.x | grep -c trixie                                                                                                                        
  Expected: nonzero. If zero: switch Node install to https://nodejs.org/dist/v${NODE_VERSION}.x.x/node-v${NODE_VERSION}.x.x-linux-x64.tar.xz — a direct binary download with no codename   
  dependency.     
                                                                                                                                                                                           
  2. MCR vscode UID is 1000:
  docker run --rm mcr.microsoft.com/devcontainers/base:ubuntu-24.04 id vscode                                                                                                              
  Expected: uid=1000(vscode) gid=1000(vscode). Our useradd hardcodes 1000 — this must match.
                                                                                                                                                                                           
  3. ARM toolchain download URL is live:                                                                                                                                                   
  curl -fsSI "https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/arm-gnu-toolchain-13.3.rel1-x86_64-arm-none-eabi.tar.xz"                                              
  Expected: HTTP 200. If 404: check ARM's releases page for the current URL format and update the Dockerfile pattern.                                                                      
                                                                                                                                                                                           
  4. debian:trixie-slim exists and uses expected apt sources:                                                                                                                              
  docker run --rm debian:trixie-slim ls /etc/apt/sources.list /etc/apt/sources.list.d/ 2>&1                                                                                                
  docker run --rm debian:trixie-slim cat /etc/apt/sources.list 2>/dev/null || \                                                                                                            
    docker run --rm debian:trixie-slim cat /etc/apt/sources.list.d/debian.sources                                                                                                          
  Confirms: (a) the tag exists; (b) there's no Ubuntu-style mirror manipulation needed.                                                                                                    
                                                                                                                                                                                           
  5. libstdc++6 is absent from debian:trixie-slim:                                                                                                                                         
  docker run --rm debian:trixie-slim dpkg -l libstdc++6                                                                                                                                    
  If present by default, drop it from the explicit apt list. If absent (expected for slim), keep it.                                                                                       
                                                                                                                                                                                           
  6. uv python install works on trixie-slim:                                                                                                                                               
  docker run --rm -v $(which uv):/usr/bin/uv debian:trixie-slim \                                                                                                                          
    uv python install 3.13 --preview                                                                                                                                                       
                                                                                                                                                                                           
  ---                                                                                                                                                                                      
  The codename upgrade story                                                                                                                                                               
                                                                                                                                                                                           
  When Forky (Debian 14) becomes stable:
                                                                                                                                                                                           
  -DEBIAN_CODENAME=trixie
  +DEBIAN_CODENAME=forky                                                                                                                                                                   
                  
  Then make sync-versions. Every stage picks it up. ARM toolchain, Python, Node, Rust, CMake, QEMU, Zenoh — all bypass the OS entirely, so they're unaffected by the base upgrade. The only
   thing to verify is that GitHub CLI's stable main repo still resolves correctly (it has on every Debian release so far) and that NodeSource adds forky support. Both are quick checks,
  not migrations.                                                                                                                                                                          
                  
  That's the payoff of centralising in VERSIONS: one line changes, everything else is enforced to follow.      