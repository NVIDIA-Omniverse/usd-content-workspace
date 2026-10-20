# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Script to build the API archive zip file for NGC distribution."""

import subprocess
import sys
import tomllib
import zipfile
from pathlib import Path


def get_project_info_from_pyproject() -> tuple[str, str, str]:
    """Extract the project name, version, and proto subdirectory from pyproject.toml.

    Returns:
        tuple: (project_name, version, proto_subdir)
               e.g., ("event-consumer-service", "1.2.9", "consumer")
                     or ("event-aggregation-service", "1.2.2", "publisher")
    """
    pyproject_path = Path.cwd() / "pyproject.toml"
    with open(pyproject_path, "rb") as f:
        data = tomllib.load(f)

    project_name = data["project"]["name"]
    version = data["project"]["version"]

    # Derive proto subdirectory from project name
    # event-consumer-service -> consumer
    # event-aggregation-service -> publisher
    if "consumer" in project_name:
        proto_subdir = "consumer"
    elif "aggregation" in project_name:
        proto_subdir = "publisher"
    else:
        raise ValueError(f"Unknown project type: {project_name}")

    return project_name, version, proto_subdir


def get_docs_project_and_version() -> tuple[str, str]:
    """Read the repo_docs project and version from the proto submodule's
    repo.toml.

    The built-docs output lands under
    ``notification_protos/docs/_build/docs/<project>/<version>``, where
    ``<project>`` and ``<version>`` come from the ``[repo_docs]`` section
    of repo.toml. Deriving them here keeps the archive's docs path in sync
    with the docs tooling config instead of hardcoding it — a hardcoded
    path silently drifted out of date and shipped docs-less archives.

    Returns:
        tuple: (docs_project, docs_version)
               e.g., ("notification-api", "1.0.0-beta.2")
    """
    repo_toml_path = Path.cwd() / "notification_protos" / "docs" / "repo.toml"
    with open(repo_toml_path, "rb") as f:
        data = tomllib.load(f)

    repo_docs = data["repo_docs"]
    return repo_docs["project"], repo_docs["version"]


def build_docs():
    """Build the documentation before archiving."""
    base_path = Path.cwd()
    docs_script = base_path / "notification_protos" / "docs" / "repo.sh"

    print("Building documentation...")
    try:
        result = subprocess.run(
            [str(docs_script), "docs"],
            cwd=base_path,
            check=True,
            capture_output=True,
            text=True,
        )
        print("Documentation built successfully")
        if result.stdout:
            print(result.stdout)
    except subprocess.CalledProcessError as e:
        print(f"Error building documentation: {e}", file=sys.stderr)
        if e.stdout:
            print(e.stdout, file=sys.stderr)
        if e.stderr:
            print(e.stderr, file=sys.stderr)
        raise


def create_archive():
    """Create the API archive zip file."""
    project_name, version, proto_subdir = get_project_info_from_pyproject()

    # Create a generic archive name based on project name
    # event-consumer-service -> notifications-consumer-api-{version}.zip
    # event-aggregation-service -> notifications-aggregation-api-{version}.zip
    api_name = project_name.replace("-service", "-api").replace(
        "event-", "notifications-"
    )
    archive_name = f"{api_name}-{version}.zip"

    # Define base paths
    base_path = Path.cwd()
    protos_base = base_path / "notification_protos" / "protos"
    docs_project, docs_version = get_docs_project_and_version()
    docs_build = (
        base_path
        / "notification_protos"
        / "docs"
        / "_build"
        / "docs"
        / docs_project
        / docs_version
    )
    openapi_file = base_path / "openapi.yaml"
    oss_dir = base_path / "oss"
    license_file = base_path / "LICENSE.txt"
    changelog_file = base_path / "CHANGELOG.md"

    # The archive is a documentation deliverable, so a missing or empty
    # docs build is a hard error. Previously this was a soft warning, which
    # let docs-less archives ship to NGC unnoticed when the output path
    # drifted. Fail loudly here, before the zip is created.
    docs_files = (
        [p for p in docs_build.rglob("*") if p.is_file()] if docs_build.exists() else []
    )
    if not docs_files:
        docs_root = docs_build.parent.parent
        available = (
            sorted(str(p.relative_to(docs_root)) for p in docs_root.glob("*/*"))
            if docs_root.exists()
            else []
        )
        raise RuntimeError(
            f"No documentation found at {docs_build}. "
            f"Available doc builds under {docs_root}: {available or 'none'}. "
            "Ensure 'repo.sh docs' ran successfully and that the repo.toml "
            "[repo_docs] project/version still match this path."
        )

    # Create the zip file
    with zipfile.ZipFile(archive_name, "w", zipfile.ZIP_DEFLATED) as zipf:
        # Add the proto files for this service
        service_proto_path = (
            protos_base / "nvidia" / "omniverse" / "notifications" / proto_subdir
        )

        if service_proto_path.exists():
            for proto_file in service_proto_path.rglob("*.proto"):
                # Calculate the archive path relative to protos/
                archive_path = (
                    Path("protos")
                    / "nvidia"
                    / "omniverse"
                    / "notifications"
                    / proto_subdir
                    / proto_file.relative_to(service_proto_path)
                )
                zipf.write(proto_file, archive_path)
                print(f"Added: {archive_path}")
        else:
            print(f"Warning: Proto path not found: {service_proto_path}")

        # Add the built docs (validated above) under "docs" in the archive
        for doc_file in docs_files:
            # Calculate the relative path and put it under docs/
            rel_path = doc_file.relative_to(docs_build)
            archive_path = Path("docs") / rel_path
            zipf.write(doc_file, archive_path)
            print(f"Added: {archive_path}")

        # Add openapi.yaml at openapi/v1beta/openapi.yaml
        if openapi_file.exists():
            archive_path = Path("openapi") / "v1beta" / "openapi.yaml"
            zipf.write(openapi_file, archive_path)
            print(f"Added: {archive_path}")
        else:
            print(f"Warning: OpenAPI file not found: {openapi_file}")

        # Add the oss directory contents to the root of the archive
        if oss_dir.exists():
            for oss_file in oss_dir.rglob("*"):
                if oss_file.is_file():
                    rel_path = oss_file.relative_to(oss_dir)
                    zipf.write(oss_file, rel_path)
                    print(f"Added: {rel_path}")
        else:
            print(f"Warning: OSS directory not found: {oss_dir}")

        # Add LICENSE.txt from the root
        if license_file.exists():
            zipf.write(license_file, Path("LICENSE.txt"))
            print("Added: LICENSE.txt")
        else:
            print(f"Warning: LICENSE.txt file not found: {license_file}")

        # Add CHANGELOG.md from the root
        if changelog_file.exists():
            zipf.write(changelog_file, Path("CHANGELOG.md"))
            print("Added: CHANGELOG.md")
        else:
            print(f"Warning: CHANGELOG.md file not found: {changelog_file}")

    print(f"\nArchive created successfully: {archive_name}")
    return archive_name


def main():
    """Main entry point for the script."""
    try:
        build_docs()
        archive_name = create_archive()
        print(f"Success! Created: {archive_name}")
        return 0
    except Exception as e:
        print(f"Error creating archive: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
