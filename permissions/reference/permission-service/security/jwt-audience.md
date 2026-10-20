# JWT audience — security assumptions

This document records the deliberate design decisions behind how the Helm chart configures the accepted `aud`
values for inbound bearer tokens (Helm `openId.allowedJwtAudience`, environment variable `JWT_AUDIENCE`), so
reviewers do not re-open settled questions. It covers one attack surface: a token that is validly signed by the
trusted identity provider but issued for a different application, and operator misconfiguration that would
silently widen or disable the audience check. Because the `aud`, `iss`, and `nbf` checks share one verification
setup, it also records how the presence of these claims is enforced.

## Scope

- **Producer:** `helm/permission-service/values.yaml` (L307–311, `openId.allowedJwtAudience`),
  `helm/permission-service/templates/_helpers.tpl` (L117, `permissions.jwtAudience`), and
  `helm/permission-service/templates/deployment.yaml` (L166–169), which renders `JWT_AUDIENCE` only when the list
  is non-empty.
- **Consumers:** `crates/permission-auth/src/config.rs` (L60–68, `jwt_audience`; required by `AuthConfig::load`
  at L239–245 when authentication is enabled with the `jwt` or `jwtuserinfo` type) and `decode_principal_from_key`
  in `crates/permission-auth/src/lib.rs` (L466–482), which passes the list to `jsonwebtoken`
  `Validation::set_audience`, marks `aud` (and `iss` when issuers are configured) as required, and enables `nbf`
  validation. The `opaque` type never reaches that function (`lib.rs` L127–130).
- **Upstream dependencies:** the OpenID identity provider that stamps `aud` into the tokens it issues.

## Baseline posture

| Property | Setting | Rationale |
|----------|---------|-----------|
| Source of accepted audiences | `openId.allowedJwtAudience` only | One explicit allow-list, rendered verbatim; no value is derived from other settings. |
| Default | `[]` | No placeholder audience is shipped, so an unconfigured deployment cannot accept tokens for an unintended `aud`. |
| Empty list with `openId.enabled: true` and `jwt` / `jwtuserinfo` | `JWT_AUDIENCE` is omitted; the service exits at startup | Fail closed: the service never verifies JWTs without an audience check. |
| Empty list with `openId.enabled: true` and `opaque` | The service starts; no audience is checked | Opaque tokens are validated by the IdP `userinfo` endpoint and are not decoded as JWTs, so there is no `aud` claim to check. |
| Token without `aud`, or with an `aud` that is neither a string nor a list of strings | Rejected (`401`) | Presence is enforced, not just equality; otherwise a token minted without `aud` bypasses the allow-list. |
| Token without `iss` while `JWT_ISSUERS` is non-empty | Rejected (`401`) | Same presence rule for the issuer allow-list. With no issuers configured, `iss` is not checked (see `config.rs` L70–77). |
| Token whose `nbf` is in the future (beyond `JWT_LEEWAY`) | Rejected (`401`) | RFC 7519 §4.1.5: the token must not be accepted before `nbf`. |
| Rendering | Sorted, deduplicated, comma-separated | Same shape as `JWT_ISSUERS`; the service trims entries and rejects blank ones (`config.rs` L183–193). |

## Trust boundaries

- **Untrusted:** the bearer token and its `aud` claim until signature and claim validation succeed.
- **Trusted:** the operator as the author of the allow-list; the Helm values file and the container environment.
- **Deployment assumptions:** only operators can change the pod environment; every application whose tokens the
  service must accept has its audience listed.
- **Consequences when an assumption is violated:** a missing entry rejects that application's tokens with
  `401 Unauthorized` (fail closed); an extra entry accepts tokens minted for that audience, which is why the list
  is explicit rather than derived.

## Threat model

The realistic attacker holds a token that the trusted IdP legitimately issued for another application in the
same tenant and replays it against Permission Service.

Goals:

- Only tokens whose `aud` matches an operator-listed value are accepted; a token without a usable `aud` is
  rejected.
- When issuers are configured, only tokens whose `iss` matches a listed issuer are accepted; a token without a
  usable `iss` is rejected.
- Tokens are not accepted before their `nbf` time.
- No chart default or derived value widens the accepted set.
- A deployment that verifies JWTs (`jwt` or `jwtuserinfo`) with no audience configured does not start.

Non-goals:

- Restricting which client application obtained a token for an accepted audience. That is
  `openId.requiredJwtClaims` (see `security/required-claims.md`).

## Design and rationale

**Decision: a single explicit allow-list.** Earlier chart versions built `JWT_AUDIENCE` from
`openId.clientRegistrations[*].clientId` merged with `openId.additionalJwtAudience`. The registrations carried
`name` and `scope` fields that the service never used, and the chart shipped a placeholder registration
(`some_client_id`) that became an accepted audience in any deployment that did not override it. Accepted
audiences now come only from `openId.allowedJwtAudience` (`_helpers.tpl` L117), so what the operator lists is
exactly what the service accepts.

**Decision: omit `JWT_AUDIENCE` when the list is empty and let startup fail.** `deployment.yaml` (L166–169)
renders the variable only when the helper yields a value, mirroring `JWT_ISSUERS`. `AuthConfig::load`
(`config.rs` L239–245) rejects a missing audience for the `jwt` and `jwtuserinfo` types before contacting the
IdP, and `main.rs` (L79–83) exits with `Failed to initialize authentication: JWT audience is not specified`
(verified by running the service with `ENABLE_AUTH=true`, `AUTH_TYPE=jwt`, and no `JWT_AUDIENCE`). Rendering an
empty string instead would depend on how the argument parser treats empty environment values, which is not a
behavior the chart should rely on.

**Decision: do not require an audience for `opaque`.** Opaque tokens are sent to the IdP `userinfo` endpoint and
never decoded as JWTs, so `JWT_AUDIENCE` has no effect in that mode; requiring it would force operators to invent
an unused value. The check is a negative match on `opaque` (`authentication_type != AuthType::Opaque`), so any
future JWT-based type requires an audience by default. It lives in `AuthConfig::load` rather than in the argument
declaration because clap 4.5 cannot express "required unless another argument equals a value", and its
`required_if_eq` ignores default values (`clap_builder` 4.5.41 `MatchedArg::check_explicit` returns `false` for
`ValueSource::DefaultValue`), while `AUTH_TYPE` defaults to `jwt`.

**Decision: require `aud`, and `iss` when issuers are configured, to be present.** In `jsonwebtoken` 9.3.1,
`Validation::new` requires only `exp`, and the `aud` and `iss` comparisons run only when the claim is present and
parses as a string or a list of strings; an absent or mistyped claim falls through to `_ => {}` and is accepted
([`src/validation.rs` L113–133 and L255–332](https://docs.rs/jsonwebtoken/9.3.1/src/jsonwebtoken/validation.rs.html#255-332)).
The field documentation states that validation "only happens if `aud` claim is present in the token. Adding `aud`
to `required_spec_claims` will make it required"
([`Validation`](https://docs.rs/jsonwebtoken/9.3.1/jsonwebtoken/struct.Validation.html)). `decode_principal_from_key`
therefore inserts `aud` into `required_spec_claims` whenever an audience is configured (`lib.rs` L468), and `iss`
whenever a non-empty issuer list is configured (`lib.rs` L478). A required claim must parse to count as present
(`validation.rs` L255–269), so a mistyped `aud` or `iss` is rejected with `MissingRequiredClaim`. Without this, a
token validly signed by the IdP but minted without `aud` — for example an Amazon Cognito access token issued
without a resource binding, which shares the pool's keys and `iss` with every app client
([Cognito access token reference](https://docs.aws.amazon.com/cognito/latest/developerguide/amazon-cognito-user-pools-using-the-access-token.html))
— would be accepted regardless of `JWT_AUDIENCE`.

**Decision: validate `nbf`.** `Validation::new` sets `validate_nbf: false`. `decode_principal_from_key` sets it to
`true` (`lib.rs` L481), so a token is rejected with `ImmatureSignature` while `nbf` is later than now plus
`JWT_LEEWAY`, which is the same skew already applied to `exp` (capped at 300 s, `config.rs` L23–31). `nbf` itself
stays optional, as RFC 7519 §4.1.5 defines it.

## Accepted trade-offs

Reviewers should not re-raise these; each is **accepted**.

- **No render-time validation.** The chart does not `fail` when `openId.enabled` is `true` and the list is empty;
  the error surfaces as a pod in `CrashLoopBackOff` with the startup error in its log. Accepted to keep the chart
  consistent with `JWT_ISSUERS` and to keep the service as the single place that enforces the requirement.
- **Breaking change for existing values files.** Deployments that set `openId.clientRegistrations` or
  `openId.additionalJwtAudience` must move those audiences to `openId.allowedJwtAudience`; otherwise the service
  does not start, which fails closed rather than open.

## Tests that lock this in

- `crates/permission-auth/src/config.rs`: `load_requires_jwt_audience_for_jwt_authentication_types`,
  `load_does_not_require_jwt_audience_for_opaque_authentication_type`, and
  `load_accepts_jwt_audience_for_jwt_authentication_type` cover the startup requirement per authentication type.
- **Open:** no automated test covers the chart rendering. It was verified manually with `helm template` (empty
  list omits `JWT_AUDIENCE`; `{b-aud,a-aud,b-aud}` renders `"a-aud, b-aud"`).
- `crates/permission-auth/src/lib.rs`: `decode_rejects_token_without_audience`,
  `decode_rejects_token_with_non_string_audience`, and `decode_rejects_token_with_unlisted_audience` cover the
  audience allow-list and claim presence; `decode_rejects_token_without_issuer_when_issuers_are_configured` and
  `decode_accepts_token_without_issuer_when_issuers_are_not_configured` cover issuer presence;
  `decode_rejects_token_that_is_not_yet_valid` covers `nbf`; `decode_accepts_token_with_listed_audience_and_issuer`
  guards against over-rejection.

## See also

- `security/required-claims.md` — restricting accepted tokens beyond `aud` and `iss`.
- Service documentation: *Authenticating with an OpenID Connect Identity Provider → Helm values*.
- [JSON Web Token (RFC 7519) §4.1.3](https://datatracker.ietf.org/doc/html/rfc7519#section-4.1.3) — the `aud`
  claim.
- [jsonwebtoken 9.3.1 `Validation::set_audience`](https://docs.rs/jsonwebtoken/9.3.1/jsonwebtoken/struct.Validation.html#method.set_audience)
- [jsonwebtoken 9.3.1 `Validation::required_spec_claims` and `validate_nbf`](https://docs.rs/jsonwebtoken/9.3.1/jsonwebtoken/struct.Validation.html)
- [JSON Web Token (RFC 7519) §4.1.5](https://datatracker.ietf.org/doc/html/rfc7519#section-4.1.5) — the `nbf`
  claim.
