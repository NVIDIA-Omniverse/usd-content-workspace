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

pub mod grpc;
pub mod http;

/// Extract a bearer token from a raw `Authorization` / `authorization` header value.
///
/// Accepts any casing of the `Bearer` scheme. Returns `None` when the scheme is
/// absent/wrong or the credential is missing/blank after the scheme.
pub(crate) fn bearer_token_from_header(value: &str) -> Option<String> {
    let mut parts = value.splitn(2, char::is_whitespace);
    let scheme = parts.next()?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = parts.next()?.trim();
    if token.is_empty() {
        return None;
    }
    Some(token.to_string())
}

#[cfg(test)]
mod tests {
    use super::bearer_token_from_header;

    #[test]
    fn bearer_token_accepts_standard_and_lowercase_scheme() {
        assert_eq!(
            bearer_token_from_header("Bearer abc"),
            Some("abc".to_string())
        );
        assert_eq!(
            bearer_token_from_header("bearer abc"),
            Some("abc".to_string())
        );
        assert_eq!(
            bearer_token_from_header("BEARER abc"),
            Some("abc".to_string())
        );
    }

    #[test]
    fn bearer_token_rejects_missing_or_blank_credential() {
        assert_eq!(bearer_token_from_header("Bearer"), None);
        assert_eq!(bearer_token_from_header("Bearer "), None);
        assert_eq!(bearer_token_from_header("Bearer   "), None);
        assert_eq!(bearer_token_from_header("Basic abc"), None);
        assert_eq!(bearer_token_from_header(""), None);
    }
}
