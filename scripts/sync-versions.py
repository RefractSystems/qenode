#!/usr/bin/env python3
import logging
import re
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


def sync() -> None:
    versions = get_versions()
    zenoh_ver = versions.get("ZENOH_VERSION")
    if not zenoh_ver:
        logger.error("Error: ZENOH_VERSION not found in BUILD_DEPS")
        return

    # 1. Update tools/deterministic_coordinator/Cargo.toml
    for cargo_path in ["tools/deterministic_coordinator/Cargo.toml"]:
        if Path(cargo_path).exists():
            with Path(cargo_path).open() as f:
                content = f.read()
            new_content = re.sub(r'zenoh = "[^"]+"', f'zenoh = "{zenoh_ver}"', content)
            if content != new_content:
                logger.info(f"Updating {cargo_path} to zenoh {zenoh_ver}")
                with Path(cargo_path).open("w") as f:
                    f.write(new_content)

    # 1.5 Update root Cargo.toml (workspace)
    hw_cargo_path = "Cargo.toml"
    if Path(hw_cargo_path).exists():
        with Path(hw_cargo_path).open() as f:
            content = f.read()
        new_content = re.sub(r'zenoh = "[^"]+"', f'zenoh = "{zenoh_ver}"', content)
        if content != new_content:
            logger.info(f"Updating {hw_cargo_path} to zenoh {zenoh_ver}")
            with Path(hw_cargo_path).open("w") as f:
                f.write(new_content)

    # 2. Update worlds/pendulum.yml
    pendulum_path = "worlds/pendulum.yml"
    if Path(pendulum_path).exists():
        with Path(pendulum_path).open() as f:
            content = f.read()
        new_content = re.sub(
            r"uv pip install eclipse-zenoh==[^\s]+", f"uv pip install eclipse-zenoh=={zenoh_ver}", content
        )
        if content != new_content:
            logger.info(f"Updating {pendulum_path} to eclipse-zenoh {zenoh_ver}")
            with Path(pendulum_path).open("w") as f:
                f.write(new_content)

    # 3. Update requirements.txt
    req_path = "requirements.txt"
    if Path(req_path).exists():
        with Path(req_path).open() as f:
            content = f.read()
        new_content = re.sub(r"eclipse-zenoh==[^\s]+", f"eclipse-zenoh=={zenoh_ver}", content)
        if content != new_content:
            logger.info(f"Updating {req_path} to eclipse-zenoh {zenoh_ver}")
            with Path(req_path).open("w") as f:
                f.write(new_content)

    # 3.5 Update pyproject.toml
    pyproject_path = "pyproject.toml"
    if Path(pyproject_path).exists():
        with Path(pyproject_path).open() as f:
            content = f.read()
        new_content = re.sub(r'"eclipse-zenoh==[^"]+"', f'"eclipse-zenoh=={zenoh_ver}"', content)
        if content != new_content:
            logger.info(f"Updating {pyproject_path} to eclipse-zenoh {zenoh_ver}")
            with Path(pyproject_path).open("w") as f:
                f.write(new_content)
            # Run uv lock to update uv.lock
            import subprocess

            try:
                import shutil

                uv_path = shutil.which("uv")
                if not uv_path:
                    raise RuntimeError("uv not found")
                subprocess.run([uv_path, "lock"], check=True)
                logger.info("✓ Updated uv.lock")
            except (subprocess.CalledProcessError, FileNotFoundError) as e:
                logger.error(f"Error: could not run uv lock: {e}")

    # 4. Update docker/Dockerfile
    dockerfile_path = "docker/Dockerfile"
    qemu_ver = versions.get("QEMU_VERSION")
    cmake_ver = versions.get("CMAKE_VERSION")
    if Path(dockerfile_path).exists():
        with Path(dockerfile_path).open() as f:
            content = f.read()

        new_content = re.sub(r"ARG ZENOH_C_REF=[^\n]+", f"ARG ZENOH_C_REF={zenoh_ver}", content)
        if qemu_ver:
            new_content = re.sub(r"ARG QEMU_REF=v[^\n]+", f"ARG QEMU_REF=v{qemu_ver}", new_content)
        if cmake_ver:
            new_content = re.sub(r"ARG CMAKE_VERSION=[^\n]+", f"ARG CMAKE_VERSION={cmake_ver}", new_content)

        # Also update the comment example
        new_content = re.sub(r"\(no \'v\' prefix, e\.g\. [^\)]+\)", f"(no 'v' prefix, e.g. {zenoh_ver})", new_content)

        # Update new ARGs
        for key in [
            "DEBIAN_CODENAME",
            "NODE_VERSION",
            "PYTHON_VERSION",
            "ARM_TOOLCHAIN_VERSION",
            "MDBOOK_VERSION",
            "MDBOOK_MERMAID_VERSION",
            "MDBOOK_PDF_VERSION",
        ]:
            val = versions.get(key)
            if val:
                new_content = re.sub(f"ARG {key}=[^\n]+", f"ARG {key}={val}", new_content)

        if content != new_content:
            logger.info(
                f"Updating {dockerfile_path} to ZENOH_C_REF {zenoh_ver}"
                + (f" and QEMU_REF v{qemu_ver}" if qemu_ver else "")
            )
            with Path(dockerfile_path).open("w") as f:
                f.write(new_content)

    # 4b. Propagate variables into ci workflows hardcoded env block
    py_ver = versions.get("PYTHON_VERSION")
    image_reg = versions.get("VIRTMCU_IMAGE_REGISTRY")
    devenv_img = versions.get("VIRTMCU_DEVENV_IMAGE")
    ci_img = versions.get("VIRTMCU_CI_IMAGE")

    if py_ver or image_reg or devenv_img or ci_img:
        for ci_path in [
            ".github/workflows/ci-main.yml",
            ".github/workflows/ci-pr.yml",
            ".github/workflows/ci-asan.yml",
        ]:
            if Path(ci_path).exists():
                with Path(ci_path).open() as f:
                    ci_content = f.read()
                
                new_ci = ci_content
                if py_ver:
                    new_ci = re.sub(r'(PYTHON_VERSION:\s*")[^"]+(")', rf"\g<1>{py_ver}\g<2>", new_ci)
                
                if image_reg:
                    # Registry is usually the first part, image name lower is the rest
                    reg_parts = image_reg.split("/", 1)
                    if len(reg_parts) == 2:
                        reg_host, reg_path = reg_parts
                        new_ci = re.sub(r'(REGISTRY:\s*)[^\s\n]+', rf"\g<1>{reg_host}", new_ci)
                        new_ci = re.sub(r'(IMAGE_NAME_LOWER:\s*)[^\s\n]+', rf"\g<1>{reg_path}", new_ci)

                if devenv_img:
                    new_ci = re.sub(r'(VIRTMCU_DEVENV_IMAGE:\s*)[^\s\n]+', rf"\g<1>{devenv_img}", new_ci)
                
                if ci_img:
                    new_ci = re.sub(r'(VIRTMCU_CI_IMAGE:\s*)[^\s\n]+', rf"\g<1>{ci_img}", new_ci)

                if ci_content != new_ci:
                    logger.info(f"Updating {ci_path} env variables")
                    with Path(ci_path).open("w") as f:
                        f.write(new_ci)

    # 5. Update FlatBuffers versions
    flatbuffers_ver = versions.get("FLATBUFFERS_VERSION")
    if flatbuffers_ver:
        # Update requirements.txt
        req_path = "requirements.txt"
        if Path(req_path).exists():
            with Path(req_path).open() as f:
                req_content = f.read()
            new_req = re.sub(r"flatbuffers==[^\s]+", f"flatbuffers=={flatbuffers_ver}", req_content)
            if req_content != new_req:
                logger.info(f"Updating {req_path} to flatbuffers {flatbuffers_ver}")
                with Path(req_path).open("w") as f:
                    f.write(new_req)

        # Update pyproject.toml
        pyproject_path = "pyproject.toml"
        if Path(pyproject_path).exists():
            with Path(pyproject_path).open() as f:
                content = f.read()
            # Handle both "flatbuffers==X" and "flatbuffers>=X"
            new_content = re.sub(r'"flatbuffers[>=]=?[^"]+"', f'"flatbuffers=={flatbuffers_ver}"', content)
            if content != new_content:
                logger.info(f"Updating {pyproject_path} to flatbuffers {flatbuffers_ver}")
                with Path(pyproject_path).open("w") as f:
                    f.write(new_content)

        # Update docker/Dockerfile
        dockerfile_path = "docker/Dockerfile"
        flatcc_ver = versions.get("FLATCC_VERSION")
        if Path(dockerfile_path).exists():
            with Path(dockerfile_path).open() as f:
                df_content = f.read()
            new_df = re.sub(r"ARG FLATBUFFERS_VERSION=[^\n]+", f"ARG FLATBUFFERS_VERSION={flatbuffers_ver}", df_content)
            if flatcc_ver:
                new_df = re.sub(r"ARG FLATCC_VERSION=[^\n]+", f"ARG FLATCC_VERSION={flatcc_ver}", new_df)
            if df_content != new_df:
                logger.info(
                    f"Updating {dockerfile_path} to FLATBUFFERS_VERSION {flatbuffers_ver} and FLATCC_VERSION {flatcc_ver}"
                )
                with Path(dockerfile_path).open("w") as f:
                    f.write(new_df)

        # Update root Cargo.toml (workspace)
        cargo_path = "Cargo.toml"
        if Path(cargo_path).exists():
            with Path(cargo_path).open() as f:
                cargo_content = f.read()
            new_cargo = re.sub(r'flatbuffers = "[^"]+"', f'flatbuffers = "{flatbuffers_ver}"', cargo_content)
            if cargo_content != new_cargo:
                logger.info(f"Updating {cargo_path} to flatbuffers {flatbuffers_ver}")
                with Path(cargo_path).open("w") as f:
                    f.write(new_cargo)

    # 6. Update Pytest versions
    pytest_ver = versions.get("PYTEST_VERSION")
    pytest_asyncio_ver = versions.get("PYTEST_ASYNCIO_VERSION")
    if pytest_ver and pytest_asyncio_ver:
        # Update requirements.txt
        req_path = "requirements.txt"
        if Path(req_path).exists():
            with Path(req_path).open() as f:
                req_content = f.read()
            new_req = re.sub(r"pytest==[^\s]+", f"pytest=={pytest_ver}", req_content)
            new_req = re.sub(r"pytest-asyncio[>=]=?[^\s]+", f"pytest-asyncio=={pytest_asyncio_ver}", new_req)
            if req_content != new_req:
                logger.info(f"Updating {req_path} to pytest {pytest_ver} and pytest-asyncio {pytest_asyncio_ver}")
                with Path(req_path).open("w") as f:
                    f.write(new_req)

        # Update pyproject.toml
        pyproject_path = "pyproject.toml"
        if Path(pyproject_path).exists():
            with Path(pyproject_path).open() as f:
                content = f.read()
            new_content = re.sub(r'"pytest==[^"]+"', f'"pytest=={pytest_ver}"', content)
            new_content = re.sub(r'"pytest-asyncio[>=]=?[^"]+"', f'"pytest-asyncio=={pytest_asyncio_ver}"', new_content)
            if content != new_content:
                logger.info(f"Updating {pyproject_path} to pytest {pytest_ver} and pytest-asyncio {pytest_asyncio_ver}")
                with Path(pyproject_path).open("w") as f:
                    f.write(new_content)

    # 7. Update devcontainer post-create.sh
    post_create_path = ".devcontainer/post-create.sh"
    mdbook_ver = versions.get("MDBOOK_VERSION")
    mdbook_mermaid_ver = versions.get("MDBOOK_MERMAID_VERSION")
    mdbook_pdf_ver = versions.get("MDBOOK_PDF_VERSION")
    if Path(post_create_path).exists():
        with Path(post_create_path).open() as f:
            content = f.read()

        new_content = content
        if mdbook_ver:
            new_content = re.sub(
                r"download/v[0-9\.]+/mdbook-v[0-9\.]+", f"download/v{mdbook_ver}/mdbook-v{mdbook_ver}", new_content
            )
        if mdbook_mermaid_ver:
            new_content = re.sub(
                r"download/v[0-9\.]+/mdbook-mermaid-v[0-9\.]+",
                f"download/v{mdbook_mermaid_ver}/mdbook-mermaid-v{mdbook_mermaid_ver}",
                new_content,
            )
        if mdbook_pdf_ver:
            new_content = re.sub(
                r"download/v[0-9\.]+/mdbook-pdf-v[0-9\.]+",
                f"download/v{mdbook_pdf_ver}/mdbook-pdf-v{mdbook_pdf_ver}",
                new_content,
            )

        if content != new_content:
            logger.info(
                f"Updating {post_create_path} to mdbook v{mdbook_ver}, mdbook-mermaid v{mdbook_mermaid_ver}, and mdbook-pdf v{mdbook_pdf_ver}"
            )
            with Path(post_create_path).open("w") as f:
                f.write(new_content)

    # 8. Update devcontainer.json image registry
    devcontainer_json_path = ".devcontainer/devcontainer.json"
    image_reg = versions.get("VIRTMCU_IMAGE_REGISTRY")
    devenv_img = versions.get("VIRTMCU_DEVENV_IMAGE")
    
    if image_reg and devenv_img and Path(devcontainer_json_path).exists():
        with Path(devcontainer_json_path).open() as f:
            content = f.read()
        
        # Matches "image": "..."
        new_content = re.sub(
            r'("image":\s*")[^"]+(")',
            rf"\g<1>{image_reg}/{devenv_img}:latest\g<2>",
            content
        )
        if content != new_content:
            logger.info(f"Updating {devcontainer_json_path} image to {image_reg}/{devenv_img}:latest")
            with Path(devcontainer_json_path).open("w") as f:
                f.write(new_content)


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    sync()
