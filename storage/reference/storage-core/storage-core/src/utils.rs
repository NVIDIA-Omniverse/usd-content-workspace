// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core_util::observability::metrics::{self, UIntCounter};
use storage_core_util::warn_limited;

metrics::metric_base!(
    static RATE_LIMIT_COUNTER_BASE: UIntCounter,
    name = "sdk.rate_limit",
    description = "Number of rate-limit (429/503) responses from the storage backend",
    { sdk_operation, storage_backend, pod_name, pkg_version }
);

pub fn rate_limit_counter_base() -> &'static UIntCounter {
    &RATE_LIMIT_COUNTER_BASE
}

/// Environment variable naming the UCC proxy that the S3 and Azure backends send their traffic through.
pub const UCC_PROXY_ENV: &str = "UCC_PROXY";

/// Environment variable that controls TLS server verification on the UCC proxy path.
pub const UCC_PROXY_SSL_VERIFYCERT_ENV: &str = "UCC_PROXY_SSL_VERIFYCERT";

/// The only `UCC_PROXY_SSL_VERIFYCERT` value that turns TLS server verification off.
pub const UCC_PROXY_SSL_VERIFYCERT_INSECURE: &str = "insecure";

/// Decides whether TLS server certificates and hostnames are verified on the UCC proxy path,
/// given the value of `UCC_PROXY_SSL_VERIFYCERT` (`None` when it is unset or not valid UTF-8).
///
/// Verification is turned off only for the exact value `insecure`. Every other value, including
/// `false`, `0` and an unset variable, keeps verification on, so a typo or a generic boolean can
/// never disable it.
pub fn ucc_proxy_verify_certs(verifycert: Option<&str>) -> bool {
    verifycert != Some(UCC_PROXY_SSL_VERIFYCERT_INSECURE)
}

/// Reads `UCC_PROXY_SSL_VERIFYCERT` and decides whether the backend's connection through the UCC
/// proxy at `proxy_address` verifies TLS server certificates and hostnames (see
/// [`ucc_proxy_verify_certs`]). Logs a warning when verification is turned off, and when a value
/// that used to turn it off (`false`, `0`) is set, since that value now keeps verification on.
pub fn ucc_proxy_tls_verification(proxy_address: &str) -> bool {
    let verifycert = std::env::var(UCC_PROXY_SSL_VERIFYCERT_ENV).ok();
    let verify_certs = ucc_proxy_verify_certs(verifycert.as_deref());
    if !verify_certs {
        warn_limited!(
            proxy = proxy_address,
            "{UCC_PROXY_SSL_VERIFYCERT_ENV}={UCC_PROXY_SSL_VERIFYCERT_INSECURE}: TLS certificate and hostname \
             verification is disabled for connections through the UCC proxy; use this only with a local test proxy"
        );
    } else if let Some(value) = verifycert
        .as_deref()
        .filter(|v| v.eq_ignore_ascii_case("false") || *v == "0")
    {
        warn_limited!(
            proxy = proxy_address,
            "{UCC_PROXY_SSL_VERIFYCERT_ENV}={value} no longer disables TLS verification for the UCC proxy; \
             verification stays on. Set it to {UCC_PROXY_SSL_VERIFYCERT_INSECURE} to disable verification \
             for a local test proxy"
        );
    }
    verify_certs
}

#[cfg(test)]
mod tests {
    use super::ucc_proxy_verify_certs;

    #[test]
    fn ucc_proxy_verifies_when_unset() {
        assert!(ucc_proxy_verify_certs(None));
    }

    #[test]
    fn ucc_proxy_verifies_for_legacy_false_values() {
        for value in ["false", "FALSE", "False", "0"] {
            assert!(ucc_proxy_verify_certs(Some(value)), "{value} must keep verification on");
        }
    }

    #[test]
    fn ucc_proxy_verifies_for_other_values() {
        for value in [
            "",
            "true",
            "1",
            "no",
            "off",
            "INSECURE",
            "Insecure",
            " insecure",
            "insecure ",
        ] {
            assert!(
                ucc_proxy_verify_certs(Some(value)),
                "{value:?} must keep verification on"
            );
        }
    }

    #[test]
    fn ucc_proxy_skips_verification_only_for_insecure() {
        assert!(!ucc_proxy_verify_certs(Some("insecure")));
    }
}
