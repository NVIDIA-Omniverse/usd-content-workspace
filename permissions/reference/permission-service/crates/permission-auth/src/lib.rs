// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use crate::config::{AuthConfig, AuthType};
use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve, Jwk, JwkSet, KeyAlgorithm};
use jsonwebtoken::Algorithm;
use jsonwebtoken::{decode, decode_header, DecodingKey, TokenData, Validation};
use moka::future::Cache;
use permission_core::Principal;
use reqwest::{Client as HttpClient, StatusCode};
use serde_json::{Map, Value};
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::ops::Add;
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use tokio::sync::RwLock;

pub mod claim_rules;
pub mod config;
#[cfg(feature = "grpc")]
pub mod interceptors;
pub mod service_identity;

#[tracing::instrument(level = "info", skip(token, config, cache, client))]
pub async fn verify_user_token(
    token: &str,
    config: &AuthConfig,
    client: &HttpClient,
    cache: &AuthCache,
) -> Result<Arc<Principal>, AuthError> {
    if config.openid_response.is_none() {
        return Err(AuthError::InternalError(
            "OpenID configuration is not initialized.".to_string(),
        ));
    }

    // Reject oversized tokens up-front, before touching the cache, parsing the
    // JWT header, or forwarding the token to the IdP. Without this bound an
    // attacker could (1) poison/evict entries in the moka token cache by
    // sending many large bogus tokens, (2) coerce the service into amplifying
    // traffic to the IdP `userinfo` endpoint in opaque mode, and (3) force
    // base64 decoding of arbitrarily large inputs in `decode_header`.
    if token.len() > config.max_token_length {
        tracing::warn!(
            token_len = token.len(),
            max_token_length = config.max_token_length,
            "Rejecting bearer token: length exceeds max_token_length"
        );
        return Err(AuthError::Unauthorized());
    }

    // Hash the bearer once and reuse the fingerprint for every cache probe on
    // the hot path, instead of re-hashing the whole token per probe.
    let token_key = cache.token_key(token);

    // Negative cache is checked first: a hit short-circuits without cloning a
    // principal, and bogus-token floods land here without touching the
    // valid-principal cache.
    if get_cached_invalid(&token_key, cache).await {
        return Err(AuthError::Unauthorized());
    }

    if let Some(principal) = get_cached_valid(&token_key, cache).await {
        return Ok(principal);
    }

    match &config.authentication_type {
        AuthType::Jwt | AuthType::JwtWithUserInfo => {
            let jwks = get_jwks(config, cache, client).await?;

            let mut principal = decode_token_with_jwks(token, &jwks, config, cache).await?;

            if let AuthType::JwtWithUserInfo = &config.authentication_type {
                let userinfo_endpoint = if let Some(openid) = &config.openid_response {
                    &openid.userinfo_endpoint
                } else {
                    return Err(AuthError::InvalidInput(
                        "userinfo_endpoint is not specified.".to_string(),
                    ));
                };

                let userinfo = fetch_userinfo(token, client, userinfo_endpoint).await?;
                if principal.id.is_empty() {
                    let claim = &config.principal_id_claim;
                    if let Some(Value::String(id)) = userinfo.get(claim.as_str()) {
                        principal.id = id.clone();
                    } else if claim != "sub" {
                        if let Some(Value::String(id)) = userinfo.get("sub") {
                            principal.id = id.clone();
                        }
                    }
                    if principal.id.is_empty() {
                        tracing::debug!("Unable to get userinfo '{}' or 'sub'", claim);
                        cache_invalid_token(token, cache, config).await;
                        return Err(AuthError::Unauthorized());
                    }
                }

                let info = principal.info.clone();
                if let Some(Value::Object(mut info)) = info {
                    info.extend(userinfo);
                    principal.info = Some(Value::Object(info));
                } else {
                    principal.info = Some(Value::Object(userinfo));
                }
            }

            Ok(cache_valid_token(&token_key, principal, cache, config).await)
        }
        AuthType::Opaque => {
            let principal = opaque_auth_check(token, config, client, cache).await?;
            Ok(cache_valid_token(&token_key, principal, cache, config).await)
        }
    }
}

#[tracing::instrument(level = "debug", skip(client, token))]
pub async fn fetch_userinfo(
    token: &str,
    client: &HttpClient,
    userinfo_endpoint: &str,
) -> Result<Map<String, Value>, AuthError> {
    if userinfo_endpoint.is_empty() {
        return Err(AuthError::InvalidInput(
            "userinfo_endpoint is not specified.".to_string(),
        ));
    }

    let response = client
        .get(userinfo_endpoint)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|err| {
            tracing::error!("Failed to get userinfo: {:?}", err);
            AuthError::InternalError("failed to get userinfo".to_string())
        })?;

    if response.status().is_success() {
        let json = response.json().await.map_err(|err| {
            tracing::error!("Failed to parse userinfo response: {:?}", err);
            AuthError::InternalError("failed to parse userinfo response".to_string())
        })?;
        Ok(json)
    } else {
        let status = response.status();
        let text = response.text().await.map_err(|err| {
            tracing::error!("Failed to parse userinfo error: {:?}", err);
            AuthError::InternalError("failed to parse userinfo error".to_string())
        })?;

        tracing::warn!("Fetch userinfo request returned HTTP {} {}", status, text);
        match status {
            StatusCode::FORBIDDEN => Err(AuthError::Forbidden()),
            _ => Err(AuthError::Unauthorized()),
        }
    }
}

#[tracing::instrument(level = "debug", skip(config, cache, client))]
async fn get_jwks(
    config: &AuthConfig,
    cache: &AuthCache,
    client: &HttpClient,
) -> Result<Arc<JwkSet>, AuthError> {
    {
        let cache_lock = cache.jwk.read().await;
        if let Some(jwks) = get_valid_jwks_from_cache(&cache_lock, config) {
            return Ok(jwks);
        }
        if jwks_failure_backoff_active(&cache_lock, config) {
            return Err(AuthError::NetworkError(
                "JWKS fetch suppressed by failure backoff".to_string(),
            ));
        }
    }

    // Cache is expired, acquire write lock to coordinate fetch
    let mut cache_lock = cache.jwk.write().await;

    // Double-check cache after acquiring write lock (another thread might have updated it)
    if let Some(jwks) = get_valid_jwks_from_cache(&cache_lock, config) {
        return Ok(jwks);
    }
    // Re-check the backoff window: if a sibling task just failed, every
    // queued request must short-circuit instead of issuing the same call.
    if jwks_failure_backoff_active(&cache_lock, config) {
        return Err(AuthError::NetworkError(
            "JWKS fetch suppressed by failure backoff".to_string(),
        ));
    }

    // Cache is still expired, fetch new JWKs
    let jwks_uri = if let Some(openid) = &config.openid_response {
        &openid.jwks_uri
    } else {
        tracing::warn!("jwks_uri is not specified, unable to fetch JWKs.");
        return Ok(Arc::new(JwkSet { keys: vec![] }));
    };

    if !jwks_uri.is_empty() {
        tracing::debug!("Getting new JWKs from {}...", jwks_uri);
        let result = fetch_jwks(client, jwks_uri).await;
        match result {
            Ok(jwks) => {
                let jwks = Arc::new(jwks);
                cache_lock.keys = Some(Arc::clone(&jwks));
                cache_lock.time = OffsetDateTime::now_utc();
                cache_lock.last_failure = None;

                tracing::info!("Pulled new JWKs.");
                Ok(jwks)
            }
            Err(err) => {
                cache_lock.last_failure = Some(OffsetDateTime::now_utc());
                Err(err)
            }
        }
    } else {
        tracing::warn!("jwks_uri is not specified, unable to fetch JWKs.");
        Ok(Arc::new(JwkSet { keys: vec![] }))
    }
}

async fn fetch_jwks(client: &HttpClient, jwks_uri: &str) -> Result<JwkSet, AuthError> {
    let response = client.get(jwks_uri).send().await.map_err(|err| {
        tracing::error!("Failed to get JWKs: {:?}", err);
        AuthError::NetworkError("failed to get JWKs".to_string())
    })?;

    if response.status().is_success() {
        response.json::<JwkSet>().await.map_err(|err| {
            tracing::error!("Failed to parse JWKs response: {:?}", err);
            AuthError::NetworkError("failed to parse JWKs response".to_string())
        })
    } else {
        let status = response.status();
        let text = response.text().await.map_err(|err| {
            tracing::error!("Failed to parse JWKs error: {:?}", err);
            AuthError::NetworkError("failed to parse JWKs error".to_string())
        })?;

        tracing::warn!("Fetch JWKs request returned HTTP {} {}", status, text);
        Err(AuthError::NetworkError(format!(
            "Fetch JWKs request returned HTTP {} {}",
            status, text
        )))
    }
}

/// Returns `true` when a recent JWKS fetch failed and the operator-configured
/// backoff window has not yet elapsed.
fn jwks_failure_backoff_active(cache_lock: &JwkCache, config: &AuthConfig) -> bool {
    match cache_lock.last_failure {
        Some(failure_time) => {
            OffsetDateTime::now_utc() < failure_time.add(config.jwks_failure_backoff)
        }
        None => false,
    }
}

fn get_valid_jwks_from_cache(cache_lock: &JwkCache, config: &AuthConfig) -> Option<Arc<JwkSet>> {
    if let Some(jwks) = &cache_lock.keys {
        let expiration_time = cache_lock.time.add(config.jwks_ttl);
        if OffsetDateTime::now_utc() <= expiration_time {
            return Some(Arc::clone(jwks));
        }
    }
    None
}

/// Returns the cached principal for the given token, or `None` if the token
/// is not in the positive cache or its TTL has expired. Expired entries are
/// proactively removed.
#[tracing::instrument(level = "debug", skip(token_key, cache))]
async fn get_cached_valid(token_key: &TokenKey, cache: &AuthCache) -> Option<Arc<Principal>> {
    match cache.valid_tokens.get(token_key).await {
        Some(cached) => {
            let now = OffsetDateTime::now_utc();
            if cached.exp > now {
                Some(cached.principal)
            } else {
                cache.valid_tokens.remove(token_key).await;
                None
            }
        }
        None => None,
    }
}

/// Returns `true` if the token has a non-expired entry in the negative cache.
/// Expired entries are proactively removed.
#[tracing::instrument(level = "debug", skip(token_key, cache))]
async fn get_cached_invalid(token_key: &TokenKey, cache: &AuthCache) -> bool {
    match cache.invalid_tokens.get(token_key).await {
        Some(exp) => {
            let now = OffsetDateTime::now_utc();
            if exp > now {
                true
            } else {
                cache.invalid_tokens.remove(token_key).await;
                false
            }
        }
        None => false,
    }
}

/// Records a token as invalid in the bounded negative cache.
///
/// Stored separately from the positive cache so that a flood of bogus
/// bearers cannot evict verified principals (the eviction policy is per
/// moka instance). The negative cache is sized and TTL'd much smaller than
/// the positive cache; see `AuthConfig::invalid_token_cache_size` and
/// `invalid_token_cache_ttl`.
///
/// The per-entry expiration is bounded by `now + invalid_token_cache_ttl`
/// so that an IdP-side change of mind (a token that was rejected once and
/// then re-issued, or an upstream policy flip) is reconsidered within a
/// short, operator-controlled window rather than being treated as
/// permanently invalid for the remainder of `token_ttl`.
#[tracing::instrument(level = "debug", skip(token, cache, config))]
async fn cache_invalid_token(token: &str, cache: &AuthCache, config: &AuthConfig) {
    let exp = OffsetDateTime::now_utc().add(config.invalid_token_cache_ttl);
    cache
        .invalid_tokens
        .insert(cache.token_key(token), exp)
        .await;
}

/// Records a verified principal in the positive cache.
///
/// The cached lifetime is `min(principal.exp, now + token_ttl)`.
///
/// Returns the shared `Arc<Principal>` that was stored so the caller can hand
/// it to the request without re-cloning the claims tree.
#[tracing::instrument(level = "debug", skip(token_key, principal, cache, config))]
async fn cache_valid_token(
    token_key: &TokenKey,
    principal: AuthPrincipal,
    cache: &AuthCache,
    config: &AuthConfig,
) -> Arc<Principal> {
    let cap = OffsetDateTime::now_utc().add(config.token_ttl);
    let exp = principal.exp.min(cap);
    let shared = Arc::new(Principal {
        id: principal.id,
        info: principal.info,
    });
    cache
        .valid_tokens
        .insert(
            *token_key,
            CachedPrincipal {
                exp,
                principal: Arc::clone(&shared),
            },
        )
        .await;
    shared
}

#[derive(Clone)]
struct CachedPrincipal {
    exp: OffsetDateTime,
    principal: Arc<Principal>,
}

async fn opaque_auth_check(
    token: &str,
    config: &AuthConfig,
    client: &HttpClient,
    cache: &AuthCache,
) -> Result<AuthPrincipal, AuthError> {
    let userinfo_endpoint = if let Some(openid) = &config.openid_response {
        &openid.userinfo_endpoint
    } else {
        return Err(AuthError::InvalidInput(
            "userinfo_endpoint is not specified.".to_string(),
        ));
    };

    match fetch_userinfo(token, client, userinfo_endpoint).await {
        Ok(userinfo) => {
            let token_data = TokenData {
                header: Default::default(),
                claims: convert_claims_from_userinfo(userinfo, config),
            };
            enforce_required_claims(token, &token_data.claims, config, cache).await?;
            create_principal(token_data, config)
        }
        Err(AuthError::Unauthorized()) => {
            cache_invalid_token(token, cache, config).await;
            Err(AuthError::Unauthorized())
        }
        Err(err) => Err(err),
    }
}

fn convert_claims_from_userinfo(
    userinfo: Map<String, Value>,
    config: &AuthConfig,
) -> HashMap<String, Value> {
    let mut result: HashMap<String, Value> = userinfo.into_iter().collect();
    result.insert(
        "exp".to_string(),
        OffsetDateTime::now_utc()
            .add(config.token_ttl)
            .unix_timestamp()
            .into(),
    );
    result
}

async fn enforce_required_claims(
    token: &str,
    claims: &HashMap<String, Value>,
    config: &AuthConfig,
    cache: &AuthCache,
) -> Result<(), AuthError> {
    match config.jwt_required_claims.evaluate(claims) {
        Ok(()) => Ok(()),
        Err(violation) => {
            tracing::debug!("Rejecting token: {}", violation);
            cache_invalid_token(token, cache, config).await;
            Err(AuthError::Unauthorized())
        }
    }
}

async fn decode_principal_from_key(
    token: &str,
    decoding_key: DecodingKey,
    allowed_algorithms: Vec<Algorithm>,
    config: &AuthConfig,
    cache: &AuthCache,
) -> Result<AuthPrincipal, AuthError> {
    let validation = {
        // `Validation::new` takes a single algorithm; pick any from the
        // allowed set as a seed and immediately replace the field with the
        // full set so verification accepts any of them.
        let seed = allowed_algorithms
            .first()
            .copied()
            .unwrap_or(Algorithm::RS256);
        let mut validation = Validation::new(seed);
        validation.algorithms = allowed_algorithms;

        if let Some(aud) = &config.jwt_audience {
            validation.set_audience(aud);
            validation.required_spec_claims.insert("aud".to_string());
        }
        // Bind verification to an explicit `iss` allowlist when configured.
        // Operators are expected to list every accepted issuer here; the
        // discovery-document issuer is not added implicitly to keep behavior
        // predictable across IdPs that diverge from their discovery URL
        if let Some(issuers) = &config.jwt_issuers {
            let issuers: Vec<&str> = issuers.iter().map(String::as_str).collect();
            if !issuers.is_empty() {
                validation.set_issuer(&issuers);
                validation.required_spec_claims.insert("iss".to_string());
            }
        }
        validation.validate_nbf = true;
        validation.leeway = config.jwt_leeway;
        validation
    };

    match decode::<HashMap<String, Value>>(token, &decoding_key, &validation) {
        Ok(decoded_token) => {
            enforce_required_claims(token, &decoded_token.claims, config, cache).await?;
            create_principal(decoded_token, config)
        }
        Err(err) => {
            tracing::debug!("Failed to decode token: {:?}", err);
            cache_invalid_token(token, cache, config).await;
            Err(AuthError::Unauthorized())
        }
    }
}

#[tracing::instrument(level = "debug", skip(token, jwk, config, cache))]
async fn decode_jwt_token(
    header_alg: Algorithm,
    token: &str,
    jwk: &Jwk,
    config: &AuthConfig,
    cache: &AuthCache,
) -> Result<AuthPrincipal, AuthError> {
    // The set of acceptable signing algorithms is derived from the JWK
    // (its `alg` field if present, otherwise a per-key-type allowlist),
    // not from the attacker-controlled JWT header. This prevents a token
    // from selecting an algorithm the IdP did not bind to the key.
    let allowed_algorithms = match allowed_algorithms_for_jwk(jwk) {
        Some(algs) => algs,
        None => {
            tracing::error!("Unsupported JWK: {:?}", jwk.algorithm);
            return Err(AuthError::InternalError(
                "unsupported JWK algorithm".to_string(),
            ));
        }
    };

    if !allowed_algorithms.contains(&header_alg) {
        tracing::debug!(
            "JWT header algorithm {:?} is not permitted for the matched JWK",
            header_alg
        );
        cache_invalid_token(token, cache, config).await;
        return Err(AuthError::Unauthorized());
    }

    match &jwk.algorithm {
        AlgorithmParameters::RSA(rsa) => match DecodingKey::from_rsa_components(&rsa.n, &rsa.e) {
            Ok(key) => {
                decode_principal_from_key(token, key, allowed_algorithms, config, cache).await
            }
            Err(err) => {
                tracing::error!("Failed to parse RSA components from the key: {:?}", err);
                Err(AuthError::InternalError(
                    "failed to parse RSA components from the key".to_string(),
                ))
            }
        },
        AlgorithmParameters::EllipticCurve(ec) => {
            match DecodingKey::from_ec_components(&ec.x, &ec.y) {
                Ok(key) => {
                    decode_principal_from_key(token, key, allowed_algorithms, config, cache).await
                }
                Err(err) => {
                    tracing::error!("Failed to parse EC components from the key: {:?}", err);
                    Err(AuthError::InternalError(
                        "failed to parse EC components from the key".to_string(),
                    ))
                }
            }
        }
        _ => {
            tracing::error!("Unsupported JWK algorithm: {:?}", jwk.algorithm);
            Err(AuthError::InternalError(
                "unsupported JWK algorithm".to_string(),
            ))
        }
    }
}

/// Returns the set of signing algorithms that should be accepted for a token
/// verified with the given JWK.
///
/// When the JWK declares an `alg`, that single algorithm is the only one
/// accepted (RFC 7517 §4.4: the key is intended for use with that algorithm).
/// Otherwise, fall back to a conservative allowlist driven by the key type
/// and (for EC) the curve. Returns `None` when the JWK is not usable for
/// JWT signature verification (e.g. key-encryption algorithm or unsupported
/// curve).
fn allowed_algorithms_for_jwk(jwk: &Jwk) -> Option<Vec<Algorithm>> {
    if let Some(key_algorithm) = jwk.common.key_algorithm {
        return key_algorithm_to_signature_algorithm(key_algorithm).map(|alg| vec![alg]);
    }

    match &jwk.algorithm {
        AlgorithmParameters::RSA(_) => Some(vec![
            Algorithm::RS256,
            Algorithm::RS384,
            Algorithm::RS512,
            Algorithm::PS256,
            Algorithm::PS384,
            Algorithm::PS512,
        ]),
        AlgorithmParameters::EllipticCurve(ec) => match ec.curve {
            EllipticCurve::P256 => Some(vec![Algorithm::ES256]),
            EllipticCurve::P384 => Some(vec![Algorithm::ES384]),
            // P-521 has no corresponding `Algorithm` in jsonwebtoken 9.x;
            // Ed25519 keys are represented as OctetKeyPair rather than
            // EllipticCurve and are handled via the outer `_` arm.
            EllipticCurve::P521 | EllipticCurve::Ed25519 => None,
        },
        _ => None,
    }
}

/// Maps a JWK `alg` value (RFC 7518) to the signature `Algorithm` that
/// jsonwebtoken understands. Returns `None` for key-management / encryption
/// algorithms which must not be used for token-signature verification.
fn key_algorithm_to_signature_algorithm(key_algorithm: KeyAlgorithm) -> Option<Algorithm> {
    match key_algorithm {
        KeyAlgorithm::RS256 => Some(Algorithm::RS256),
        KeyAlgorithm::RS384 => Some(Algorithm::RS384),
        KeyAlgorithm::RS512 => Some(Algorithm::RS512),
        KeyAlgorithm::PS256 => Some(Algorithm::PS256),
        KeyAlgorithm::PS384 => Some(Algorithm::PS384),
        KeyAlgorithm::PS512 => Some(Algorithm::PS512),
        KeyAlgorithm::ES256 => Some(Algorithm::ES256),
        KeyAlgorithm::ES384 => Some(Algorithm::ES384),
        KeyAlgorithm::EdDSA => Some(Algorithm::EdDSA),
        // HMAC families require a shared symmetric key and must never be
        // accepted in combination with public-key JWKs (classic alg-confusion
        // attack vector).
        KeyAlgorithm::HS256 | KeyAlgorithm::HS384 | KeyAlgorithm::HS512 => None,
        // Key-encryption algorithms are not valid for signature verification.
        KeyAlgorithm::RSA1_5 | KeyAlgorithm::RSA_OAEP | KeyAlgorithm::RSA_OAEP_256 => None,
    }
}

#[tracing::instrument(level = "debug", skip(token, jwks, config, cache))]
async fn decode_token_with_jwks(
    token: &str,
    jwks: &JwkSet,
    config: &AuthConfig,
    cache: &AuthCache,
) -> Result<AuthPrincipal, AuthError> {
    match decode_header(token) {
        Ok(header) => match header.kid {
            Some(kid) => match jwks.find(&kid) {
                Some(jwk) => decode_jwt_token(header.alg, token, jwk, config, cache).await,
                None => {
                    tracing::debug!("JWK with ID {} does not exist", kid);
                    Err(AuthError::Unauthorized())
                }
            },
            None => {
                tracing::debug!("Missing JWK key ID");
                Err(AuthError::Unauthorized())
            }
        },
        Err(_) => {
            // Intentionally do NOT negative-cache here. Reaching this arm
            // means the bearer is not even a parseable JWT (decode_header
            // failed before any signature or claim work). Caching such
            // strings turns the moka token cache into an attacker-writable
            // sink: ~`token_cache_size` distinct random bearers are enough
            // to evict every legitimate principal entry. Re-parsing a bad
            // header on each request is cheap and involves no IdP I/O, so
            // there is nothing to amortize by remembering the failure.
            tracing::debug!("Missing JWT header");
            Err(AuthError::Unauthorized())
        }
    }
}

#[tracing::instrument(level = "debug", skip(token_data, config))]
fn create_principal(
    token_data: TokenData<HashMap<String, Value>>,
    config: &AuthConfig,
) -> Result<AuthPrincipal, AuthError> {
    let exp = token_data
        .claims
        .get("exp")
        .and_then(|claim| claim.as_i64())
        .and_then(|exp| OffsetDateTime::from_unix_timestamp(exp).ok())
        .unwrap_or_else(|| OffsetDateTime::now_utc().add(config.token_ttl));

    // The principal id is sourced from the configured claim, falling back
    // to `sub` only when the configured claim is something else. Empty
    // strings are treated as "missing" and rejected.
    let claim = &config.principal_id_claim;
    let id = match read_claim(&token_data.claims, claim) {
        Some(id) => id,
        None if claim != "sub" => match read_claim(&token_data.claims, "sub") {
            Some(id) => id,
            None => {
                tracing::debug!("Missing or empty JWT '{}' and 'sub' attributes", claim);
                return Err(AuthError::Unauthorized());
            }
        },
        None => {
            tracing::debug!("Missing or empty JWT 'sub' attribute");
            return Err(AuthError::Unauthorized());
        }
    };

    let info = serde_json::to_value(token_data.claims.clone()).ok();
    Ok(AuthPrincipal { id, info, exp })
}

/// Reads a JWT claim that is expected to carry a meaningful string value.
///
/// Returns the claim's string value when it is present, of string type,
/// and not empty. A missing claim, a non-string value, or an empty
/// string are all reported as `None` so that callers can treat them
/// uniformly as "not provided".
fn read_claim(claims: &HashMap<String, Value>, claim: &str) -> Option<String> {
    match claims.get(claim) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

#[derive(Clone)]
struct AuthPrincipal {
    pub id: String,
    pub info: Option<Value>,
    pub exp: OffsetDateTime,
}

#[derive(Debug)]
pub enum AuthError {
    InvalidInput(String),
    NetworkError(String),
    Unauthorized(),
    Forbidden(),
    InternalError(String),
}

impl Error for AuthError {}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AuthError::InvalidInput(ref msg) => write!(f, "invalid input: {}", msg),
            AuthError::NetworkError(ref msg) => write!(f, "network error: {}", msg),
            AuthError::Unauthorized() => write!(f, "unauthorized"),
            AuthError::Forbidden() => write!(f, "forbidden"),
            AuthError::InternalError(ref msg) => write!(f, "internal error: {}", msg),
        }
    }
}

/// Cache key derived from a bearer token.
///
/// Bearer tokens are never stored verbatim in the in-memory cache: a heap
/// dump, core dump, or any debugger attached to the process could otherwise
/// trivially exfiltrate live credentials. Instead, the cache is keyed on a
/// 128-bit fingerprint produced by hashing the token under two independently
/// keyed SipHash-1-3 instances. The hasher seeds are sampled from the OS RNG
/// once per `AuthCache` (via `RandomState`), so an attacker reading the
/// cache cannot map an entry back to a token without also knowing the
/// per-process keys and the original token.
type TokenKey = [u64; 2];

/// In-memory cache backing `verify_user_token`.
///
/// The cache is split into two independent moka instances:
///
/// - `valid_tokens` stores verified principals and is sized for steady-state
///   traffic.
/// - `invalid_tokens` is a bounded, short-TTL negative cache for tokens that
///   reached a downstream verification step and were rejected. Keeping it
///   separate from `valid_tokens` ensures that an attacker flooding the
///   service with bogus bearers cannot evict legitimate principals (moka
///   eviction is per-instance).
pub struct AuthCache {
    jwk: RwLock<JwkCache>,
    valid_tokens: Cache<TokenKey, CachedPrincipal>,
    invalid_tokens: Cache<TokenKey, OffsetDateTime>,
    token_hasher: TokenHasher,
}

impl AuthCache {
    pub fn new(
        valid_cache_size: u64,
        valid_cache_ttl: Duration,
        invalid_cache_size: u64,
        invalid_cache_ttl: Duration,
    ) -> AuthCache {
        AuthCache {
            jwk: RwLock::new(JwkCache {
                keys: None,
                time: OffsetDateTime::now_utc(),
                last_failure: None,
            }),
            valid_tokens: Cache::builder()
                .time_to_live(valid_cache_ttl)
                .max_capacity(valid_cache_size)
                .build(),
            invalid_tokens: Cache::builder()
                .time_to_live(invalid_cache_ttl)
                .max_capacity(invalid_cache_size)
                .build(),
            token_hasher: TokenHasher::new(),
        }
    }

    pub fn valid_token_count(&self) -> u64 {
        self.valid_tokens.entry_count()
    }

    pub fn invalid_token_count(&self) -> u64 {
        self.invalid_tokens.entry_count()
    }

    pub async fn get_keys(&self) -> Option<Arc<JwkSet>> {
        let jwk = self.jwk.read().await;
        jwk.keys.as_ref().map(Arc::clone)
    }

    fn token_key(&self, token: &str) -> TokenKey {
        self.token_hasher.hash(token)
    }
}

/// Two `RandomState`s combined to produce a 128-bit keyed fingerprint of a
/// token. Two independent hashers are required because each `RandomState`
/// produces only a 64-bit output, which would not be collision-resistant
/// across the cache's lifetime under adversarial input.
struct TokenHasher {
    s1: RandomState,
    s2: RandomState,
}

impl TokenHasher {
    fn new() -> Self {
        Self {
            s1: RandomState::new(),
            s2: RandomState::new(),
        }
    }

    fn hash(&self, token: &str) -> TokenKey {
        let bytes = token.as_bytes();
        let mut h1 = self.s1.build_hasher();
        h1.write(bytes);
        let mut h2 = self.s2.build_hasher();
        h2.write(bytes);
        [h1.finish(), h2.finish()]
    }
}

pub struct JwkCache {
    keys: Option<Arc<JwkSet>>,
    time: OffsetDateTime,
    /// Timestamp of the most recent JWKS fetch failure, used to drive the
    /// short-window negative cache that suppresses retries after an IdP error.
    /// Cleared on the next successful refresh.
    last_failure: Option<OffsetDateTime>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim_rules::parse_claim_rules;
    use serde_json::json;

    #[tokio::test]
    async fn enforce_required_claims_accepts_satisfying_claims_without_caching() {
        let config = config_requiring_appid();
        let cache = cache_for(&config);

        let result =
            enforce_required_claims("token", &claims_with(json!("web-client")), &config, &cache)
                .await;

        assert!(result.is_ok());
        assert!(!get_cached_invalid(&cache.token_key("token"), &cache).await);
    }

    #[tokio::test]
    async fn enforce_required_claims_rejects_and_caches_violating_token() {
        let config = config_requiring_appid();
        let cache = cache_for(&config);

        let result = enforce_required_claims(
            "token",
            &claims_with(json!("other-client")),
            &config,
            &cache,
        )
        .await;

        assert!(matches!(result, Err(AuthError::Unauthorized())));
        assert!(get_cached_invalid(&cache.token_key("token"), &cache).await);
        assert!(!get_cached_invalid(&cache.token_key("other-token"), &cache).await);
    }

    fn config_requiring_appid() -> AuthConfig {
        AuthConfig {
            jwt_required_claims: parse_claim_rules(
                r#"[{"claim": "appid", "operator": "in", "values": ["web-client", "cli-client"]}]"#,
            )
            .unwrap(),
            ..AuthConfig::default()
        }
    }

    fn cache_for(config: &AuthConfig) -> AuthCache {
        AuthCache::new(
            config.token_cache_size,
            config.token_ttl,
            config.invalid_token_cache_size,
            config.invalid_token_cache_ttl,
        )
    }

    fn claims_with(appid: Value) -> HashMap<String, Value> {
        HashMap::from([
            ("sub".to_string(), json!("user-1")),
            ("appid".to_string(), appid),
        ])
    }

    #[tokio::test]
    async fn decode_accepts_token_with_listed_audience_and_issuer() {
        let principal = decode_with_validated_claims(valid_claims()).await.unwrap();

        assert_eq!(principal.id, "user-1");
    }

    #[tokio::test]
    async fn decode_rejects_token_without_audience() {
        let mut claims = valid_claims();
        claims.remove("aud");

        let result = decode_with_validated_claims(claims).await;

        assert!(matches!(result, Err(AuthError::Unauthorized())));
    }

    #[tokio::test]
    async fn decode_rejects_token_with_non_string_audience() {
        let mut claims = valid_claims();
        claims.insert("aud".to_string(), json!(42));

        let result = decode_with_validated_claims(claims).await;

        assert!(matches!(result, Err(AuthError::Unauthorized())));
    }

    #[tokio::test]
    async fn decode_rejects_token_with_unlisted_audience() {
        let mut claims = valid_claims();
        claims.insert("aud".to_string(), json!("other-service"));

        let result = decode_with_validated_claims(claims).await;

        assert!(matches!(result, Err(AuthError::Unauthorized())));
    }

    #[tokio::test]
    async fn decode_rejects_token_without_issuer_when_issuers_are_configured() {
        let mut claims = valid_claims();
        claims.remove("iss");

        let result = decode_with_validated_claims(claims).await;

        assert!(matches!(result, Err(AuthError::Unauthorized())));
    }

    #[tokio::test]
    async fn decode_accepts_token_without_issuer_when_issuers_are_not_configured() {
        let mut claims = valid_claims();
        claims.remove("iss");
        let config = AuthConfig {
            jwt_issuers: None,
            ..validated_claims_config()
        };

        let result = decode_with(claims, &config).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn decode_rejects_token_that_is_not_yet_valid() {
        let mut claims = valid_claims();
        claims.insert("nbf".to_string(), json!(now_plus(3600)));

        let result = decode_with_validated_claims(claims).await;

        assert!(matches!(result, Err(AuthError::Unauthorized())));
    }

    async fn decode_with_validated_claims(
        claims: HashMap<String, Value>,
    ) -> Result<AuthPrincipal, AuthError> {
        decode_with(claims, &validated_claims_config()).await
    }

    async fn decode_with(
        claims: HashMap<String, Value>,
        config: &AuthConfig,
    ) -> Result<AuthPrincipal, AuthError> {
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(Algorithm::HS256),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(TEST_SIGNING_SECRET),
        )
        .unwrap();

        decode_principal_from_key(
            &token,
            DecodingKey::from_secret(TEST_SIGNING_SECRET),
            vec![Algorithm::HS256],
            config,
            &cache_for(config),
        )
        .await
    }

    const TEST_SIGNING_SECRET: &[u8] = b"test-signing-secret";

    fn validated_claims_config() -> AuthConfig {
        AuthConfig {
            jwt_audience: Some(vec!["permission-service".to_string()]),
            jwt_issuers: Some(vec!["https://idp.example".to_string()]),
            ..AuthConfig::default()
        }
    }

    fn valid_claims() -> HashMap<String, Value> {
        HashMap::from([
            ("sub".to_string(), json!("user-1")),
            ("aud".to_string(), json!("permission-service")),
            ("iss".to_string(), json!("https://idp.example")),
            ("exp".to_string(), json!(now_plus(600))),
        ])
    }

    fn now_plus(seconds: i64) -> i64 {
        OffsetDateTime::now_utc().unix_timestamp() + seconds
    }
}
