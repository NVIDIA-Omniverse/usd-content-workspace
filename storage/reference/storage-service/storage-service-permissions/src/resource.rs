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

use std::iter::FusedIterator;

use fluent_uri::{pct_enc::DecodedChunk, Uri};
use percent_encoding::{percent_encode, AsciiSet};
use snafu::{IntoError, Location, Snafu};
use url_escape::NON_ALPHANUMERIC;

use crate::types::ResourceType;

/// For escaping path segments, follow the encodeURIComponent standard. All characters must be encoded except for the
/// following unreserved set: A–Z a–z 0–9 - _ . ! ~ * ' ( ). We also remove the path separator character from the
/// encode set.
const URI_COMPONENT_SET: AsciiSet = NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'/');

/// A url parse/normalization error.
#[derive(Debug, Snafu)]
pub enum ParseError {
    #[snafu(display("url has invalid format"))]
    Parse {
        #[snafu(source)]
        source: fluent_uri::ParseError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("url has no authority"))]
    AuthorityMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },
}

/// A normalized resource url - either a folder url or an object url.
#[derive(Debug)]
pub struct ResourceUrl {
    url: String,
    resource_type: ResourceType,
    path_start: usize,
}

impl ResourceUrl {
    /// Parse the url assuming it being of the specified resource type. Query parameters and fragment will be discarded.
    /// For folder urls, a missing trailing slash will be added to the path component.
    pub fn parse(url: &str, resource_type: Option<ResourceType>) -> Result<Self, ParseError> {
        // Discard query and fragment. We don't want to validate these components.
        let url = url.split_once(['?', '#']).map(|(prefix, _)| prefix).unwrap_or(url);

        let url = Uri::parse(url).map_err(|source| ParseSnafu.into_error(source))?;
        let authority = url.authority().ok_or_else(|| AuthorityMissingSnafu.build())?;

        let mut buffer = String::new();
        buffer.push_str(url.scheme().as_str());
        buffer.push_str("://");
        buffer.push_str(authority.as_str());

        let path_start = buffer.len();

        // We don't want empty characters in the path.
        let path = url.path();
        if path.is_empty() {
            buffer.push('/');
        } else {
            for chunk in path.decode() {
                match chunk {
                    DecodedChunk::PctDecoded(byte) => {
                        for string in percent_encode(&[byte], &URI_COMPONENT_SET) {
                            buffer.push_str(string);
                        }
                    },
                    DecodedChunk::Unencoded(string) => {
                        buffer.push_str(string);
                    },
                }
            }

            if resource_type == Some(ResourceType::Folder) && !buffer.ends_with('/') {
                buffer.push('/');
            }
        }

        Ok(Self {
            resource_type: resource_type.unwrap_or_else(|| {
                if buffer.ends_with('/') {
                    ResourceType::Folder
                } else {
                    ResourceType::Object
                }
            }),
            url: buffer,
            path_start,
        })
    }

    /// Decompose the resource url into its value in the deduced resource type values.
    pub fn into_components(self) -> (String, ResourceType) {
        (self.url, self.resource_type)
    }

    /// Iterate over the resources in the hierarchy in reverse order.
    pub fn iter_hierarchy<'a>(&'a self) -> ResourceHierarchyIter<'a> {
        ResourceHierarchyIter {
            url: &self.url,
            state: match self.resource_type {
                ResourceType::Folder => ResourceHierarchyIterState::FirstFolder(self.path_start),
                ResourceType::Object => ResourceHierarchyIterState::FirstObject(self.path_start),
            },
        }
    }
}

/// An iterator over resources in a hierarchy.
#[derive(Debug)]
pub struct ResourceHierarchyIter<'a> {
    url: &'a str,
    state: ResourceHierarchyIterState,
}

impl<'a> ResourceHierarchyIter<'a> {
    fn advance(&mut self, path_start: usize, path_end: usize) {
        if path_start > path_end {
            self.state = ResourceHierarchyIterState::Exhausted;
        } else if let Some(path_separator_pos) = &self.url[path_start..path_end].rfind('/') {
            self.state = ResourceHierarchyIterState::NextFolder(path_start, path_start + path_separator_pos + 1);
        } else {
            self.state = ResourceHierarchyIterState::Exhausted;
        }
    }
}

impl<'a> Iterator for ResourceHierarchyIter<'a> {
    type Item = (&'a str, ResourceType);

    fn next(&mut self) -> Option<Self::Item> {
        match self.state {
            ResourceHierarchyIterState::FirstObject(path_start) => {
                let result = (self.url, ResourceType::Object);
                self.advance(path_start, self.url.len());

                Some(result)
            },
            ResourceHierarchyIterState::FirstFolder(path_start) => {
                let result = (self.url, ResourceType::Folder);
                self.advance(path_start, self.url.len() - 1);

                Some(result)
            },
            ResourceHierarchyIterState::NextFolder(path_start, path_end) => {
                let result = (&self.url[..path_end], ResourceType::Folder);
                self.advance(path_start, path_end - 1);

                Some(result)
            },
            ResourceHierarchyIterState::Exhausted => None,
        }
    }
}

impl FusedIterator for ResourceHierarchyIter<'_> {}

#[derive(Debug, Clone, Copy)]
enum ResourceHierarchyIterState {
    FirstObject(usize),
    FirstFolder(usize),
    NextFolder(usize, usize),
    Exhausted,
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::{resource::ResourceUrl, types::ResourceType};

    #[rstest]
    #[case(
        "https://hostname/a/b/c",
        ResourceType::Object,
        vec![
            ("https://hostname/a/b/c", ResourceType::Object),
            ("https://hostname/a/b/", ResourceType::Folder),
            ("https://hostname/a/", ResourceType::Folder),
            ("https://hostname/", ResourceType::Folder),
        ]
    )]
    #[case(
        "https://hostname/a/b/",
        ResourceType::Object,
        vec![
            ("https://hostname/a/b/", ResourceType::Object),
            ("https://hostname/a/b/", ResourceType::Folder),
            ("https://hostname/a/", ResourceType::Folder),
            ("https://hostname/", ResourceType::Folder),
        ]
    )]
    #[case(
        "https://hostname/a/b/",
        ResourceType::Folder,
        vec![
            ("https://hostname/a/b/", ResourceType::Folder),
            ("https://hostname/a/", ResourceType::Folder),
            ("https://hostname/", ResourceType::Folder),
        ]
    )]
    #[case(
        "https://hostname/a/b//",
        ResourceType::Folder,
        vec![
            ("https://hostname/a/b//", ResourceType::Folder),
            ("https://hostname/a/b/", ResourceType::Folder),
            ("https://hostname/a/", ResourceType::Folder),
            ("https://hostname/", ResourceType::Folder),
        ]
    )]
    #[case(
        "https://hostname",
        ResourceType::Object,
        vec![
            ("https://hostname/", ResourceType::Object),
            ("https://hostname/", ResourceType::Folder),
        ]
    )]
    #[case(
        "https://hostname",
        ResourceType::Folder,
        vec![
            ("https://hostname/", ResourceType::Folder),
        ]
    )]
    #[case(
        "https://hostname/../a",
        ResourceType::Object,
        vec![
            ("https://hostname/../a", ResourceType::Object),
            ("https://hostname/../", ResourceType::Folder),
            ("https://hostname/", ResourceType::Folder),
        ]
    )]
    #[case(
        "https://hostname/../a",
        ResourceType::Folder,
        vec![
            ("https://hostname/../a/", ResourceType::Folder),
            ("https://hostname/../", ResourceType::Folder),
            ("https://hostname/", ResourceType::Folder),
        ]
    )]
    fn test_resource_hierarchy_iter_produces_a_valid_sequence(
        #[case] url: &str,
        #[case] resource_type: ResourceType,
        #[case] expected_entries: Vec<(&str, ResourceType)>,
    ) {
        let parsed = ResourceUrl::parse(url, Some(resource_type)).expect("url parse failed");

        let entries: Vec<_> = parsed.iter_hierarchy().collect();
        assert_eq!(entries, expected_entries);
    }

    #[rstest]
    #[case("https://myhost.mydomain/a%2fb%2fc", None, "https://myhost.mydomain/a/b/c")]
    #[case(
        "https://myhost.mydomain/%41%2F%42/%43/%61/%62%63",
        None,
        "https://myhost.mydomain/A/B/C/a/bc"
    )]
    #[case("https://myhost.mydomain/%7ba%7d%2f", None, "https://myhost.mydomain/%7Ba%7D/")]
    #[case("https://myhost.mydomain", None, "https://myhost.mydomain/")]
    #[case("https://myhost.mydomain", Some(ResourceType::Object), "https://myhost.mydomain/")]
    #[case("https://myhost.mydomain", Some(ResourceType::Folder), "https://myhost.mydomain/")]
    #[case(
        "https://myhost.mydomain/ab",
        Some(ResourceType::Folder),
        "https://myhost.mydomain/ab/"
    )]
    #[case(
        "https://myhost.mydomain/ab%2f",
        Some(ResourceType::Folder),
        "https://myhost.mydomain/ab/"
    )]
    #[case(
        "https://myhost.mydomain/abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ/-_.!~*'()",
        None,
        "https://myhost.mydomain/abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ/-_.!~*'()"
    )]
    #[case("https://myhost.mydomain/?query=\"etag\"", None, "https://myhost.mydomain/")]
    #[case("https://myhost.mydomain/#frag", None, "https://myhost.mydomain/")]
    fn test_resource_url_parse_transcodes_path(
        #[case] url: &str,
        #[case] resource_type: Option<ResourceType>,
        #[case] expected: &str,
    ) {
        let resource_url = ResourceUrl::parse(url, resource_type).expect("url parse failed");
        let (resource_url, _) = resource_url.into_components();

        assert_eq!(resource_url, expected);
    }
}
