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
        new_content = re.sub(r'pip install eclipse-zenoh==[^ ]+', f'pip install eclipse-zenoh=={zenoh_ver}', content)
        if content != new_content:
            print(f"Updating {pendulum_path} to eclipse-zenoh {zenoh_ver}")
            with open(pendulum_path, "w") as f:
                f.write(new_content)

    # 3. Update requirements.txt
    req_path = "requirements.txt"
    if os.path.exists(req_path):
        with open(req_path, "r") as f:
            content = f.read()
        new_content = re.sub(r'eclipse-zenoh==[^ ]+', f'eclipse-zenoh=={zenoh_ver}', content)
        if content != new_content:
            print(f"Updating {req_path} to eclipse-zenoh {zenoh_ver}")
            with open(req_path, "w") as f:
                f.write(new_content)

if __name__ == "__main__":
    sync()
