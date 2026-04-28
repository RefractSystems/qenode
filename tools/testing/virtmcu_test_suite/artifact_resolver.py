import os
from pathlib import Path


def get_rust_binary_path(name: str) -> Path:
    """
    Finds the expected path for a built Rust binary across standard workspace locations.
    It returns the path even if the file doesn't exist yet, prioritizing locations
    where it actually exists if multiple are possible.
    """
    workspace_root = Path(__file__).resolve().parent.parent.parent.parent

    if "CARGO_TARGET_DIR" in os.environ:
        p = Path(os.environ["CARGO_TARGET_DIR"]) / f"release/{name}"
        if p.exists():
            return p

    paths = [
        workspace_root / "target/release" / name,
        workspace_root / f"tools/{name}/target/release/{name}",
        # Some tools belong to specific workspaces like cyber_bridge
        workspace_root / f"tools/cyber_bridge/target/release/{name}",
        workspace_root / f"tools/zenoh_coordinator/target/release/{name}",
        workspace_root / f"tools/deterministic_coordinator/target/release/{name}",
    ]

    for p in paths:
        if p.exists():
            return p

    # Fallback to standard target dir if it doesn't exist anywhere
    if "CARGO_TARGET_DIR" in os.environ:
        return Path(os.environ["CARGO_TARGET_DIR"]) / f"release/{name}"
    return workspace_root / "target/release" / name


def resolve_rust_binary(name: str) -> Path:
    """
    Finds a built Rust binary across standard workspace locations.
    Raises FileNotFoundError if it doesn't exist.
    """
    p = get_rust_binary_path(name)
    if not p.exists():
        raise FileNotFoundError(f"Binary {name} not found. Searched path: {p}. Did you run 'cargo build'?")
    return p
