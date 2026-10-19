# Required claims — security assumptions

This document records the deliberate design decisions behind the operator-configured claim rules that
`crates/permission-auth` enforces on every verified bearer token (`JWT_REQUIRED_CLAIMS`, Helm
`openId.requiredJwtClaims`), so reviewers do not re-open settled questions. It covers two attack surfaces: a
token that is validly signed for an accepted audience and issuer but issued to a caller the deployment must not
accept (for example, the wrong client application or tenant), and operator misconfiguration that would silently
weaken or disable the check.

## Scope

- **Producer:** `crates/permission-auth/src/claim_rules.rs` — `parse_claim_rules` (L21–24),
  `ClaimRules::evaluate` (L31–43), `ClaimRule::is_satisfied_by` (L124–145), `ClaimValue::matches` (L149–158),
  `is_scalar` (L161–163), `contains` (L165–170), and rule validation in `RawClaimRule` / `TryFrom<RawClaimRule>`
  (L187–236). Configuration lives in `crates/permission-auth/src/config.rs` (L80–87); enforcement is
  `enforce_required_claims` in `crates/permission-auth/src/lib.rs` (L432–446), called from `opaque_auth_check`
  (L406) and `decode_principal_from_key` (L485).
- **Consumers:** the REST and gRPC authentication paths of `permission-service`, which call `verify_user_token`
  for every authenticated request; `crates/permission-auth` is also consumed by other services through the
  internal Cargo registry, which inherit the same field (empty by default).
- **Upstream dependencies:** the OpenID identity provider that issues the tokens (JWT modes) or answers
  `userinfo` (opaque mode); the Helm chart that renders `openId.requiredJwtClaims` into `JWT_REQUIRED_CLAIMS`
  with `toJson` (`helm/permission-service/templates/deployment.yaml`).

## Baseline posture

| Property | Setting | Rationale |
|----------|---------|-----------|
| Default | `[]` — no extra checks | Preserves existing deployments; the rules only narrow what `aud`/`iss` already accept. |
| Combination | Every rule must pass (AND) | An operator adding a rule can only make acceptance stricter. |
| Missing or `null` claim | Fails every operator, including `notEquals`, `notIn`, and `exists` | Fail closed: a rule never passes because the IdP omitted a claim. |
| Typing | Strict: string, integer (`i64`), boolean; no coercion, case-sensitive | `"1"` ≠ `1`, `"true"` ≠ `true`, floats and integers beyond `i64` never match. |
| Shape | Single-value operators never match arrays; array operators treat a single value as a one-element array | Prevents `["a","b"]` from passing `equals "a"`; tolerates IdPs that emit single-valued lists as scalars. |
| Invalid configuration | Rejected by the clap value parser at startup | A typo cannot silently drop a rule and leave the service accepting more than intended. |
| Rejection | `401 Unauthorized` / gRPC `UNAUTHENTICATED`; the token is recorded in the negative cache | Same outcome as any other failed token verification; retries are short-circuited. |
| Logging | Debug log with the claim name and operator only | Claim values (group ids, client ids, emails) stay out of logs. |

## Trust boundaries

- **Untrusted:** the bearer token and every claim in it until the token has been verified. The rules are
  evaluated only after verification succeeded — after signature, `exp`/`nbf`, `aud`, and `iss` checks for JWTs
  (`lib.rs` L88), or after a successful `userinfo` response for opaque tokens.
- **Trusted:** the identity provider as the source of claim values once the token is verified; the operator as
  the author of the rules; the Helm values file and the container environment.
- **Deployment assumptions:** the IdP populates the claims the rules reference; only operators can change the
  pod environment.
- **Consequences when an assumption is violated:**
  - If the IdP stops emitting a referenced claim, every token fails the rule and the service rejects all
    callers (fail closed, visible as a spike of `401`s).
  - In `jwtuserinfo` mode the rules see only the JWT claims (they are enforced while decoding the JWT, before
    `userinfo` is fetched and merged at `lib.rs` L99). A rule on a claim that only `userinfo` returns always fails; it cannot be satisfied by an
    attacker, only by fixing the configuration.
  - In `opaque` mode the claim set is the `userinfo` response, so the rules are only as trustworthy as the
    `userinfo` endpoint and the TLS channel to it.

## Threat model

The realistic attacker holds a token that the IdP legitimately issued and that passes signature, audience, and
issuer validation, but that the deployment should not accept — for example, a token issued to an unapproved
client application in a multi-client tenant, or a user outside the permitted groups.

Goals:

- Operators can restrict accepted tokens on any top-level claim with a small, predictable operator set.
- No configuration mistake widens acceptance: absence, wrong type, or wrong shape of a claim fails the rule, and
  invalid rules stop the service from starting.
- Rule evaluation adds no network I/O and no per-request allocation beyond reading the verified claims.

Non-goals:

- Replacing Cedar policies. The rules gate authentication; fine-grained authorization stays in policies, which
  also see every claim as a principal attribute.
- Nested claim paths, regular expressions, numeric ranges, or exact-array equality.
- Protecting against a compromised IdP that signs arbitrary claims.

## Design and rationale

**Decision: evaluate rules on the verified claim set, at the two points where verification succeeds.**
`enforce_required_claims` (`lib.rs` L432–446) runs right after a JWT passes decoding and validation
(`lib.rs` L485, covering `jwt` and `jwtuserinfo`) and right after `userinfo` accepts an opaque token
(`lib.rs` L406), before the principal is built. Evaluating earlier would act on unverified claims; evaluating
after the `userinfo` merge in `jwtuserinfo` would let `userinfo` data override token claims.

**Decision: record rule rejections in the negative cache.** A rule verdict is deterministic for a given token
and rule set, so `enforce_required_claims` calls `cache_invalid_token` before returning `Unauthorized`
(`lib.rs` L442). Retries with the same token are answered from `invalid_tokens` for
`invalid_token_cache_ttl` without re-verifying the signature or, in `opaque` mode, calling `userinfo` again —
the same amplification bound that applies to tokens with invalid signatures (see `security/token-caching.md`).
The negative cache keys on the keyed fingerprint only and stays separate from `valid_tokens`, so these
rejections cannot evict verified principals. Changing the rules requires a restart, which starts with an empty
cache, so a cached rejection never outlives the rule set that produced it.

**Decision: fail closed on missing, `null`, mistyped, or mis-shaped claims.** `is_satisfied_by` returns `false`
for an absent or `null` claim before looking at the operator (`claim_rules.rs` L125–128). `notEquals` and
`notIn` additionally require a scalar (`claim_rules.rs` L131–135), so a deny-list rule cannot be bypassed by an
IdP emitting the claim as an array or object. `ClaimValue::matches` compares only like types and uses
`serde_json::Number::as_i64`, which returns `None` for floats and for integers above `i64::MAX`
(serde_json 1.0.145 `src/number.rs` L130–146), so `3.0` never equals `3` and `u64::MAX` never wraps to `-1`.

**Decision: array operators accept a single value as a one-element array.** RFC 7519 section 4.1.3 permits
`aud` to be a single string when there is one audience, and IdPs vary in how they emit single-element lists.
Treating a scalar as a one-element array in `contains` (`claim_rules.rs` L165–170) keeps `containsAny` usable
for such claims without widening what the rule accepts — the scalar still has to equal a configured value. The
reverse (single-value operators accepting arrays) is deliberately not done: it would let `["allowed",
"anything"]` satisfy `equals "allowed"` and turn `notIn` into "at least one element is not denied".

**Decision: validate the configuration at startup.** `parse_claim_rules` is the clap value parser for
`JWT_REQUIRED_CLAIMS` (`config.rs` L86). `RawClaimRule` uses `deny_unknown_fields`, operators are a closed
enum, values must deserialize into string/integer/boolean, and `TryFrom<RawClaimRule>` enforces the operand
each operator needs and rejects empty `values` and blank claim names (`claim_rules.rs` L187–236). A rule that
is silently ignored would leave the service accepting tokens the operator believed were excluded.

**Decision: log only the claim name and operator.** The rejection log in `enforce_required_claims` renders
`ClaimRuleViolation`, which carries the claim name and operator but no claim values (`claim_rules.rs` L46–60),
so group memberships or identifiers from rejected tokens are not written to logs.

## Accepted trade-offs

Reviewers should not re-raise these; each is **accepted**.

- **Rule rejections share the negative cache's capacity.** A flood of distinct tokens that each fail a rule can
  fill `invalid_tokens` and evict other negative entries. Accepted for the same reasons recorded for bogus
  bearers in `security/token-caching.md`: it never touches `valid_tokens`, and entries expire within
  `invalid_token_cache_ttl`.
- **Top-level claims only.** Nested claims (for example, Keycloak's `realm_access.roles`) cannot be addressed.
  Accepted to keep the rule language small; such checks belong in Cedar policies.
- **`notEquals` / `notIn` are deny-lists.** They pass for any scalar value not listed, including values of
  another type (`notEquals 0` passes for `"0"`). Operators should prefer allow-lists (`equals`, `in`,
  `containsAny`, `containsAll`).
- **No exact-array or subset operators.** `containsAll` checks a superset only; extra elements are allowed.

## Tests that lock this in

- Operator × value-type tables, shape handling, and fail-closed cases — `crates/permission-auth/src/claim_rules.rs`:
  `equals_matches_string_integer_and_boolean_claims` (L283), `not_equals_matches_string_integer_and_boolean_claims`
  (L307), `in_matches_string_integer_and_boolean_claims` (L330), `not_in_matches_string_integer_and_boolean_claims`
  (L349), `contains_matches_string_integer_and_boolean_claims` (L377),
  `contains_any_matches_string_integer_and_boolean_claims` (L408),
  `contains_all_matches_string_integer_and_boolean_claims` (L431),
  `contains_all_accepts_scalar_claim_as_single_element_array` (L454), `exists_matches_any_present_non_null_claim`
  (L463), `integers_outside_i64_range_never_match` (L480).
- AND semantics and defaults — `sample_token_satisfies_every_documented_example` (L499),
  `evaluate_requires_every_rule_to_pass` (L530), `empty_rules_accept_any_claims` (L547).
- Startup validation — `parser_rejects_invalid_rules` (L569), `parser_error_explains_operand_requirement` (L607),
  `rules_round_trip_through_serialization` (L618).
- Enforcement and negative caching — `enforce_required_claims_accepts_satisfying_claims_without_caching` and
  `enforce_required_claims_rejects_and_caches_violating_token` in `crates/permission-auth/src/lib.rs` (L849,
  L862); `test_jwt_required_claims_are_enforced` (L3535) and
  `test_opaque_token_violating_required_claims_is_negative_cached` (L3572, a single `userinfo` call for two
  requests) in `crates/permission-service/src/tests.rs`.

## See also

- `crates/permission-auth/src/claim_rules.rs` — rule parsing, validation, and evaluation.
- `crates/permission-auth/src/lib.rs` — `enforce_required_claims`, where the rules are enforced and rejections
  are negative-cached.
- `security/token-caching.md` — the positive and negative token caches referenced above.
- Service documentation: *Authenticating with an OpenID Connect Identity Provider → Required claims*.
- [JSON Web Token (RFC 7519) §4.1.3](https://datatracker.ietf.org/doc/html/rfc7519#section-4.1.3) — `aud` as a
  single string or an array.
- [serde_json 1.0.145 `Number::as_i64`](https://docs.rs/serde_json/1.0.145/serde_json/struct.Number.html#method.as_i64)
- [Microsoft identity platform access token claims reference](https://learn.microsoft.com/entra/identity-platform/access-token-claims-reference)
  — `azp` (v2.0) and `appid` (v1.0) client application id claims.
