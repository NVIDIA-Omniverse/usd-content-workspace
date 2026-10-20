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

"""Unit tests for `scripts/osrb-oss-inventory.py`.

The script's output is a compliance attestation to a review board, and until these
tests existed nothing exercised the lines that produce it. Its own docstring argues
that "a hand-built table is correct once and then rots" -- equally true of a
generator nobody tests.

Everything here is pure: no `cargo`, no `glab`, no network, no filesystem. That is
deliberate rather than a limitation. The risk surface worth pinning is licence
expression parsing and risk tiering, and both are reachable without a dependency
graph. The two tiers that matter most -- `Needs OSRB review` and `Critical (license
not detected)` -- fire on zero of the 404 real components, so a test is the only
thing that has ever run them.

Written against stdlib `unittest` rather than pytest, deliberately: the repo carries
no Python test dependency and `openspec/specs/ci-rust-builder/spec.md` forbids network
bootstrap inside jobs, so a suite needing `pip install` could not run in CI without an
image change. `python3 -m unittest discover -s tests/scripts` needs nothing, and
pytest runs these classes unchanged for anyone who prefers it.

Loaded through `importlib` because the script's filename is hyphenated and so cannot
be imported as a module name.
"""

import importlib.util
import pathlib
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[2] / "scripts" / "osrb-oss-inventory.py"


def _load():
    spec = importlib.util.spec_from_file_location("osrb_oss_inventory", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


osrb = _load()


class AllowlistCase(unittest.TestCase):
    """A stand-in allowlist, casefolded the way `main` casefolds the real one."""

    def setUp(self):
        self.low = {
            osrb.normalize(entry).casefold()
            for entry in [
                "MIT",
                "Apache-2.0",
                "BSD-3-Clause",
                "Apache-2.0 WITH LLVM-exception",
            ]
        }


class TestNormalize(unittest.TestCase):
    """Cargo's `license` field predates SPDX, so the input is not uniform."""

    def test_slash_is_disjunction(self):
        self.assertEqual(osrb.normalize("MIT/Apache-2.0"), "MIT OR Apache-2.0")

    def test_spaced_slash_is_disjunction(self):
        self.assertEqual(osrb.normalize("Apache-2.0 / MIT"), "Apache-2.0 OR MIT")

    def test_redundant_outer_parens_are_stripped(self):
        self.assertEqual(osrb.normalize("(MIT)"), "MIT")
        self.assertEqual(osrb.normalize("((MIT))"), "MIT")

    def test_separate_groups_are_not_unwrapped(self):
        """The regression this pins: `(MIT) AND (Apache-2.0)`.

        A strip conditioned only on the first and last characters produced
        `MIT) AND (Apache-2.0`, which then classified as needing OSRB review -- the
        safe direction, and still an answer nobody could have explained.
        """
        self.assertEqual(
            osrb.normalize("(MIT) AND (Apache-2.0)"), "(MIT) AND (Apache-2.0)"
        )

    def test_inner_group_survives(self):
        self.assertEqual(
            osrb.normalize("(MIT OR Apache-2.0) AND BSD-3-Clause"),
            "(MIT OR Apache-2.0) AND BSD-3-Clause",
        )

    def test_operator_case_is_canonicalised(self):
        """Without this the AND / OR splits cannot find their separator, so
        case-insensitive identifier matching would still fail on the very expression
        that needed it."""
        self.assertEqual(osrb.normalize("mit or apache-2.0"), "mit OR apache-2.0")
        self.assertEqual(osrb.normalize("mit and apache-2.0"), "mit AND apache-2.0")

    def test_whitespace_is_collapsed(self):
        self.assertEqual(
            osrb.normalize("  MIT   AND    Apache-2.0 "), "MIT AND Apache-2.0"
        )


class TestIsLowRisk(AllowlistCase):
    def test_single_allowlisted_term(self):
        self.assertTrue(osrb.is_low_risk("MIT", self.low))

    def test_or_needs_only_one_low_risk_term(self):
        """NVIDIA may elect either, so one permissive option is enough."""
        self.assertTrue(osrb.is_low_risk("MIT OR GPL-3.0", self.low))

    def test_and_requires_every_term(self):
        self.assertTrue(osrb.is_low_risk("MIT AND Apache-2.0", self.low))
        self.assertFalse(osrb.is_low_risk("MIT AND GPL-3.0", self.low))

    def test_copyleft_alone_is_not_low_risk(self):
        self.assertFalse(osrb.is_low_risk("GPL-3.0", self.low))
        self.assertFalse(osrb.is_low_risk("AGPL-3.0", self.low))

    def test_nested_expression(self):
        self.assertTrue(
            osrb.is_low_risk("(MIT OR Apache-2.0) AND BSD-3-Clause", self.low)
        )
        self.assertFalse(osrb.is_low_risk("(MIT OR Apache-2.0) AND GPL-3.0", self.low))

    def test_separate_groups_are_evaluated_not_mangled(self):
        """The user-visible half of the `normalize` paren fix."""
        self.assertTrue(osrb.is_low_risk("(MIT) AND (Apache-2.0)", self.low))

    def test_identifier_matching_is_case_insensitive(self):
        self.assertTrue(osrb.is_low_risk("mit", self.low))
        self.assertTrue(osrb.is_low_risk("mit or apache-2.0", self.low))

    def test_slash_form_is_low_risk(self):
        self.assertTrue(osrb.is_low_risk("MIT/Apache-2.0", self.low))

    def test_unknown_identifier_is_not_low_risk(self):
        self.assertFalse(osrb.is_low_risk("NVIDIA-Proprietary", self.low))


class TestClassifyRisk(AllowlistCase):
    """The two tiers that have never run against the real dependency set."""

    def test_allowlisted_expression_is_low(self):
        self.assertEqual(
            osrb.classify_risk({"license": "MIT OR Apache-2.0"}, self.low), "Low"
        )

    def test_unrecognised_expression_needs_review(self):
        self.assertEqual(
            osrb.classify_risk({"license": "GPL-3.0"}, self.low), "Needs OSRB review"
        )

    def test_no_licence_metadata_at_all_is_critical(self):
        self.assertEqual(
            osrb.classify_risk({}, self.low), "Critical (license not detected)"
        )
        self.assertEqual(
            osrb.classify_risk({"license": ""}, self.low),
            "Critical (license not detected)",
        )

    def test_licence_file_without_expression_is_review_not_critical(self):
        """Critical is the one tier that mandates an OSRB bug, so a crate that used
        Cargo's other licence field must not land there for that reason alone."""
        self.assertEqual(
            osrb.classify_risk({"license_file": "LICENSE"}, self.low),
            "Needs OSRB review",
        )

    def test_expression_wins_over_licence_file(self):
        self.assertEqual(
            osrb.classify_risk({"license": "MIT", "license_file": "LICENSE"}, self.low),
            "Low",
        )


class TestReportedLicense(unittest.TestCase):
    """One function because two consumers disagreed.

    The CSV row said `DECLARED IN LICENSE FILE` for a `license_file`-only crate while
    the stderr review summary said `NOT DETECTED` for the same crate -- two
    conditionals kept in step by hand, until they were not.
    """

    def test_an_expression_is_reported_as_itself(self):
        self.assertEqual(
            osrb.reported_license({"license": "MIT OR Apache-2.0"}), "MIT OR Apache-2.0"
        )

    def test_a_licence_file_is_named_rather_than_called_undetected(self):
        self.assertEqual(
            osrb.reported_license({"license_file": "LICENSE"}),
            "DECLARED IN LICENSE FILE",
        )

    def test_no_licence_metadata_is_undetected(self):
        self.assertEqual(osrb.reported_license({}), "NOT DETECTED")
        self.assertEqual(osrb.reported_license({"license": ""}), "NOT DETECTED")

    def test_it_agrees_with_the_risk_tier_on_every_shape(self):
        """The property the divergence broke: whatever `classify_risk` decides, the
        reported licence must not contradict it. A crate routed to review for having a
        licence file must not also be described as having no licence."""
        low = {"mit"}
        for pkg in ({"license": "MIT"}, {"license_file": "LICENSE"}, {}):
            reported = osrb.reported_license(pkg)
            critical = osrb.classify_risk(pkg, low) == "Critical (license not detected)"
            self.assertEqual(
                reported == "NOT DETECTED",
                critical,
                f"{pkg} reported {reported!r} against critical={critical}",
            )


class TestLicenseEvidence(unittest.TestCase):
    """Only the behaviours 8.15-8.17 are not about to change.

    The `MONOREPO_LICENSE_URLS` precedence is deliberately left unpinned: 8.15 exists
    to change it, because the map is consulted before the `repository` fallback and so
    shadows it, and a test asserting today's order would have to be rewritten by the
    change that fixes it. What is pinned here is the fallback chain below that map,
    which 8.15 does not touch.
    """

    def test_repository_is_used_when_present(self):
        link, evidence = osrb.license_evidence(
            {"name": "some-crate", "version": "1.0.0", "repository": "https://example.test/x"},
            None,
        )
        self.assertEqual(link, "https://example.test/x")
        self.assertEqual(evidence, "see upstream repository")

    def test_crates_io_is_the_last_resort(self):
        """A few crates publish neither a licence file nor a repository, and a blank
        cell would read as an oversight in the OSRB attachment."""
        link, evidence = osrb.license_evidence(
            {"name": "some-crate", "version": "1.0.0"}, None
        )
        self.assertEqual(link, "https://crates.io/crates/some-crate/1.0.0")
        self.assertIn("no license file published", evidence)

    def test_vendor_dir_reports_the_file_actually_present(self):
        """The `--vendor-dir` branch, which produces the evidence the docstring
        foregrounds and which `docs/oss-compliance.md` does not currently invoke --
        the subject of 8.16."""
        import tempfile

        with tempfile.TemporaryDirectory() as vendor:
            crate = pathlib.Path(vendor) / "some-crate-1.0.0"
            crate.mkdir()
            (crate / "LICENSE-MIT").write_text("...")
            (crate / "LICENSE-APACHE").write_text("...")
            (crate / "src").mkdir()

            link, evidence = osrb.license_evidence(
                {"name": "some-crate", "version": "1.0.0", "repository": "https://example.test/x"},
                vendor,
            )

        # Sorted, so the reported file is deterministic across filesystems.
        self.assertEqual(link, "vendor:some-crate-1.0.0/LICENSE-APACHE")
        self.assertEqual(evidence, "LICENSE-APACHE; LICENSE-MIT")

    def test_vendor_dir_falls_through_when_the_crate_is_absent(self):
        import tempfile

        with tempfile.TemporaryDirectory() as vendor:
            link, _ = osrb.license_evidence(
                {"name": "absent", "version": "1.0.0", "repository": "https://example.test/x"},
                vendor,
            )
        self.assertEqual(link, "https://example.test/x")


class TestValidateAllowlist(unittest.TestCase):
    """A missing or malformed allowlist must stop the run rather than silently
    classify everything as needing review -- which would put 404 components in front
    of the review board for no reason."""

    def test_accepts_a_well_formed_allowlist(self):
        data = {"low_risk": ["MIT", "Apache-2.0"]}
        self.assertEqual(osrb.validate_allowlist(data, "test"), data)

    def test_rejects_a_non_object(self):
        with self.assertRaises(SystemExit):
            osrb.validate_allowlist(["MIT"], "test")

    def test_rejects_a_missing_low_risk_key(self):
        with self.assertRaises(SystemExit):
            osrb.validate_allowlist({"high_risk": ["GPL-3.0"]}, "test")

    def test_rejects_an_empty_low_risk_list(self):
        with self.assertRaises(SystemExit):
            osrb.validate_allowlist({"low_risk": []}, "test")

    def test_rejects_a_low_risk_string_rather_than_a_list(self):
        """Truthy, and iterating it yields single characters -- a low-risk set of
        `{"A", "p", "c", ...}` that no real expression matches. It fails safe and
        silently, which costs whoever has to work out that all 404 findings are an
        artifact of a malformed file."""
        with self.assertRaises(SystemExit):
            osrb.validate_allowlist({"low_risk": "MIT"}, "test")

    def test_rejects_blank_entries(self):
        with self.assertRaises(SystemExit):
            osrb.validate_allowlist({"low_risk": [""]}, "test")
        with self.assertRaises(SystemExit):
            osrb.validate_allowlist({"low_risk": ["   "]}, "test")

    def test_rejects_non_string_entries(self):
        with self.assertRaises(SystemExit):
            osrb.validate_allowlist({"low_risk": [1]}, "test")
        with self.assertRaises(SystemExit):
            osrb.validate_allowlist({"low_risk": ["MIT", None]}, "test")


class TestSplitTop(unittest.TestCase):
    """Splitting has to respect parentheses, or `(A OR B) AND C` loses its grouping."""

    def test_splits_at_depth_zero_only(self):
        """The parens come off the first part because `split_top` normalizes each one,
        and by then they wrap the whole term rather than separating groups."""
        self.assertEqual(
            osrb.split_top("(MIT OR Apache-2.0) AND BSD-3-Clause", " AND "),
            ["MIT OR Apache-2.0", "BSD-3-Clause"],
        )

    def test_does_not_split_inside_parens(self):
        self.assertEqual(
            osrb.split_top("(MIT OR Apache-2.0)", " OR "), ["MIT OR Apache-2.0"]
        )

    def test_single_term_is_returned_whole(self):
        self.assertEqual(osrb.split_top("MIT", " AND "), ["MIT"])


if __name__ == "__main__":
    unittest.main()
