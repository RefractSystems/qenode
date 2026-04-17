#!/usr/bin/env python3
import os
import re
import sys

def get_versions():
    versions = {}
    with open("VERSIONS", "r") as f:
        for line in f:
            if "=" in line and not line.startswith("#"):
                key, value = line.strip().split("=")
                versions[key] = value
    return versions

def check():
    versions = get_versions()
    errors = []

    # 1. Check docker/Dockerfile
    dockerfile_path = "docker/Dockerfile"
    if os.path.exists(dockerfile_path):
        with open(dockerfile_path, "r") as f:
            content = f.read()
        
        mappings = {
            "QEMU_VERSION": r"ARG QEMU_REF=v([^ \n]+)",
            "ZENOH_VERSION": r"ARG ZENOH_C_REF=([^ \n]+)",
            "CMAKE_VERSION": r"ARG CMAKE_VERSION=([^ \n]+)",
            "RUST_VERSION": r"ARG RUST_VERSION=([^ \n]+)",
            "FLATBUFFERS_VERSION": r"ARG FLATBUFFERS_VERSION=([^ \n]+)",
            "DEBIAN_CODENAME": r"ARG DEBIAN_CODENAME=([^ \n]+)",
            "NODE_VERSION": r"ARG NODE_VERSION=([^ \n]+)",
            "PYTHON_VERSION": r"ARG PYTHON_VERSION=([^ \n]+)",
            "ARM_TOOLCHAIN_VERSION": r"ARG ARM_TOOLCHAIN_VERSION=([^ \n]+)",
            "GEMINI_CLI_VERSION": r"ARG GEMINI_CLI_VERSION=([^ \n]+)",
        }

        for key, pattern in mappings.items():
            expected = versions.get(key)
            if not expected:
                continue
            match = re.search(pattern, content)
            if not match:
                errors.append(f"{dockerfile_path}: Could not find ARG for {key}")
            elif match.group(1) != expected:
                errors.append(f"{dockerfile_path}: {key} mismatch. Expected {expected}, found {match.group(1)}")

    # 2. Check pyproject.toml
    pyproject_path = "pyproject.toml"
    if os.path.exists(pyproject_path):
        with open(pyproject_path, "r") as f:
            content = f.read()
        
        zenoh_ver = versions.get("ZENOH_VERSION")
        if zenoh_ver:
            match = re.search(r'"eclipse-zenoh==([^"]+)"', content)
            if not match:
                errors.append(f"{pyproject_path}: Could not find eclipse-zenoh dependency")
            elif match.group(1) != zenoh_ver:
                errors.append(f"{pyproject_path}: eclipse-zenoh mismatch. Expected {zenoh_ver}, found {match.group(1)}")

        fb_ver = versions.get("FLATBUFFERS_VERSION")
        if fb_ver:
            match = re.search(r'"flatbuffers==([^"]+)"', content)
            if not match:
                errors.append(f"{pyproject_path}: Could not find flatbuffers dependency (exact version)")
            elif match.group(1) != fb_ver:
                errors.append(f"{pyproject_path}: flatbuffers mismatch. Expected {fb_ver}, found {match.group(1)}")

    # 3. Check requirements.txt
    req_path = "requirements.txt"
    if os.path.exists(req_path):
        with open(req_path, "r") as f:
            content = f.read()
        
        zenoh_ver = versions.get("ZENOH_VERSION")
        if zenoh_ver:
            match = re.search(r"eclipse-zenoh==([^ \n]+)", content)
            if not match:
                errors.append(f"{req_path}: Could not find eclipse-zenoh dependency")
            elif match.group(1) != zenoh_ver:
                errors.append(f"{req_path}: eclipse-zenoh mismatch. Expected {zenoh_ver}, found {match.group(1)}")

        fb_ver = versions.get("FLATBUFFERS_VERSION")
        if fb_ver:
            match = re.search(r"flatbuffers==([^ \n]+)", content)
            if not match:
                errors.append(f"{req_path}: Could not find flatbuffers dependency")
            elif match.group(1) != fb_ver:
                errors.append(f"{req_path}: flatbuffers mismatch. Expected {fb_ver}, found {match.group(1)}")

    # 4. Check ci.yml hardcoded PYTHON_VERSION matches VERSIONS
    ci_path = ".github/workflows/ci.yml"
    if os.path.exists(ci_path):
        with open(ci_path, "r") as f:
            content = f.read()
        py_ver = versions.get("PYTHON_VERSION")
        if py_ver:
            match = re.search(r'PYTHON_VERSION:\s*"([^"]+)"', content)
            if not match:
                errors.append(f"{ci_path}: Could not find hardcoded PYTHON_VERSION env var")
            elif match.group(1) != py_ver:
                errors.append(f"{ci_path}: PYTHON_VERSION mismatch. Expected {py_ver}, found {match.group(1)}")

    if errors:
        print("Version check FAILED:")
        for err in errors:
            print(f"  - {err}")
        print("\nRun 'make sync-versions' to fix.")
        sys.exit(1)
    else:
        print("Version check PASSED")

if __name__ == "__main__":
    check()
