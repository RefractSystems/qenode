#!/usr/bin/env python3
import logging
import re
import sys
from pathlib import Path

logger = logging.getLogger(__name__)


def get_versions() -> dict[str, str]:
    versions = {}
    with Path("BUILD_DEPS").open() as f:
        for line in f:
            if "=" in line and not line.startswith("#"):
                key, value = line.strip().split("=")
                versions[key] = value
    return versions


def check() -> None:
    versions = get_versions()
    errors = []

    # 1. Check docker/Dockerfile
    dockerfile_path = "docker/Dockerfile"
    if Path(dockerfile_path).exists():
        with Path(dockerfile_path).open() as f:
            content = f.read()

        mappings = {
            "QEMU_VERSION": r"ARG QEMU_REF=v([^ \n]+)",
            "ZENOH_VERSION": r"ARG ZENOH_C_REF=([^ \n]+)",
            "CMAKE_VERSION": r"ARG CMAKE_VERSION=([^ \n]+)",
            "RUST_VERSION": r"ARG RUST_VERSION=([^ \n]+)",
            "FLATBUFFERS_VERSION": r"ARG FLATBUFFERS_VERSION=([^ \n]+)",
            "FLATCC_VERSION": r"ARG FLATCC_VERSION=([^ \n]+)",
            "DEBIAN_CODENAME": r"ARG DEBIAN_CODENAME=([^ \n]+)",
            "NODE_VERSION": r"ARG NODE_VERSION=([^ \n]+)",
            "PYTHON_VERSION": r"ARG PYTHON_VERSION=([^ \n]+)",
            "ARM_TOOLCHAIN_VERSION": r"ARG ARM_TOOLCHAIN_VERSION=([^ \n]+)",
            "HADOLINT_VERSION": r"ARG HADOLINT_VERSION=([^ \n]+)",
            "ACTIONLINT_VERSION": r"ARG ACTIONLINT_VERSION=([^ \n]+)",
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
    if Path(pyproject_path).exists():
        with Path(pyproject_path).open() as f:
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
    if Path(req_path).exists():
        with Path(req_path).open() as f:
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

    # 4. Check ci workflows hardcoded PYTHON_VERSION matches BUILD_DEPS
    py_ver = versions.get("PYTHON_VERSION")
    if py_ver:
        for ci_path in [
            ".github/workflows/ci-main.yml",
            ".github/workflows/ci-pr.yml",
            ".github/workflows/ci-asan.yml",
        ]:
            if Path(ci_path).exists():
                with Path(ci_path).open() as f:
                    content = f.read()
                match = re.search(r'PYTHON_VERSION:\s*"([^"]+)"', content)
                if not match:
                    errors.append(f"{ci_path}: Could not find hardcoded PYTHON_VERSION env var")
                elif match.group(1) != py_ver:
                    errors.append(f"{ci_path}: PYTHON_VERSION mismatch. Expected {py_ver}, found {match.group(1)}")

    # 5. Check Cargo.toml (workspace)
    cargo_path = "Cargo.toml"
    if Path(cargo_path).exists():
        with Path(cargo_path).open() as f:
            content = f.read()

        zenoh_ver = versions.get("ZENOH_VERSION")
        if zenoh_ver:
            match = re.search(r'zenoh = "([^"]+)"', content)
            if not match:
                errors.append(f"{cargo_path}: Could not find zenoh dependency")
            elif match.group(1) != zenoh_ver:
                errors.append(f"{cargo_path}: zenoh mismatch. Expected {zenoh_ver}, found {match.group(1)}")

        fb_ver = versions.get("FLATBUFFERS_VERSION")
        if fb_ver:
            match = re.search(r'flatbuffers = "([^"]+)"', content)
            if not match:
                errors.append(f"{cargo_path}: Could not find flatbuffers dependency")
            elif match.group(1) != fb_ver:
                errors.append(f"{cargo_path}: flatbuffers mismatch. Expected {fb_ver}, found {match.group(1)}")

    # 6. Check tools/*/Cargo.toml
    for child_cargo in ["tools/deterministic_coordinator/Cargo.toml"]:
        if Path(child_cargo).exists():
            with Path(child_cargo).open() as f:
                content = f.read()
            if zenoh_ver and "zenoh =" in content:
                match = re.search(r'zenoh = "([^"]+)"', content)
                if not match:
                    errors.append(f"{child_cargo}: Could not parse zenoh dependency")
                elif match.group(1) != zenoh_ver:
                    errors.append(f"{child_cargo}: zenoh mismatch. Expected {zenoh_ver}, found {match.group(1)}")

    if errors:
        logger.info("Version check FAILED:")
        for err in errors:
            logger.info(f"  - {err}")
        logger.info("\nRun 'make sync-versions' to fix.")
        sys.exit(1)
    else:
        logger.info("Version check PASSED")


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    check()
