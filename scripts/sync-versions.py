#!/usr/bin/env python3
import os
import re


def get_versions():
    versions = {}
    with open("VERSIONS", "r") as f:
        for line in f:
            if "=" in line and not line.startswith("#"):
                key, value = line.strip().split("=")
                versions[key] = value
    return versions


def sync():
    versions = get_versions()
    zenoh_ver = versions.get("ZENOH_VERSION")
    if not zenoh_ver:
        print("Error: ZENOH_VERSION not found in VERSIONS")
        return

    # 1. Update tools/zenoh_coordinator/Cargo.toml
    cargo_path = "tools/zenoh_coordinator/Cargo.toml"
    if os.path.exists(cargo_path):
        with open(cargo_path, "r") as f:
            content = f.read()
        new_content = re.sub(r'zenoh = "[^"]+"', f'zenoh = "{zenoh_ver}"', content)
        if content != new_content:
            print(f"Updating {cargo_path} to zenoh {zenoh_ver}")
            with open(cargo_path, "w") as f:
                f.write(new_content)

    # 2. Update worlds/pendulum.yml
    pendulum_path = "worlds/pendulum.yml"
    if os.path.exists(pendulum_path):
        with open(pendulum_path, "r") as f:
            content = f.read()
        new_content = re.sub(
            r"uv pip install eclipse-zenoh==[^\s]+", f"uv pip install eclipse-zenoh=={zenoh_ver}", content
        )
        if content != new_content:
            print(f"Updating {pendulum_path} to eclipse-zenoh {zenoh_ver}")
            with open(pendulum_path, "w") as f:
                f.write(new_content)

    # 3. Update requirements.txt
    req_path = "requirements.txt"
    if os.path.exists(req_path):
        with open(req_path, "r") as f:
            content = f.read()
        new_content = re.sub(r"eclipse-zenoh==[^\s]+", f"eclipse-zenoh=={zenoh_ver}", content)
        if content != new_content:
            print(f"Updating {req_path} to eclipse-zenoh {zenoh_ver}")
            with open(req_path, "w") as f:
                f.write(new_content)

    # 3.5 Update pyproject.toml
    pyproject_path = "pyproject.toml"
    if os.path.exists(pyproject_path):
        with open(pyproject_path, "r") as f:
            content = f.read()
        new_content = re.sub(r'"eclipse-zenoh==[^"]+"', f'"eclipse-zenoh=={zenoh_ver}"', content)
        if content != new_content:
            print(f"Updating {pyproject_path} to eclipse-zenoh {zenoh_ver}")
            with open(pyproject_path, "w") as f:
                f.write(new_content)
            # Run uv lock to update uv.lock
            import subprocess

            try:
                subprocess.run(["uv", "lock"], check=True)
                print("✓ Updated uv.lock")
            except Exception as e:
                print(f"Warning: could not run uv lock: {e}")

    # 4. Update docker/Dockerfile
    dockerfile_path = "docker/Dockerfile"
    qemu_ver = versions.get("QEMU_VERSION")
    cmake_ver = versions.get("CMAKE_VERSION")
    if os.path.exists(dockerfile_path):
        with open(dockerfile_path, "r") as f:
            content = f.read()

        new_content = re.sub(r"ARG ZENOH_C_REF=[^\n]+", f"ARG ZENOH_C_REF={zenoh_ver}", content)
        if qemu_ver:
            new_content = re.sub(r"ARG QEMU_REF=v[^\n]+", f"ARG QEMU_REF=v{qemu_ver}", new_content)
        if cmake_ver:
            new_content = re.sub(r"ARG CMAKE_VERSION=[^\n]+", f"ARG CMAKE_VERSION={cmake_ver}", new_content)

        # Also update the comment example
        new_content = re.sub(r"\(no \'v\' prefix, e\.g\. [^\)]+\)", f"(no 'v' prefix, e.g. {zenoh_ver})", new_content)

        # Update new ARGs
        for key in ["DEBIAN_CODENAME", "NODE_VERSION", "PYTHON_VERSION", "ARM_TOOLCHAIN_VERSION", "GEMINI_CLI_VERSION"]:
            val = versions.get(key)
            if val:
                new_content = re.sub(f"ARG {key}=[^\n]+", f"ARG {key}={val}", new_content)

        if content != new_content:
            print(
                f"Updating {dockerfile_path} to ZENOH_C_REF {zenoh_ver}"
                + (f" and QEMU_REF v{qemu_ver}" if qemu_ver else "")
            )
            with open(dockerfile_path, "w") as f:
                f.write(new_content)

    # 4b. Propagate PYTHON_VERSION into ci.yml hardcoded env block
    ci_path = ".github/workflows/ci.yml"
    py_ver = versions.get("PYTHON_VERSION")
    if py_ver and os.path.exists(ci_path):
        with open(ci_path, "r") as f:
            ci_content = f.read()
        new_ci = re.sub(r'(PYTHON_VERSION:\s*")[^"]+(")', rf'\g<1>{py_ver}\g<2>', ci_content)
        if ci_content != new_ci:
            print(f"Updating {ci_path} to PYTHON_VERSION {py_ver}")
            with open(ci_path, "w") as f:
                f.write(new_ci)

    # 5. Update FlatBuffers versions
    flatbuffers_ver = versions.get("FLATBUFFERS_VERSION")
    if flatbuffers_ver:
        # Update requirements.txt
        req_path = "requirements.txt"
        if os.path.exists(req_path):
            with open(req_path, "r") as f:
                req_content = f.read()
            new_req = re.sub(r"flatbuffers==[^\s]+", f"flatbuffers=={flatbuffers_ver}", req_content)
            if req_content != new_req:
                print(f"Updating {req_path} to flatbuffers {flatbuffers_ver}")
                with open(req_path, "w") as f:
                    f.write(new_req)

        # Update pyproject.toml
        pyproject_path = "pyproject.toml"
        if os.path.exists(pyproject_path):
            with open(pyproject_path, "r") as f:
                content = f.read()
            # Handle both "flatbuffers==X" and "flatbuffers>=X"
            new_content = re.sub(r'"flatbuffers[>=]=?[^"]+"', f'"flatbuffers=={flatbuffers_ver}"', content)
            if content != new_content:
                print(f"Updating {pyproject_path} to flatbuffers {flatbuffers_ver}")
                with open(pyproject_path, "w") as f:
                    f.write(new_content)

        # Update docker/Dockerfile
        dockerfile_path = "docker/Dockerfile"
        if os.path.exists(dockerfile_path):
            with open(dockerfile_path, "r") as f:
                df_content = f.read()
            new_df = re.sub(r"ARG FLATBUFFERS_VERSION=[^\n]+", f"ARG FLATBUFFERS_VERSION={flatbuffers_ver}", df_content)
            if df_content != new_df:
                print(f"Updating {dockerfile_path} to FLATBUFFERS_VERSION {flatbuffers_ver}")
                with open(dockerfile_path, "w") as f:
                    f.write(new_df)

        # Update Rust Cargo.toml
        cargo_path = "hw/rust/zenoh-telemetry/Cargo.toml"
        if os.path.exists(cargo_path):
            with open(cargo_path, "r") as f:
                cargo_content = f.read()
            new_cargo = re.sub(r'flatbuffers = "[^"]+"', f'flatbuffers = "{flatbuffers_ver}"', cargo_content)
            if cargo_content != new_cargo:
                print(f"Updating {cargo_path} to flatbuffers {flatbuffers_ver}")
                with open(cargo_path, "w") as f:
                    f.write(new_cargo)


if __name__ == "__main__":
    sync()
