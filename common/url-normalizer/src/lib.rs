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

use url::Url;

const DEFAULT_ENCODE_SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC;

/// Normalize a path by removing consecutive slashes
fn normalize_path(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut prev_char = '\0';

    for ch in path.chars() {
        if ch == '/' && prev_char == '/' {
            // Skip consecutive slashes
            continue;
        }
        result.push(ch);
        prev_char = ch;
    }

    result
}

/// Reove default port i it matches the scheme's default
/// - Port 80 for http scheme
/// - Port 443 for https scheme  
/// - Port 21 for ftp scheme
/// - Any port for file scheme
fn should_remove_port(port: u16, scheme: &str) -> bool {
    let scheme = scheme.to_lowercase();
    let should_remove = match scheme.as_str() {
        "http" => port == 80,
        "https" => port == 443,
        "ftp" => port == 21,
        "file" => true, // Remove any port for file scheme
        _ => false,
    };
    should_remove
}

fn construct_normalized_url(
    scheme: &str,
    host: &str,
    port: u16,
    path: &str,
    query: &str,
) -> String {
    format!(
        "{}://{}{}{}{}",
        scheme,
        host,
        if port != 0 && !should_remove_port(port, scheme) {
            format!(":{}", port)
        } else {
            "".to_string()
        },
        path,
        if query.is_empty() {
            "".to_string()
        } else {
            format!("?{}", query)
        }
    )
}

fn sort_and_encode_query_params(mut query_pairs: Vec<(String, String)>) -> String {
    if query_pairs.is_empty() {
        return "".to_string();
    }
    query_pairs.sort_by(|a, b| a.0.cmp(&b.0));
    query_pairs
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                percent_encoding::utf8_percent_encode(k, DEFAULT_ENCODE_SET),
                percent_encoding::utf8_percent_encode(v, DEFAULT_ENCODE_SET)
            )
        })
        .collect::<Vec<String>>()
        .join("&")
}

/// Normalize a URL by:
/// - Removing fragments
/// - Sorting query parameters
/// - Normalizing the path (removing consecutive slashes)
/// - Removing default ports
/// - Percent-encoding the path
pub fn normalize_url(url: impl AsRef<str>) -> Result<String, url::ParseError> {
    let parsed_url = Url::parse(url.as_ref())?;

    let scheme = parsed_url.scheme().to_string();
    // A URL with no authority -- `mailto:`, `data:`, and other non-special
    // schemes -- has nothing to normalize here, and there is no host to put in
    // the `scheme://host/path` shape this function builds. Return it unchanged
    // rather than inventing one: the previous fallback spliced the literal
    // string "None" into the authority, producing URLs like
    // `data://None/text/html,...` that name a host the input never had (and
    // "None" is itself a resolvable name).
    let Some(host) = parsed_url.host_str().map(str::to_string) else {
        return Ok(parsed_url.to_string());
    };
    let port = parsed_url.port().unwrap_or(0);

    // Handle query parameters gracefully
    let query_pairs: Vec<(String, String)> = parsed_url
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    // Sort and encode query parameters
    let query_string = sort_and_encode_query_params(query_pairs);

    // Normalize the path by removing consecutive slashes
    let normalized_path = normalize_path(parsed_url.path());

    Ok(construct_normalized_url(
        &scheme,
        &host,
        port,
        &normalized_path,
        &query_string,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hostless_scheme_is_returned_unchanged() {
        let url = "mailto:user@example.com";
        assert_eq!(normalize_url(url).unwrap(), url);
    }

    #[test]
    fn test_hostless_scheme_does_not_gain_an_authority() {
        let normalized = normalize_url("data:text/html,hello").unwrap();
        assert!(
            !normalized.contains("//"),
            "expected no authority component, got {normalized}"
        );
    }

    #[test]
    fn test_hostless_scheme_does_not_produce_the_literal_none() {
        let normalized = normalize_url("data:text/html,hello").unwrap();
        assert!(
            !normalized.contains("None"),
            "expected no fabricated host, got {normalized}"
        );
    }

    #[test]
    fn test_verify_scheme() {
        let url = "www.NVIDIA.com/en-us/";
        let normalized_url = normalize_url(url);
        println!("normalized_url: {:?}", normalized_url);
        assert!(normalized_url.is_err());
    }

    #[test]
    fn test_verify_user_info() {
        let url = "https://user:pass@www.NVIDIA.com/en-us/";
        let normalized_url = normalize_url(url);
        assert_eq!(normalized_url.unwrap(), "https://www.nvidia.com/en-us/");
    }

    #[test]
    fn test_normalize_url() {
        let url = "https://www.nvidia.com/en-us/ Lorem ipsum.txt";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(
            normalized_url,
            "https://www.nvidia.com/en-us/%20Lorem%20ipsum.txt"
        );
    }

    #[test]
    fn test_normalize_url_double_slash() {
        let url = "https://www.nvidia.com///en-us///";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(normalized_url, "https://www.nvidia.com/en-us/");
        let url = "https://www.nvidia.com///en-us///";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(normalized_url, "https://www.nvidia.com/en-us/");
        let url = "https://www.nvidia.com///en-us///";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(normalized_url, "https://www.nvidia.com/en-us/");
    }
    #[test]
    fn test_normalize_url_double_dot() {
        let url = "https://www.nvidia.com//.././en-us///";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(normalized_url, "https://www.nvidia.com/en-us/");
    }

    #[test]
    fn test_normalize_url_query_params() {
        let url = "https://www.nvidia.com/en-us/?q22=1&q11=2&q33=3";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(
            normalized_url,
            "https://www.nvidia.com/en-us/?q11=2&q22=1&q33=3"
        );
    }
    #[test]
    fn test_normalize_url_query_params_encode() {
        let url = "https://www.nvidia.com/en-us/?с=1&ս=2&人=3";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(
            normalized_url,
            "https://www.nvidia.com/en-us/?%D1%81=1&%D5%BD=2&%E4%BA%BA=3"
        );
    }
    #[test]
    fn test_normalize_url_fragment() {
        let url = "https://www.nvidia.com/en-us/#fragment";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(normalized_url, "https://www.nvidia.com/en-us/");
    }

    #[test]
    fn test_normalize_caps() {
        let url = "https://www.NVIDIA.com/en-us/";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(normalized_url, "https://www.nvidia.com/en-us/");
    }

    // Tests for default port removal
    #[test]
    fn test_remove_default_port() {
        let url = "http://example.com:80/path";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(normalized_url, "http://example.com/path");
        let url = "https://example.com:443/path";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(normalized_url, "https://example.com/path");
        let url = "ftp://example.com:21/path";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(normalized_url, "ftp://example.com/path");
    }

    #[test]
    fn test_normalize_url_with_default_ports() {
        let url = "http://example.com:80/path";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(normalized_url, "http://example.com/path");
    }

    #[test]
    fn test_normalize_url_with_non_default_ports() {
        let url = "http://example.com:8080/path";
        let normalized_url = normalize_url(url).unwrap();
        assert_eq!(normalized_url, "http://example.com:8080/path");
    }
}
