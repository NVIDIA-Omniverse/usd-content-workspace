# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Scripts for the notification-common package."""

import argparse
import importlib
import logging
import os
import sys
from pathlib import Path
from subprocess import check_call

import yaml

from notification_common.scripts.header_lint import (
    check_headers,
    repo_root,
    stamp_header,
)

# The generated Discovery CapabilitiesService stubs that ``update_proto`` writes,
# relative to the repo root.
GENERATED_STUBS_DIR = Path("nvidia/omniverse/discovery/capabilities/v2alpha")

# The copyright year stamped on those stubs. Their protos live in the remote buf
# module, not in this repo, so there is no local proto header to take the year
# from. It is the year the stubs were first published here, fixed so that
# regeneration never changes it.
GENERATED_STUBS_COPYRIGHT_YEAR = 2026


def lint() -> None:
    """Run the linter"""
    check_call(["black", "."])
    check_call(["flake8", "."])


def nc_lint() -> None:
    """Internal lint: standard lint plus the Apache-header check.

    Not re-exported to consumer services; the Apache-header gate is specific to
    this repo's published tree and must not run against a consumer's installed
    package.
    """
    lint()
    check_headers()


def start() -> None:
    """Run the FastApi app in uvicorn server for development purposes"""
    check_call(["python", "-um", "app.main"])


def update_proto() -> None:
    """Regenerate the Discovery CapabilitiesService stubs via the buf CLI.

    Internal-maintainer tooling: this command and its ``buf.gen.yaml`` config
    are not runnable from the public GitHub mirror (both require internal
    buf-registry access and ``buf.gen.yaml`` is excluded from publication).
    Pulls the published buf module from the internal buf module registry and
    regenerates the committed
    ``nvidia/omniverse/discovery/capabilities/v2alpha`` namespace tree
    according to ``buf.gen.yaml``. Requires the buf CLI and a buf login to the
    internal buf module registry. Mirrors the event services' ``update_proto``.

    ``buf generate`` writes the stubs with no license header, so each one is then
    stamped with the Apache-2.0 header that ``nc-lint`` requires.
    """
    check_call(["buf", "generate"])
    stub_dir = repo_root() / GENERATED_STUBS_DIR
    for stub in sorted([*stub_dir.glob("*.py"), *stub_dir.glob("*.pyi")]):
        stamp_header(stub, GENERATED_STUBS_COPYRIGHT_YEAR)


def test_internals() -> None:
    test("notification_common")


def test(coverage_dir="app") -> None:
    """Run all tests"""
    check_call(
        [
            "python3",
            "-m",
            "pytest",
            "--junitxml=pytest_report/pytest_report.xml",
            f"--cov={coverage_dir}",
            "--cov-fail-under=95",
            "--cov-report=term-missing",
            "--cov-report=html",
            "--no-header",
            "--random-order",
            "-rp",
            "tests/",
        ]
    )


def setup_logging(verbose: bool = False) -> None:
    """Setup logging configuration."""
    level = logging.DEBUG if verbose else logging.INFO
    logging.basicConfig(
        level=level,
        format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
        handlers=[logging.StreamHandler(sys.stdout)],
    )


def generate_openapi_spec_internal(
    module_path: str,
    app_variable: str,
    output_file: str,
    verbose: bool = False,
) -> None:
    """
    Generate OpenAPI YAML specification from a FastAPI app.

    Args:
        module_path: Python module path to import (e.g., 'app.rest.rest_server')
        app_variable: Name of the FastAPI app variable in the module
        output_file: Path to save the generated OpenAPI YAML file
        verbose: Enable verbose logging
    """
    setup_logging(verbose)
    logger = logging.getLogger(__name__)

    try:
        # Set placeholder environment variables for OpenAPI generation
        # These are needed so that the app can be imported without
        # requiring actual runtime configuration
        logger.info("Setting placeholder environment variables...")
        placeholder_vars = {
            # The default principal source (jwtUnverified) is refused at
            # import without an explicit opt-in, so select the verifying jwt
            # source, whose prerequisites the two placeholders below supply.
            "OMNI_EVENTS_PRINCIPAL_SOURCE": "jwt",
            "OMNI_EVENTS_OPENID_ENABLED": "true",
            "OMNI_EVENTS_OPENID_CONFIGURATION_URI": (
                "https://example.com/.well-known/openid-configuration"
            ),
            "OMNI_EVENTS_JWT_AUDIENCES": "openapi-generation",
        }
        for key, value in placeholder_vars.items():
            if key not in os.environ:
                os.environ[key] = value
                logger.debug(f"Set {key} to placeholder value")

        # Import the FastAPI app
        logger.info(f"Importing FastAPI application from {module_path}...")
        module = importlib.import_module(module_path)
        rest_server = getattr(module, app_variable)

        # Generate OpenAPI schema
        logger.info("Generating OpenAPI schema...")
        openapi_schema = rest_server.openapi()

        # Convert to YAML format
        logger.info("Converting schema to YAML format...")
        yaml_content = yaml.dump(
            openapi_schema,
            default_flow_style=False,
            sort_keys=False,
            allow_unicode=True,
            indent=2,
        )

        # Write to file
        output_path = Path(output_file)
        logger.info(f"Writing OpenAPI YAML to: {output_path.absolute()}")

        # Create directory if it doesn't exist
        output_path.parent.mkdir(parents=True, exist_ok=True)

        # License header to add at the top of the file
        license_header = (
            "# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA "
            "CORPORATION & AFFILIATES. All rights reserved.\n"
            "# SPDX-License-Identifier: Apache-2.0\n"
            "\n"
        )

        with open(output_path, "w", encoding="utf-8") as f:
            f.write(license_header)
            f.write(yaml_content)

        logger.info(
            f"Successfully generated OpenAPI YAML specification at: "
            f"{output_path.absolute()}"
        )

        # Print some stats
        path_count = len(openapi_schema.get("paths", {}))
        logger.info(f"Schema contains {path_count} endpoints")
        if "components" in openapi_schema and "schemas" in openapi_schema["components"]:
            schema_count = len(openapi_schema["components"]["schemas"])
            logger.info(f"Schema contains {schema_count} component schemas")

    except Exception as e:
        logger.error(f"Failed to generate OpenAPI YAML: {e}")
        if verbose:
            logger.exception("Full exception details:")
        sys.exit(1)


def generate_openapi_yaml() -> None:
    """
    Generate OpenAPI YAML specification from a FastAPI application.

    Configuration via environment variables or command-line arguments:
        - FASTAPI_MODULE: Module path (default: 'app.rest.rest_server')
        - FASTAPI_APP_VAR: App variable name (default: 'rest_server')
    """
    parser = argparse.ArgumentParser(
        description="Generate OpenAPI YAML specification from FastAPI app",
        formatter_class=argparse.ArgumentDefaultsHelpFormatter,
    )

    default_module = os.getenv("FASTAPI_MODULE", "app.rest.rest_server")
    default_app_var = os.getenv("FASTAPI_APP_VAR", "rest_server")

    parser.add_argument(
        "-m",
        "--module",
        default=default_module,
        help="Python module path to import the FastAPI app from",
    )

    parser.add_argument(
        "-a",
        "--app",
        default=default_app_var,
        help="Name of the FastAPI app variable in the module",
    )

    parser.add_argument(
        "-o",
        "--output",
        default="openapi.yaml",
        help="Output file path for the generated OpenAPI YAML",
    )

    parser.add_argument(
        "-v", "--verbose", action="store_true", help="Enable verbose logging"
    )

    args = parser.parse_args()

    generate_openapi_spec_internal(args.module, args.app, args.output, args.verbose)
