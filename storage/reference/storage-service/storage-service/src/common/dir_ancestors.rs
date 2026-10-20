// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

/// Return the ancestor directory URLs for the given resource URL, from deepest to shallowest.
///
/// "Ancestor" means every prefix that ends at a `/` boundary, starting one segment shallower
/// than the input. The authority (`scheme://host[:port]/`) is never emitted — it's the root
/// of the URL space, not a directory. For S3 buckets this falls out naturally because the
/// bucket sits in the authority (`https://<bucket>.s3.<region>.amazonaws.com/...`); for Azure
/// the container is a path segment and is emitted like any other directory, so subscribers
/// that want container-level signals get one.
///
/// All resource URLs in this codebase use the `https://` scheme — `s3://` is not supported.
///
/// Examples:
/// - `https://b.s3.us-west-2.amazonaws.com/a/b/c/file.usd` →
///   `["https://b.s3.us-west-2.amazonaws.com/a/b/c/", "https://b.s3.us-west-2.amazonaws.com/a/b/", "https://b.s3.us-west-2.amazonaws.com/a/"]`
/// - `https://b.s3.us-west-2.amazonaws.com/file.usd` → `[]`
/// - `https://acct.blob.core.windows.net/container/a/file` →
///   `["https://acct.blob.core.windows.net/container/a/", "https://acct.blob.core.windows.net/container/"]`
pub fn ancestor_dirs(url: &str) -> Vec<String> {
    let Some(scheme_end) = url.find("://") else {
        return Vec::new();
    };
    let authority_start = scheme_end + 3;
    let Some(authority_slash) = url[authority_start..].find('/') else {
        return Vec::new();
    };
    // Absolute position of the '/' that separates authority from the first path segment.
    let prefix_end = authority_start + authority_slash;

    let prefix = &url[..=prefix_end];
    let mut rest = &url[prefix_end + 1..];
    // Trim a single trailing slash so directory-shaped inputs only yield their parents.
    if rest.ends_with('/') {
        rest = &rest[..rest.len() - 1];
    }

    let mut ancestors = Vec::new();
    let mut end = rest.len();
    while let Some(slash) = rest[..end].rfind('/') {
        ancestors.push(format!("{}{}", prefix, &rest[..=slash]));
        end = slash;
    }
    ancestors
}

#[cfg(test)]
mod tests {
    use super::ancestor_dirs;
    use rstest::rstest;

    #[rstest]
    #[case(
        "https://b.s3.us-west-2.amazonaws.com/a/b/c/file.usd",
        &[
            "https://b.s3.us-west-2.amazonaws.com/a/b/c/",
            "https://b.s3.us-west-2.amazonaws.com/a/b/",
            "https://b.s3.us-west-2.amazonaws.com/a/",
        ],
    )]
    #[case(
        "https://b.s3.us-west-2.amazonaws.com/a/file.usd",
        &["https://b.s3.us-west-2.amazonaws.com/a/"],
    )]
    #[case("https://b.s3.us-west-2.amazonaws.com/file.usd", &[])]
    #[case("https://b.s3.us-west-2.amazonaws.com/", &[])]
    #[case("https://b.s3.us-west-2.amazonaws.com", &[])]
    #[case(
        "https://b.s3.us-west-2.amazonaws.com/a/b/c/",
        &[
            "https://b.s3.us-west-2.amazonaws.com/a/b/",
            "https://b.s3.us-west-2.amazonaws.com/a/",
        ],
    )]
    #[case(
        "https://acct.blob.core.windows.net/container/a/b/file",
        &[
            "https://acct.blob.core.windows.net/container/a/b/",
            "https://acct.blob.core.windows.net/container/a/",
            "https://acct.blob.core.windows.net/container/",
        ],
    )]
    #[case(
        "https://acct.blob.core.windows.net/container/file",
        &["https://acct.blob.core.windows.net/container/"],
    )]
    #[case("plain-path-no-scheme", &[])]
    #[case(
        "https://b.s3.us-west-2.amazonaws.com//a/file",
        &[
            "https://b.s3.us-west-2.amazonaws.com//a/",
            "https://b.s3.us-west-2.amazonaws.com//",
        ],
    )]
    #[case("", &[])]
    #[case("https://", &[])]
    #[case(
        "https://host:8080/container/a/file",
        &[
            "https://host:8080/container/a/",
            "https://host:8080/container/",
        ],
    )]
    // Percent-encoded slashes in the path are NOT decoded — they stay as part of a single
    // segment. This matches how the rest of the codebase treats resource URLs.
    #[case(
        "https://b.s3.us-west-2.amazonaws.com/a%2Fb/file",
        &["https://b.s3.us-west-2.amazonaws.com/a%2Fb/"],
    )]
    fn ancestors(#[case] input: &str, #[case] expected: &[&str]) {
        let actual = ancestor_dirs(input);
        let actual_refs: Vec<&str> = actual.iter().map(String::as_str).collect();
        assert_eq!(actual_refs, expected);
    }
}
