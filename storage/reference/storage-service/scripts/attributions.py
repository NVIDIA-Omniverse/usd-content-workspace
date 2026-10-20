#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

import datetime
import logging
import subprocess
import json
import sys
import typing
import dataclasses
import pathlib
import hashlib
import argparse
import tempfile
import contextlib


logger = logging.getLogger("attributions")


class Package(typing.TypedDict):
    id: str
    name: str
    version: str
    license: str
    repository: str
    manifest_path: str


class DependencyKind(typing.TypedDict):
    kind: str | None


class Dependency(typing.TypedDict):
    name: str
    pkg: str
    dep_kinds: list[DependencyKind]


class Node(typing.TypedDict):
    id: str
    deps: list[Dependency]


class Resolve(typing.TypedDict):
    nodes: list[Node]


class Manifest(typing.TypedDict):
    packages: list[Package]
    workspace_members: list[str]
    resolve: Resolve


class AttributionsLicense(typing.TypedDict):
    url: str
    text: str | None


class AttributionsPackage(typing.TypedDict):
    name: str
    version: str
    repository: str
    license: str
    license_data: list[AttributionsLicense]


class AttributionsRoot(typing.TypedDict):
    packages: list[AttributionsPackage]
    digest: str


@dataclasses.dataclass(frozen=True)
class PackageMetadata:
    name: str
    version: str
    license: str
    repository: str
    manifest_path: str


@dataclasses.dataclass(frozen=True)
class Repository:
    path: pathlib.Path
    resolve_file_url: typing.Callable[[str], str]


def get_manifest() -> Manifest:
    """Get the cargo manifest for the workspace."""

    with subprocess.Popen(["cargo", "metadata", "--format-version=1"], stdout=subprocess.PIPE) as p:
        manifest = json.load(p.stdout)
        if status := p.wait():
            raise RuntimeError(f"Cargo exited with a non-zero exit code ({status!r}).")

    return manifest


def is_first_party(name: str) -> bool:
    """NVIDIA's own crates from outside this workspace: storage-core and url_normalizer.

    They are attributed through their dependencies rather than listed themselves. Matched
    by name, so the same rule holds whether they come from a registry or, in the
    consolidated GitHub repository, from sibling paths.
    """
    return name == "url_normalizer" or name == "storage-core" or name.startswith("storage-core-")


def collect_dependencies(manifest: Manifest) -> list[PackageMetadata]:
    """Collect dependency metadata from a manifest."""

    packages = manifest["packages"]
    resolve_nodes = manifest["resolve"]["nodes"]
    package_node_id_by_package_id = {p["id"]: idx for idx, p in enumerate(resolve_nodes)}

    visited: set[int] = set()
    dependency_node_ids: set[int] = set()

    def add_dependencies_of(node_id: int):
        if node_id in visited:
            return

        visited.add(node_id)

        logger.info("Collecting package dependencies; pkg=%r", packages[node_id]["name"])

        node = resolve_nodes[node_id]
        for dependency in node["deps"]:
            # We don't want to include workspace members in the list of dependencies and rather include their deps.
            dependency_pkg = dependency["pkg"]
            dependency_node_id = package_node_id_by_package_id[dependency_pkg]
            if dependency_pkg in manifest["workspace_members"]:
                add_dependencies_of(dependency_node_id)
                continue

            # Include dependencies of private packages.
            if is_first_party(packages[dependency_node_id]["name"]):

                add_dependencies_of(dependency_node_id)
                continue

            # We don't want to include dev or build dependencies.
            if all(d["kind"] is not None for d in dependency["dep_kinds"]):
                continue

            dependency_node_ids.add(dependency_node_id)

    # Start with the storage-service and gradually add the rest of the workspace members' dependencies.
    add_dependencies_of(
        next(idx for idx, p in enumerate(packages) if p["name"] == "storage-service")
    )

    metadata = [
        PackageMetadata(
            name=packages[node_id]["name"],
            version=packages[node_id]["version"],
            license=packages[node_id]["license"],
            repository=packages[node_id]["repository"],
            manifest_path=packages[node_id]["manifest_path"],
        )
        for node_id in dependency_node_ids
        if packages[node_id]["id"] not in manifest["workspace_members"]
    ]
    metadata.sort(key=lambda m: m.name)

    return metadata


def get_license_file_candidates(license: str) -> list[tuple[str, list[str]]]:
    """Get file name candidates for the license files."""

    if license == "MIT":
        return [
            ("MIT", ["license"]),
            ("MIT", ["license-mit"]),
        ]
    elif license == "Apache-2.0":
        return [
            ("Apache-2.0", ["license"]),
            ("Apache-2.0", ["license-apache"]),
        ]
    elif license == "CC0-1.0":
        return [
            ("CC0-1.0", ["license"]),
        ]
    elif license == "BSL-1.0":
        return [
            ("BSL-1.0", ["license"]),
        ]
    elif license in ("MIT OR Apache-2.0", "Apache-2.0 OR MIT", "MIT/Apache-2.0", "Apache-2.0 OR ISC OR MIT"):
        return [
            ("MIT OR Apache-2.0", ["license-mit", "license-apache"]),
            ("MIT OR Apache-2.0", ["license"]),
        ]
    elif license == "(MIT OR Apache-2.0) AND Apache-2.0":
        return [
            ("MIT AND Apache-2.0", ["license-mit", "license-apache"]),
            ("MIT AND Apache-2.0", ["license"]),
        ]
    elif license == "CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception":
        return [
            ("CC0-1.0 OR Apache-2.0", ["license_cc0", "license_a2"]),
            ("CC0-1.0 OR Apache-2.0", ["license"]),
        ]
    else:
        raise RuntimeError(f"Unexpected license kind {license!r}.")


def clone_repositories(path: pathlib.Path, repositories: typing.Iterator[str]) -> list[Repository]:
    """Clone repositories and return their fs locations and urls."""

    visited: dict[str, Repository] = {}

    def get_repository_params(repository: str) -> tuple[str, typing.Callable[[str, str], str]]:
        if repository.startswith("https://github.com/"):
            path_segments = repository.removeprefix("https://github.com/").split("/", maxsplit=2)
            if len(path_segments) < 2:
                raise RuntimeError(f"Malfolrmed GitHub repository url {repository!r}.")

            def resolve_address(branch: str, file: str) -> str:
                return f"https://github.com/{path_segments[0]}/{path_segments[1]}/tree/{branch}/{file}"

            return f"https://github.com/{path_segments[0]}/{path_segments[1]}", resolve_address
        elif repository.startswith("https://codeberg.org/"):
            path_segments = repository.removeprefix("https://codeberg.org/").split("/", maxsplit=2)
            if len(path_segments) < 2:
                raise RuntimeError(f"Malfolrmed CodeBerg repository url {repository!r}.")

            def resolve_address(branch: str, file: str) -> str:
                return f"https://codeberg.org/{path_segments[0]}/{path_segments[1]}/src/branch/{branch}/{file}"

            return f"https://codeberg.org/{path_segments[0]}/{path_segments[1]}", resolve_address
        else:
            raise RuntimeError(f"Unknown repository location {repository!r}.")

    def clone_one(repository: str) -> Repository:
        address, resolve_path = get_repository_params(repository)
        if (already_visited := visited.get(address, None)) is not None:
            return already_visited

        folder = "".join((c if c.isalnum() else "_") for c in address)
        target_path = path / folder

        if not target_path.exists():
            logger.info("Cloning a repository; repository=%r, target_folder=%r", repository, target_path)
            if exit_code := subprocess.call(["git", "clone", address, target_path, "--depth=1"]):
                raise RuntimeError(f"Git exited with unexpected code ({exit_code!r}).")

        with subprocess.Popen(["git", "branch", "--show-current"], stdout=subprocess.PIPE, cwd=target_path) as p:
            branch = p.stdout.readline().decode().strip()

            if exit_code := p.wait():
                raise RuntimeError(f"Get exited with unexpected code ({exit_code!r}).")

        logger.debug("Adding repository metadata; path=%r, branch=%r", target_path, branch)
        result = Repository(path=target_path, resolve_file_url=lambda path: resolve_path(branch, path))
        visited[address] = result

        return result

    return [clone_one(repository) for repository in repositories]


def get_package_metadata(package: PackageMetadata, repository: Repository, with_license_text: bool) -> AttributionsPackage:
    """Generate package metadata."""

    logger.debug("Collecting package licenses; package=%r", package.name)

    roots = [repository.path]
    for child in repository.path.iterdir():
        if child.is_dir():
            if child.name.lower() in ("license", "licenses"):
                roots.append(child)

    def create_attributions(license: str, license_data: list[AttributionsLicense]) -> AttributionsPackage:
        return {
            "name": package.name,
            "version": package.version,
            "repository": package.repository,
            "license": license,
            "license_data": license_data,
        }

    candidates = get_license_file_candidates(package.license)
    for root in roots:
        logger.debug("Searching for licenses; path=%r", root)

        for license, file_name_candidates in candidates:
            logger.debug("Collecting licenses; license=%r, candidates=%r", license, file_name_candidates)

            license_data: list[AttributionsLicense] = []
            for file_name_candidate in file_name_candidates:
                file_name_with_ext_candidates = {
                    file_name_candidate,
                    f"{file_name_candidate}.md",
                    f"{file_name_candidate}.txt",
                }

                for child in root.iterdir():
                    if child.is_file() and child.name.lower() in file_name_with_ext_candidates:
                        logger.debug("Found a license file candidate; license=%r, path=%r", license, child)

                        license_text: str | None = None
                        if with_license_text:
                            with open(child, mode="r") as f:
                                license_text = f.read()

                        license_data.append({
                            "url": repository.resolve_file_url(str(child.relative_to(repository.path))),
                            "text": license_text,
                        })

            if len(license_data) == len(file_name_candidates):
                return create_attributions(license, license_data)

            logger.debug("Failed to collect all licenses, trying next candidate.")

    # Fall back to the readme file.
    readme_candidates = {"readme", "readme.md", "readme.txt"}
    logger.debug("Trying readme candidates; candidates=%r", readme_candidates)

    for child in repository.path.iterdir():
        if child.is_file() and child.name.lower() in readme_candidates:
            logger.debug("Found a readme candidate; license=%r, path=%r", license, child)

            return create_attributions(license, [{
                "text": None,
                "url": repository.resolve_file_url(str(child.relative_to(repository.path))),
            }])

    raise RuntimeError(f"Cannot find licenses of {package.name!r}.")


def read_attributions(file: pathlib.Path | None) -> AttributionsRoot:
    """Read the attributions file."""

    if file is not None:
        logger.info("Reading attributions file; file=%r", file)

        with open(file, mode="r") as f:
            return json.load(f)
        
    logger.info("Reading attributions from stdin.")

    return json.load(sys.stdin)


def load_manifest_and_collect_dependencies() -> list[PackageMetadata]:
    """Load cargo metadata and collect project dependencies."""

    logger.info("Loading cargo manifest.")
    manifest = get_manifest()

    logger.info("Collecting workspace dependencies.")
    return collect_dependencies(manifest)


def compute_packages_digest(packages: list[AttributionsPackage]) -> str:
    """Compute the attributions file digest."""

    def iter_fields():
        for package in packages:
            yield package["name"]
            yield package["version"]
            yield package["repository"]
            yield package["license"]

            for license in package["license_data"]:
                yield license["url"]

                if (text := license["text"]) is not None:
                    yield text

    state = hashlib.sha256()
    for field in iter_fields():
        state.update(field.encode())

    return state.hexdigest()


def validate_attributions(file: pathlib.Path | None):
    """Validate the attributions file."""

    attributions = read_attributions(file)
    dependencies = load_manifest_and_collect_dependencies()

    try:
        # Check packages.
        if len(dependencies) != len(attributions["packages"]):
            raise RuntimeError("The list of dependencies has changed.")

        for expected, actual in zip(dependencies, attributions["packages"]):
            if (
                expected.name != actual["name"] or
                expected.version != actual["version"] or
                expected.repository != actual["repository"]
            ):
                raise RuntimeError(f"Dependency {actual['name']} has been changed.")

        # Check hashes.
        digest = compute_packages_digest(attributions["packages"])
        if digest != attributions["digest"]:
            raise RuntimeError(f"Attributions hash mismatch.")

    except (LookupError, TypeError) as ex:
        raise RuntimeError("Attributions file is malformed.") from ex

    logger.info("Attributions file is valid.")


def generate_attributions(temp_dir: str | None):
    """Generate the attributions file."""

    dependencies = load_manifest_and_collect_dependencies()

    with contextlib.ExitStack() as es:
        if temp_dir is None:
            logger.info("Creating a temporary directory.")
            temp_dir = es.enter_context(tempfile.TemporaryDirectory())

        logger.info("Cloning source repositories; temp_dir=%r", temp_dir)
        repositories = clone_repositories(pathlib.Path(temp_dir), (d.repository for d in dependencies))

        logger.info("Collecting package metadata.")
        packages = [
            get_package_metadata(package, repository, True)
            for package, repository in zip(dependencies, repositories)
        ]
        digest = compute_packages_digest(packages)

    logger.info("Dumping attributions file.")
    json.dump({"packages": packages, "digest": digest}, sys.stdout, indent=2)


def render_attributions_md(file: pathlib.Path | None):
    """Render the ATTRIBUTIONS.md file."""

    attributions = read_attributions(file)

    try:
        print("# Third-Party Software Attributions")
        print()
        print("This document contains the licenses for all third-party dependencies used in this project.")
        print()
        print("Generated on:", datetime.datetime.now(datetime.UTC).isoformat("T", "minutes"))

        for package in attributions["packages"]:
            print()
            print("##", package["name"], package["version"])
            print()
            print("**License**:", package["license"])
            print()
            print("**Repository**:", package["repository"])

            for license_file in package["license_data"]:
                print()
                print("### License text")
                print()
                print("```")
                print(license_file["text"])
                print("```")
    except (LookupError, TypeError) as ex:
        raise RuntimeError("Attributions file is malformed.") from ex

    logger.info("Attributions file rendered.")


def render_summary_md(file: pathlib.Path | None):
    """Render a summary markdown document."""

    attributions = read_attributions(file)
    try:
        package_width = 7
        version_width = 7
        license_width = 7
        repository_width = 10
        license_link_width = 12
        for pkg in attributions["packages"]:
            package_width = max(package_width, len(pkg["name"]))
            version_width = max(version_width, len(pkg["version"]))
            license_width = max(license_width, len(pkg["license"]))
            repository_width = max(repository_width, len(pkg["repository"]))

            for data in pkg["license_data"]:
                license_link_width = max(license_link_width, len(data["url"]))

        def formatted_cell(width: int, value: str, fill: str):
            yield value
            yield fill * (width - len(value))

        def print_row(package: str, version: str, license: str, repository: str, license_link: str, fill: str = " "):
            def yield_next():
                yield "|"
                yield from formatted_cell(package_width, package, fill)
                yield "|"
                yield from formatted_cell(version_width, version, fill)
                yield "|"
                yield from formatted_cell(license_width, license, fill)
                yield "|"
                yield from formatted_cell(repository_width, repository, fill)
                yield "|"
                yield from formatted_cell(license_link_width, license_link, fill)
                yield "|"

            print(fill.join(yield_next()))

        print_row("Package", "Version", "License", "Repository", "License Link")
        print_row("", "", "", "", "", fill="-")

        for pkg in attributions["packages"]:
            for idx, data in enumerate(pkg["license_data"]):
                if idx == 0:
                    print_row(pkg["name"], pkg["version"], pkg["license"], pkg["repository"], data["url"])
                else:
                    print_row("", "", "", "", data["url"])

    except (LookupError, TypeError) as ex:
        raise RuntimeError("Attributions file is malformed.") from ex

    logger.info("Summary file rendered.")

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("-v", "--verbose", action="store_true", dest="verbose", help="Enable verbose output.")

    # Generate command parser.
    subcommand = parser.add_subparsers(dest="command", required=True)
    generate_parser = subcommand.add_parser("generate", help="Update the attributions lock file.")
    generate_parser.add_argument("-t", "--temp-dir", dest="temp_dir", help="Temporary directory location.")

    # Validate command parser.
    validate_parser = subcommand.add_parser("validate", help="Validate the attributions lock file.")
    validate_parser.add_argument("-f", "--file", dest="file", type=pathlib.Path, help="Path to the attributions file.")

    # Render command parser.
    render_parser = subcommand.add_parser("render", help="Render the ATTRIBUTIONS.MD file.")
    render_parser.add_argument("-f", "--file", dest="file", type=pathlib.Path, help="Path to the attributions file.")

    # RenderSummary command parser.
    render_summary = subcommand.add_parser("render-summary", help="Render the summary document.")
    render_summary.add_argument("-f", "--file", dest="file", type=pathlib.Path, help="Path to the attributions file.")

    args = parser.parse_args()
    if args.verbose:
        logging.basicConfig(level=logging.DEBUG)
    else:
        logging.basicConfig(level=logging.INFO)

    if args.command == "validate":
        validate_attributions(args.file)
    elif args.command == "generate":
        generate_attributions(args.temp_dir)
    elif args.command == "render":
        render_attributions_md(args.file)
    elif args.command == "render-summary":
        render_summary_md(args.file)

main()
