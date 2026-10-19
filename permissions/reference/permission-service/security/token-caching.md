# Token caching — security assumptions

This document records the deliberate design decisions behind how bearer tokens are cached in the in-memory
token cache owned by `crates/permission-auth`, so reviewers do not re-open settled questions. It covers two
attack surfaces: passive recovery of bearer tokens from process memory, and eviction abuse of the token cache
by a flood of bogus bearers. It also records a forward-looking note on replacing the hash primitive with
BLAKE3.

## Scope

- **Producer:** `crates/permission-auth/src/lib.rs` — `AuthCache` (L756–804), `TokenHasher` (L810–831),
  `TokenKey` (L743), and the cache helpers `get_cached_valid` (L293), `get_cached_invalid` (L311),
  `cache_invalid_token` (L340), and `cache_valid_token` (L355). Configuration and the startup clamp live in
  `crates/permission-auth/src/config.rs` (L129–154, L223–233).
- **Consumers:** the REST and gRPC authentication paths of `permission-service`, which call
  `verify_user_token` for every authenticated request; `crates/permission-auth` is also consumed by other
  services through the internal Cargo registry.
- **Upstream dependencies:** the OpenID identity provider (JWKS for JWT signature verification and
  `userinfo` for `JwtWithUserInfo` and `Opaque` token verification).

## Baseline posture

| Property | Setting | Rationale |
|----------|---------|-----------|
| Cache key | 128-bit keyed fingerprint `[u64; 2]`, never the token | A heap snapshot must not yield replayable bearer tokens. |
| Hash primitive | Two independent `std` `RandomState` (SipHash-1-3) instances seeded from the OS RNG per process | Keyed PRF with 128-bit output and no new third-party dependency. |
| Positive cache | `valid_tokens`, capacity 2048 (`TOKEN_CACHE_SIZE`), TTL 30 min (`TOKEN_TTL`), entry lifetime `min(principal.exp, now + token_ttl)` | Sized for steady-state traffic; never outlives the token. |
| Negative cache | `invalid_tokens`, capacity 256 (`INVALID_TOKEN_CACHE_SIZE`), TTL 60 s (`INVALID_TOKEN_CACHE_TTL`), clamped to `token_ttl` at startup | Deflects hot retry loops only; a rejected token is reconsidered within a short window. |
| Token length | Tokens longer than `MAX_TOKEN_LENGTH` (default 8192) are rejected before hashing | Bounds per-request hashing cost on the auth path. |
| Logging | Cache helpers use `tracing::instrument(skip(token, ...))` | Tokens stay out of structured logs. |

## Trust boundaries

- **Untrusted:** every bearer token presented by a caller. It is length-checked (`lib.rs` L60–64), then either
  matched against the cache by fingerprint or verified against the IdP (JWKS signature and/or `userinfo`).
- **Trusted:** the process address space holding the two `RandomState` keys, the IdP responses, and the OS RNG
  that seeds the hashers.
- **Deployment assumptions:** the service runs as a non-root, distroless container; no other workload can
  execute code inside the process. A passive, read-only snapshot of process memory (core dump, minidump, OOM
  dump, debugger on the node) is the realistic exposure.
- **Consequences when an assumption is violated:**
  - An attacker who can execute code inside the process, or intercept tokens before they reach the cache (HTTP
    middleware, a wrapping span without `skip(token)`, an outbound `Authorization` header on the wire, a
    debugger hook on `verify_user_token`), has the live token directly. The cache design does not help.
  - An attacker who extracts both `RandomState` keys from the same dump can recompute fingerprints for
    *candidate* tokens, but must still supply or guess the original token. With bearer tokens carrying at
    least 128 bits of entropy by construction, brute force is infeasible.

## Threat model

The realistic attacker obtains a passive, read-only snapshot of the process address space, for example:

- a core dump landing on a host-mounted volume in the Kubernetes node;
- a debugger or `pprof`-equivalent attached by another workload sharing the node;
- a crash reporter that ships minidumps off-cluster.

A second attacker floods the service with distinct bogus bearers to evict legitimate principals from the cache
and force expensive IdP round-trips.

Goals:

- A heap snapshot of the running process must not yield bearer tokens that can be replayed against the service
  or against the IdP.
- The cache must remain functionally a `(token -> CachedToken)` map: lookup must succeed when the same token is
  presented again.
- No new third-party dependencies (this crate is consumed by multiple services and the supply-chain surface
  should stay small).
- Auth hot-path performance must not regress meaningfully.
- A flood of bogus bearers must not evict verified principals.

Non-goals:

- Defending against an attacker with arbitrary code execution inside the process. Such an attacker can
  intercept tokens at the HTTP boundary before they ever reach the cache; no caching strategy fixes that.
- Resistance against an attacker with a heap dump *and* unbounded compute *and* a candidate-token oracle. The
  caching layer reduces, but cannot remove, the value of a heap dump.

For the passive-snapshot attacker, recovering a usable bearer token from the cache must require either
(a) brute-forcing the entropy of the token itself, or (b) compromising material that is not stored alongside
the cache.

## Design and rationale

Originally the cache was typed as `Cache<String, CachedToken>` and the cache key *was* the bearer token.
Anything that could observe the process heap would expose live, replayable credentials. Tokens are also
high-privilege: they authenticate to the permission service and (for `JwtWithUserInfo` and `Opaque` modes) can
be forwarded to the IdP's `userinfo` endpoint. The `tracing::instrument(skip(token, ...))` annotations already
kept tokens out of structured logs; storing them at rest in the cache was the last remaining plaintext copy.

**Decision: key the cache on a 128-bit keyed fingerprint, never on the token.** `TokenKey` is `[u64; 2]`
(`lib.rs` L743). `TokenHasher` owns two independent `RandomState` instances, each seeded from the OS RNG when
`AuthCache::new` runs (`lib.rs` L815–821). To derive a key, the token bytes go through both hashers and the two
64-bit outputs are concatenated:

```rust
fn hash(&self, token: &str) -> TokenKey {
    let bytes = token.as_bytes();
    let mut h1 = self.s1.build_hasher();
    h1.write(bytes);
    let mut h2 = self.s2.build_hasher();
    h2.write(bytes);
    [h1.finish(), h2.finish()]
}
```

(`lib.rs` L823–830.) `RandomState` exposes SipHash-1-3 keyed with two 64-bit secrets sampled from `getrandom`.
SipHash is a keyed PRF: with the keys held secret, the output reveals nothing useful about the input beyond
equality with other inputs hashed under the same keys. All four cache call sites (`get_cached_valid`,
`get_cached_invalid`, `cache_invalid_token`, `cache_valid_token`) route the token through
`AuthCache::token_key` (`lib.rs` L801–803) before touching moka, so the bearer string never reaches the cache.
Reversing this decision puts replayable credentials back into every heap dump.

**Decision: use two hashers, not one.** A single `RandomState` produces a 64-bit output. With a 2048-entry
cache that is astronomically safe against accidental collisions, but a single adversary-induced collision means
token A returns user B's cached principal — a hard auth bypass. Two independent SipHash-1-3 invocations give
128 bits of output, which is well outside the reach of a chosen-input collision attacker for the lifetime of any
realistic process.

**Decision: split the positive and negative caches.** The cache is split into two independent moka instances
(`lib.rs` L756–761): `valid_tokens` for verified principals and a bounded, short-TTL `invalid_tokens` negative
cache for rejected tokens — tokens that fail JWT decoding and validation, that the IdP `userinfo` endpoint
rejects, or whose verified claims violate `jwt_required_claims` (see `security/required-claims.md`). The negative cache exists only to deflect tight retry loops on the same rejected
bearer over a short window. Mixing it with the positive cache — as the prior `Cache<TokenKey, CachedToken>`
design did — turned the moka instance into an attacker-writable sink: about `token_cache_size` distinct random
bearers were enough to evict every legitimate principal. In the split design:

- `valid_tokens` is sized for steady-state traffic (default capacity 2048, TTL `token_ttl` = 30 min). Floods of
  bogus bearers never reach it.
- `invalid_tokens` is sized for hot-retry deflection only (default capacity 256, TTL
  `invalid_token_cache_ttl` = 60 s, capped at startup to `min(invalid_token_cache_ttl, token_ttl)` in
  `config.rs` L223–233). Saturating it under attack only evicts other negative entries; the cost per lookup is
  bounded and entries fall out of the cache within seconds.

Because moka eviction is per instance, an attacker cannot use one cache to push the other below its working
set.

**Decision: bound the negative-cache entry lifetime.** `cache_invalid_token` stores `now +
invalid_token_cache_ttl` (`lib.rs` L340–346), so an IdP-side change of mind (a token that was rejected once and
then re-issued, or an upstream policy flip) is reconsidered within a short, operator-controlled window rather
than being treated as permanently invalid for the rest of `token_ttl`.

**Decision: do not remove the token from memory by another scheme.** Three alternatives were considered:

1. **Cache by JWT `jti`.** Doesn't apply to opaque tokens, isn't universally issued by IdPs, and forces a full
   JWT parse before the cache lookup — losing the negative-cache fast path.
2. **No caching.** Every authorized request re-verifies the JWT signature against the JWKS and (for
   `JwtWithUserInfo` / `Opaque`) re-hits the IdP's `userinfo` endpoint. Throughput collapses; IdP load
   multiplies.
3. **Cache by token, encrypt entries.** The encryption key has to live in the same address space as the cache,
   so an attacker with a heap dump has both. Buys nothing over hashing.

The keyed-hash design is the cheapest scheme that meets the goals.

### Performance characteristics

Compared to the previous `Cache<String, CachedToken>`:

| Aspect | Before (`String` key) | After (`[u64; 2]` key) |
|---|---|---|
| Outer hash of token bytes | one SipHash pass inside moka | two SipHash passes in `TokenHasher::hash` |
| Inner hash inside moka | over the full token string | over a 16-byte key (effectively free) |
| Allocation per `insert` | `token.to_string()` heap allocation | none (`TokenKey: Copy`) |
| Per-entry resident memory for the key | full `Arc<String>` (~1–2 KiB) | 16 bytes |
| Clone on `get` | `Arc<String>` refcount bump | `[u64; 2]` `Copy` |

Approximate hot-path cost on x86 with SipHash-1-3 at ~1–2 GB/s for short messages: ~1–2 µs of additional
hashing per cache touch on a typical ~2 KiB JWT. The work the cache exists to skip — RS256 signature
verification (~50–200 µs) and JWKS / userinfo network round-trips (milliseconds) — dominates by two to four
orders of magnitude.

`TokenHasher` is `Send + Sync` and holds no mutable state under load; both `RandomState`s are immutable after
construction and a fresh `Hasher` is built per call. There is no added contention on the auth hot path.
`RandomState::new()` calls into `getrandom` twice, but only once per `AuthCache::new()` — a startup cost, not a
per-request one.

### Future direction: BLAKE3

The current design is intentionally `std`-only. If the workspace later adopts a hash crate for any other
reason — for example, content-addressed storage, deduplication of audit events, or HMAC of cross-service
request metadata — the token cache key should switch to **BLAKE3 in keyed mode** rather than continue with two
SipHash passes.

Why BLAKE3 specifically:

- **Designed for this use.** BLAKE3 ships a first-class keyed mode (`Hasher::new_keyed(&[u8; 32])`) intended
  exactly for MAC-as-table-key patterns. The 128-bit `TokenKey` would become the first 16 bytes of a BLAKE3
  keyed digest (or the full 32 bytes for headroom).
- **Faster than the alternatives.** ~2–6 GB/s on modern x86 with SIMD, versus ~1–2 GB/s for SipHash-1-3 and
  ~300 MB/s – ~2 GB/s for SHA-256 depending on SHA-NI availability.
- **One pass, one key.** Eliminates the two-`RandomState` workaround needed to reach 128 bits of output.
- **Well-vetted, audited, no native build dependency.** Pure Rust with optional SIMD intrinsics.

A migration would look roughly like:

```toml
# Cargo.toml (workspace)
blake3 = "1"
```

```rust
// crates/permission-auth/src/lib.rs
struct TokenHasher {
    key: [u8; 32],
}

impl TokenHasher {
    fn new() -> Self {
        let mut key = [0u8; 32];
        getrandom::getrandom(&mut key).expect("OS RNG must be available");
        Self { key }
    }

    fn hash(&self, token: &str) -> TokenKey {
        let digest = blake3::keyed_hash(&self.key, token.as_bytes());
        let bytes = digest.as_bytes();
        let mut a = [0u8; 8];
        let mut b = [0u8; 8];
        a.copy_from_slice(&bytes[..8]);
        b.copy_from_slice(&bytes[8..16]);
        [u64::from_le_bytes(a), u64::from_le_bytes(b)]
    }
}
```

The key change is that `TokenHasher` holds *one* secret key rather than two `RandomState`s, the per-call
allocation profile stays at zero, and the hot-path hash work over the token bytes drops from ~2× SipHash to
~1× BLAKE3 — a strict win on both throughput and code clarity. Until the workspace has another reason to take a
dependency on a hash crate, the `RandomState` design above is the right floor: it meets the threat model, costs
nothing in supply chain, and performs comparably.

## Accepted trade-offs

Reviewers should not re-raise these; each is **accepted**.

- **Fingerprints remain in memory.** A heap dump yields `[u64; 2]` fingerprints and, alongside them, the two
  `RandomState` keys. This enables equality testing of *candidate* tokens, not recovery of unknown tokens.
  Accepted because bearer tokens carry at least 128 bits of entropy.
- **In-process interception is out of scope.** Tokens are visible at the HTTP boundary and on outbound
  `Authorization` headers before they reach the cache. A finding about that belongs to the request-handling or
  outbound-client design, not to the cache.
- **SipHash instead of a dedicated MAC.** SipHash-1-3 via `RandomState` is chosen over BLAKE3 to avoid a new
  dependency; the migration path above is recorded as future work, not an open defect.
- **Negative-cache saturation under attack.** A flood of distinct bogus bearers can fill `invalid_tokens`
  (256 entries) and evict other negative entries, causing extra IdP verifications for repeated rejected
  tokens. Accepted because it never touches `valid_tokens` and entries expire within `invalid_token_cache_ttl`;
  the `token.cache.invalid.size` metric exposes persistent saturation.

## Tests that lock this in

- `test_invalid_token_flood_does_not_evict_valid_principal` — `crates/permission-service/src/tests.rs` L3313.
- `test_valid_jwt_cache_lifetime_capped_at_token_ttl` — `crates/permission-service/src/tests.rs` L3374.
- `test_invalid_token_cache_ttl_independent_from_token_ttl` — `crates/permission-service/src/tests.rs` L3430.
- `test_authentication_invalid_jwt_cache` — `crates/permission-service/src/tests.rs` L3270.
- `test_authorization_invalid_opaque_token_cache` — `crates/permission-service/src/tests.rs` L3679.
- `test_opaque_token_violating_required_claims_is_negative_cached` — `crates/permission-service/src/tests.rs`
  L3572.
- `invalid_token_cache_ttl_is_clamped_to_token_ttl` and `invalid_token_cache_ttl_is_unchanged_when_within_bound`
  — `crates/permission-auth/src/config.rs` L335 and L349.

## See also

- `crates/permission-auth/src/lib.rs` — implementation (`AuthCache`, `TokenHasher`, `TokenKey`, and the
  `cache_valid_token` / `cache_invalid_token` / `get_cached_valid` / `get_cached_invalid` helpers).
- `crates/permission-auth/src/config.rs` — `invalid_token_cache_size` and `invalid_token_cache_ttl`
  configuration knobs and the startup clamp.
- Operational metric `token.cache.invalid.size` — emits the live entry count of the negative cache; persistent
  saturation indicates an attacker or misbehaving client hammering the auth path.
- [Rust `std::collections::hash_map::RandomState`](https://doc.rust-lang.org/std/collections/hash_map/struct.RandomState.html)
- [SipHash: a fast short-input PRF](https://www.aumasson.jp/siphash/siphash.pdf)
- [BLAKE3 specification](https://github.com/BLAKE3-team/BLAKE3-specs/blob/master/blake3.pdf)
