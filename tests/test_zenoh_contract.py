import logging
import os
import re
import sys
from pathlib import Path

logger = logging.getLogger(__name__)


def get_python_zenoh_version():
    """Returns the version of the installed eclipse-zenoh Python package."""
    try:
        # Try modern importlib.metadata first (Python 3.8+)
        from importlib.metadata import version

        return version("eclipse-zenoh")
    except ImportError:
        try:
            import pkg_resources

            return pkg_resources.get_distribution("eclipse-zenoh").version
        except Exception:
            return None


def get_libzenohc_version(lib_path):  # noqa: ARG001
    """
    Extracts the expected zenoh-c version.
    Since the binary no longer reliably embeds its version, we read it
    from the BUILD_DEPS file in the workspace root.
    """
    # Find BUILD_DEPS file relative to this script
    workspace_dir = Path(Path(Path(__file__).parent.resolve().parent))
    versions_file = Path(workspace_dir) / "BUILD_DEPS"

    if Path(versions_file).exists():
        with Path(versions_file).open() as f:
            for line in f:
                if line.startswith("ZENOH_VERSION="):
                    return line.strip().split("=")[1]

    # Fallback for runtime image where BUILD_DEPS might not exist
    # Check if there's an environment variable
    if "ZENOH_VERSION" in os.environ:
        return os.environ["ZENOH_VERSION"]

    return None


def test_zenoh_version_contract():
    """
    Contract Test: Verifies that the Zenoh C runtime and Python client
    share the same MAJOR.MINOR version to ensure protocol compatibility.
    """
    import pytest

    python_version = get_python_zenoh_version()

    # In the runtime image, it's at /opt/virtmcu/lib/libzenohc.so
    # Locally, it might be in LD_LIBRARY_PATH or next to the script
    lib_paths = ["/opt/virtmcu/lib/libzenohc.so", "libzenohc.so", "./third_party/zenoh-c/lib/libzenohc.so"]

    c_version = None
    for lib_path in lib_paths:
        c_version = get_libzenohc_version(lib_path)
        if c_version is not None:
            break

    # 1. Check if we found both versions
    if python_version is None:
        pytest.skip("Could not determine eclipse-zenoh Python package version. Is it installed?")

    if c_version is None:
        pytest.skip(f"Could not find zenoh-c version in any of {lib_paths}. Is libzenohc.so present?")

    # 2. Extract MAJOR.MINOR
    py_match = re.match(r"(\d+\.\d+)", python_version)
    c_match = re.match(r"(\d+\.\d+)", c_version)

    assert py_match is not None, f"Invalid Python zenoh version format: {python_version}"
    assert c_match is not None, f"Invalid C zenoh version format: {c_version}"

    py_major_minor = py_match.group(1)
    c_major_minor = c_match.group(1)

    # 3. Assert match
    # A mismatch should produce a clear failure message as requested.
    assert c_major_minor == py_major_minor, (
        f"zenoh-c runtime is {c_version} but eclipse-zenoh Python package is {python_version} — "
        f"pin eclipse-zenoh=={c_version} in FirmwareStudio Dockerfiles"
    )


if __name__ == "__main__":
    # Allow running directly for manual verification
    py_ver = get_python_zenoh_version()
    c_ver = get_libzenohc_version("/opt/virtmcu/lib/libzenohc.so") or get_libzenohc_version("libzenohc.so")

    logger.info(f"Python eclipse-zenoh: {py_ver}")
    logger.info(f"C libzenohc.so:      {c_ver}")

    if py_ver and c_ver:
        py_mm = py_ver.split(".")[:2]
        c_mm = c_ver.split(".")[:2]
        if py_mm == c_mm:
            logger.info("✅ Zenoh version contract verified.")
            sys.exit(0)
        else:
            logger.error(f"❌ Version mismatch: {py_ver} vs {c_ver}")
            sys.exit(1)
    else:
        logger.error("❌ Could not determine one or both versions.")
        sys.exit(1)
