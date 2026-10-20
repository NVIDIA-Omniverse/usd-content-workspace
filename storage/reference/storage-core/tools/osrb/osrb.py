#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Cargo Metadata to Google Spreadsheet Converter

This script processes the output of `cargo metadata --locked --all-features --format-version 1`
and generates a CSV file compatible with Google Spreadsheets for NVIDIA's OSRB project.

Usage:
    cargo metadata --locked --all-features --format-version 1 | python3 osrb.py > osrb.csv
"""

import json
import sys
import csv
from typing import Dict, List, Optional, Any
from urllib.parse import urlparse


def extract_repository_url(package: Dict[str, Any]) -> str:
    """
    Extract repository URL from package metadata.
    Tries multiple sources in order of preference.
    """
    # Check repository field first
    if package.get("repository"):
        return package["repository"]
    
    # Check homepage field
    if package.get("homepage"):
        return package["homepage"]
    
    # Check documentation field
    if package.get("documentation"):
        return package["documentation"]

    return ""


def extract_license(package: Dict[str, Any]) -> str:
    """
    Extract license information from package metadata.
    """
    license_field = package.get("license")
    if license_field:
        return license_field
    
    # Check for license-file if no license field
    if package.get("license_file"):
        return f"Custom (see license_file)"
    
    return "Unknown"


def is_valid_url(url: str) -> bool:
    """
    Check if the URL is valid and accessible.
    """
    if not url:
        return False
    
    try:
        result = urlparse(url)
        return all([result.scheme, result.netloc])
    except Exception:
        raise


def process_cargo_metadata(metadata: Dict[str, Any]) -> List[Dict[str, str]]:
    """
    Process cargo metadata and extract package information.
    """
    packages = metadata.get("packages", [])
    results = []
    
    for package in packages:
        name = package.get("name", "Unknown")
        version = package.get("version", "Unknown")
        license = extract_license(package)
        repository_url = extract_repository_url(package)
        comment = ""
    
        # Create hyperlink format for Google Sheets
        if is_valid_url(repository_url):
            hyperlink = f'=HYPERLINK("{repository_url}","{repository_url}")'
        else:
            hyperlink = repository_url if repository_url else "N/A"
        
        # Cleanup to match spreadsheet options
        match license:
            case '(MIT OR Apache-2.0) AND Unicode-3.0':
                license = 'MIT'
            case '0BSD OR MIT OR Apache-2.0':
                license = 'MIT'
            case 'Apache-2.0':
                license = 'Apache 2.0'
            case 'Apache-2.0 AND BSD-3-Clause':
                license = 'Apache 2.0'
            case 'Apache-2.0 AND ISC':
                license = 'Apache 2.0'
            case 'Apache-2.0 OR MIT':
                license = 'MIT'
            case 'Apache-2.0/MIT':
                license = 'MIT'
            case 'BSD-3-Clause':
                license = 'BSD (any variant)'
            case 'ISC':
                license = 'ISC (Internet Software Consortium)'
            case 'ISC AND (Apache-2.0 OR ISC)':
                license = 'ISC (Internet Software Consortium)'
            case 'ISC AND (Apache-2.0 OR ISC) AND OpenSSL':
                license = 'ISC (Internet Software Consortium)'
            case 'MIT AND BSD-3-Clause':
                license = 'MIT'
            case 'MIT OR Apache-2.0':
                license = 'MIT'
            case 'MIT/Apache-2.0':
                license = 'MIT'
            case 'MPL-2.0':
                license = 'MPL'
            case 'Unicode-3.0':
                license = 'Other (Please describe in Comments)'
                comment = 'Unicode 3.0 https://www.unicode.org/license.txt'
            case 'Unlicense OR MIT':
                license = 'MIT'
            case 'Unlicense/MIT':
                license = 'MIT'
            case 'Zlib':
                license = 'Other (Please describe in Comments)'
                comment = 'Zlib licence (https://github.com/orlp/foldhash?tab=Zlib-1-ov-file#readme)'
            case 'Zlib OR Apache-2.0 OR MIT':
                license = 'MIT'
            case 'LicenseRef-NvidiaProprietary':
                license = 'Nvidia Proprietary License'
            case 'CC0-1.0 OR MIT-0 OR Apache-2.0':
                license = 'MIT'
            case '(Apache-2.0 OR MIT) AND BSD-3-Clause':
                license = 'MIT'
            case 'Apache-2.0 / MIT':
                license = 'MIT'
            case 'Apache-2.0 OR ISC OR MIT':
                license = 'MIT'
            case 'Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT':
                license = 'MIT'
            case 'MIT OR Zlib OR Apache-2.0':
                license = 'MIT'
            case 'MIT OR Apache-2.0 OR LGPL-2.1-or-later':
                license = 'MIT'
            case 'Apache-2.0 OR BSL-1.0':
                license = 'Apache 2.0'
            case 'BSD-2-Clause OR Apache-2.0 OR MIT':
                license = 'MIT'

        # fill in missing license
        if license == "":
            if package.startswith('azure_'):
                license = 'MIT'
            if package.startswith('protoc-'):
                license = 'MIT'
            if package == 'base64-simd':
                license = 'MIT'
            if package == 'crc32c':
                license = 'Apache 2.0'
            if package == 'linkme-impl':
                license = 'MIT'
            if package == 'relative-path':
                license = 'MIT'
            if package == 'valuable':
                license = 'MIT'
            if package == 'vsimd':
                license = 'MIT'
            if package.startswith('winapi-'):
                license = 'MIT'

        results.append({
            "Name": name,
            "Version": version,
            "License": license,
            "URL": hyperlink,
            "Distribution": "Container - Added to Base Container listed in Row 1",
            "Usage": "Static Linking",
            "Comments": comment
        })

    return results


def output_csv(packages: List[Dict[str, str]], output_file=sys.stdout):
    """
    Output packages as CSV format compatible with Google Spreadsheets.
    """
    if not packages:
        print("No packages found in metadata", file=sys.stderr)
        return
    
    fieldnames = ["Name", "Version", "License", "URL", "Distribution", "Usage", "Comments"]
    writer = csv.DictWriter(output_file, fieldnames=fieldnames)
    
    # Write package data
    for package in packages:
        writer.writerow(package)


def main():
    """
    Main function to process cargo metadata from stdin or file.
    """
    try:
        # Read JSON from stdin
        if sys.stdin.isatty():
            print("Error: No input provided. Please pipe cargo metadata output to this script.", file=sys.stderr)
            print("Usage: cargo metadata --locked --all-features --format-version 1 | python3 osrb.py", file=sys.stderr)
            sys.exit(1)
        
        # Parse JSON input
        try:
            metadata = json.load(sys.stdin)
        except json.JSONDecodeError as e:
            print(f"Error: Invalid JSON input - {e}", file=sys.stderr)
            sys.exit(1)
        
        # Process metadata
        packages = process_cargo_metadata(metadata)
        
        # Output CSV
        output_csv(packages)
        
        # Print summary
        print(f"# Processed {len(packages)} packages", file=sys.stderr)
        
    except KeyboardInterrupt:
        print("\nOperation cancelled by user", file=sys.stderr)
        sys.exit(1)
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
