# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Script to generate ATTRIBUTIONS.md file from project dependencies.

Meant to be used from a project that includes this lib, not in this
lib directly.

This script reads all dependencies from the current environment,
fetches their licenses using pip-licenses, and generates an ATTRIBUTIONS.md
file at the root of the project.

The script is idempotent - running it multiple times will only update the
file if there are new or changed dependencies.
"""

import json
import subprocess
import sys
from pathlib import Path
from typing import Any


def get_dependencies_with_licenses() -> list[dict[str, Any]]:
    """
    Get all dependencies with their license information using pip-licenses.

    Returns:
        A list of dictionaries containing Name, Version, License, and
        LicenseText for each dependency.
    """
    try:
        result = subprocess.run(
            [
                "uv",
                "run",
                "pip-licenses",
                "--format",
                "json",
                "--with-license-file",
                "--no-license-path",
            ],
            capture_output=True,
            text=True,
            check=True,
        )
        return json.loads(result.stdout)
    except subprocess.CalledProcessError as e:
        print(f"Error running pip-licenses: {e}", file=sys.stderr)
        print(f"stderr: {e.stderr}", file=sys.stderr)
        sys.exit(1)
    except json.JSONDecodeError as e:
        print(f"Error parsing JSON output: {e}", file=sys.stderr)
        sys.exit(1)


def format_attribution(dependency: dict[str, Any]) -> str:
    """
    Format a single dependency's attribution information.

    Args:
        dependency: Dictionary containing Name, Version, License,
                    and LicenseText keys.

    Returns:
        Formatted markdown string for the dependency.
    """
    name = dependency.get("Name", "Unknown")
    version = dependency.get("Version", "Unknown")
    license_type = dependency.get("License", "UNKNOWN")
    license_text = dependency.get("LicenseText", "No license text available")

    # Clean up license text - ensure it doesn't have extra trailing newlines
    license_text = license_text.rstrip()

    return f"## {name}@{version}\n\n{license_type}\n\n{license_text}\n"


def parse_existing_attributions(content: str) -> dict[str, str]:
    """
    Parse the existing ATTRIBUTIONS.md file to extract package@version keys.

    Args:
        content: The content of the existing ATTRIBUTIONS.md file.

    Returns:
        Dictionary mapping "package@version" to the full attribution text.
    """
    existing = {}
    lines = content.split("\n")

    current_key = None
    current_content = []

    for line in lines:
        if line.startswith("## ") and "@" in line:
            # Save previous entry if it exists
            if current_key:
                existing[current_key] = "\n".join(current_content).strip()

            # Start new entry
            current_key = line[3:].strip()  # Remove "## " prefix
            current_content = [line]
        elif current_key:
            current_content.append(line)

    # Save the last entry
    if current_key:
        existing[current_key] = "\n".join(current_content).strip()

    return existing


def generate_attributions_content(dependencies: list[dict[str, Any]]) -> str:
    """
    Generate the full content of the ATTRIBUTIONS.md file.

    Args:
        dependencies: List of dependency dictionaries.

    Returns:
        The full md content as a string.
    """
    # Sort dependencies by name (case-insensitive)
    sorted_deps = sorted(dependencies, key=lambda x: x["Name"].lower())

    # Build the content
    content_parts = ["# Attributions\n"]

    for dep in sorted_deps:
        content_parts.append(format_attribution(dep))

    return "\n".join(content_parts)


def should_update_file(new_content: str, existing_content: str | None) -> bool:
    """
    Determine if the ATTRIBUTIONS.md file should be updated.

    Args:
        new_content: The newly generated content.
        existing_content: The current content of the file, or None if the
                          file doesn't exist.

    Returns:
        True if the file should be updated, False otherwise.
    """
    if existing_content is None:
        return True

    # Parse both versions to compare package lists
    existing_attrs = parse_existing_attributions(existing_content)
    new_attrs = parse_existing_attributions(new_content)

    # Check if package lists differ
    if set(existing_attrs.keys()) != set(new_attrs.keys()):
        return True

    # Check if any content has changed
    for key in new_attrs:
        if existing_attrs.get(key) != new_attrs.get(key):
            return True

    return False


def main():
    """Main entry point for the script."""
    print("Fetching dependency licenses...")
    dependencies = get_dependencies_with_licenses()

    print(f"Found {len(dependencies)} dependencies")

    # Generate new content
    new_content = generate_attributions_content(dependencies)

    # Check if a file exists and read current content
    # Use current working directory so it works when called from any project
    attributions_file = Path.cwd() / "ATTRIBUTIONS.md"
    existing_content = None
    if attributions_file.exists():
        existing_content = attributions_file.read_text()

    # Determine if update is needed
    if should_update_file(new_content, existing_content):
        attributions_file.write_text(new_content)
        print(f"✓ Updated {attributions_file}")
    else:
        print("✓ No changes detected, ATTRIBUTIONS.md is up to date")


if __name__ == "__main__":
    main()
