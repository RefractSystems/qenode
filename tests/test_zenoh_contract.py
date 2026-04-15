import os
import re
import sys


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


def get_libzenohc_version(lib_path):
    """
    Extracts the version string from libzenohc.so binary.
    Zenoh-c embeds its version in the binary, typically as 'zenoh-c <version>'.
    """
    if not os.path.exists(lib_path):
        return None

    try:
        # Read the binary and look for the version pattern
        with open(lib_path, "rb") as f:
            # We don't need to read the whole file if it's huge, but .so are usually okay.
            # To be safe, we read it in chunks or just the whole thing if it's a few MBs.
            data = f.read()

        # Look for 'zenoh-c ' followed by a version number X.Y.Z
        # Some versions might have a 'v' prefix: 'zenoh-c v1.8.0'
        match = re.search(rb"zenoh-c v?(\d+\.\d+\.\d+)", data)
        if match:
            return match.group(1).decode()

        # Fallback: look for just the version pattern if we can't find the prefix
        # This is riskier but might work if the prefix changed.
        # We look for something that looks like a version string at the end of the file
        # where strings are usually stored.
        matches = re.findall(rb"\b(\d+\.\d+\.\d+)\b", data)
        if matches:
            # Return the last one, as it's more likely to be the library version 
            # rather than some internal constant.
            return matches[-1].decode()

    except Exception as e:
        print(f"Error reading {lib_path}: {e}")

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
    lib_path = "/opt/virtmcu/lib/libzenohc.so"
    if not os.path.exists(lib_path):
        # Local dev fallback
        lib_path = "libzenohc.so"

    c_version = get_libzenohc_version(lib_path)

    # 1. Check if we found both versions
    if python_version is None:
        pytest.skip("Could not determine eclipse-zenoh Python package version. Is it installed?")

    if c_version is None:
        pytest.skip(f"Could not find zenoh-c version in {lib_path}. Is libzenohc.so present?")

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

    print(f"Python eclipse-zenoh: {py_ver}")
    print(f"C libzenohc.so:      {c_ver}")

    if py_ver and c_ver:
        py_mm = py_ver.split(".")[:2]
        c_mm = c_ver.split(".")[:2]
        if py_mm == c_mm:
            print("✅ Zenoh version contract verified.")
            sys.exit(0)
        else:
            print(f"❌ Version mismatch: {py_ver} vs {c_ver}")
            sys.exit(1)
    else:
        print("❌ Could not determine one or both versions.")
        sys.exit(1)

