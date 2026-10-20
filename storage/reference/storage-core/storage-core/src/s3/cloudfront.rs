// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{ops::Deref, sync::Arc};

use aws_sdk_s3::config::{interceptors::BeforeTransmitInterceptorContextMut, ConfigBag, Intercept, RuntimeComponents};
use base64::{alphabet::Alphabet, engine::general_purpose, engine::GeneralPurpose, Engine};
use openssl::{
    hash::MessageDigest,
    pkey::{PKey, Private},
    sign::Signer,
};
use storage_core_util::debug_limited;
use url::Url;

use storage_core_util::expiration::ExpirationNormalizer;

use super::error::Error;

#[derive(Debug)]
pub(crate) struct CloudfrontSigningInfo {
    key_pair_id: String,
    private_key: PKey<Private>,
    base64_engine: GeneralPurpose,
    expiration: ExpirationNormalizer,
}

impl CloudfrontSigningInfo {
    // Create new signing info from a PEM string
    fn from_pem(
        key_pair_id: impl Into<String>,
        pem: &str,
        expiration: ExpirationNormalizer,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let private_key = PKey::private_key_from_pem(pem.as_bytes())?;

        // cloudfront base64 alphabet (slightly modified from standard base64)
        // https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/private-content-creating-signed-url-canned-policy.html
        let custom_alphabet = Alphabet::new("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-~")?;

        // Create engine with custom alphabet (no padding for _ instead of =)
        let base64_engine = GeneralPurpose::new(&custom_alphabet, general_purpose::NO_PAD);

        Ok(Self {
            key_pair_id: key_pair_id.into(),
            private_key,
            base64_engine,
            expiration,
        })
    }

    // Sign a message with the private key
    fn try_sign(&self, message: &[u8]) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let mut signer = Signer::new(MessageDigest::sha1(), &self.private_key)?;
        let signature = self.base64_engine.encode(signer.sign_oneshot_to_vec(message)?);
        Ok(signature)
    }
}

#[derive(Debug)]
pub(crate) struct CloudFrontInterceptorInner {
    distribution_url: Url,
    signing_info: Option<CloudfrontSigningInfo>,
}

#[derive(Debug, Clone)]
pub(crate) struct CloudFrontInterceptor {
    // wrap inner in Arc to make it clone fast, since we need to clone the interceptor for each request
    inner: Arc<CloudFrontInterceptorInner>,
}

impl Deref for CloudFrontInterceptor {
    type Target = CloudFrontInterceptorInner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl CloudFrontInterceptor {
    // create a new interceptor with no signing info (public access)
    pub(crate) fn new(distribution_url: Url) -> Self {
        Self {
            inner: Arc::new(CloudFrontInterceptorInner {
                distribution_url,
                signing_info: None,
            }),
        }
    }

    // create a new interceptor with signing info (private access)
    pub(crate) fn with_signing_info(
        distribution_url: Url,
        key_pair_id: impl Into<String>,
        pem: &str,
        expiration: ExpirationNormalizer,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let signing_info = Some(CloudfrontSigningInfo::from_pem(key_pair_id, pem, expiration)?);
        Ok(Self {
            inner: Arc::new(CloudFrontInterceptorInner {
                distribution_url,
                signing_info,
            }),
        })
    }
}

impl Intercept for CloudFrontInterceptor {
    fn name(&self) -> &'static str {
        "CloudFront"
    }

    fn modify_before_transmit(
        &self,
        context: &mut BeforeTransmitInterceptorContextMut<'_>,
        _runtime_components: &RuntimeComponents,
        _cfg: &mut ConfigBag,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let request = context.request_mut();
        let mut request_url = Url::parse(request.uri())?;

        // set the scheme and host to the distribution URL
        request_url
            .set_scheme(self.distribution_url.scheme())
            .map_err(|_| Error::internal("failed to set scheme"))?;
        request_url.set_host(self.distribution_url.host_str())?;

        // if we are signing the request, add the signature to the URL
        if let Some(signing_info) = &self.signing_info {
            let expires = signing_info.expiration.get_expiration()?.end_time();
            let policy = format!(
                r#"{{"Statement":[{{"Resource":"{request_url}","Condition":{{"DateLessThan":{{"AWS:EpochTime":{expires}}}}}}}]}}"#
            );
            let signature = signing_info.try_sign(policy.as_bytes())?;
            debug_limited!(%signature, %policy);

            request_url
                .query_pairs_mut()
                .append_pair("Expires", &expires.to_string())
                .append_pair("Signature", &signature)
                .append_pair("Key-Pair-Id", &signing_info.key_pair_id);
        }

        // replace the request URL with the cloudfront URL
        debug_limited!(cloudfront_url = %request_url, request = %request.uri());
        request.set_uri(request_url.to_string())?;

        Ok(())
    }
}
