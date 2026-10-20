# URL remapper — security assumptions

This document records the deliberate design decisions behind how the optional URL remapper
(`config.urlRemapper`) matches resource ids against its rewrite rules, so reviewers do not re-open settled
questions. It covers one attack surface: a caller-chosen resource id whose textual form selects a different
rewrite rule than the URL it denotes, so that the request is evaluated against another storage location's
policies.

## Scope

- **Producer:** `crates/permission-service/src/url_remapper.rs` (`UrlRemapper::from_config` L130–157,
  `UrlRemapper::rewrite` L163–175, `canonical_source_prefix` L265–271), configured through
  `helm/permission-service/values.yaml` (`config.urlRemapper`).
- **Consumers:** every path that remaps a resource id before encoding it — `authorize`, `authorize_batch`, and
  the diagnostics endpoints in `crates/permission-service/src/service/authorization.rs`; policy creation, batch
  creation, listing, and the edit gate in `crates/permission-service/src/http/policies.rs`; and init-policy
  loading. All call `rewrite`, either directly or through `rewrite_resource_opt` / `rewrite_policy_text`.
- **Upstream dependencies:** `encode_resource_url` in `crates/permission-core/src/util/encoding.rs`, which
  parses ids with the WHATWG URL parser (`url` crate) and re-encodes each path segment.

## Baseline posture

| Property | Setting | Rationale |
|----------|---------|-----------|
| Default | Disabled (`config.urlRemapper.enabled: false`) | Ids pass through unchanged; no rule selection exists to confuse. |
| Form the rules are matched against | The canonical id produced by `encode_resource_url` | The same form policies are stored and evaluated in, so the selected rule is the one for the URL the id denotes. |
| Dot segments (`.`, `..`, `%2e`, `%2E` forms) | Resolved before matching | A dot segment can no longer place the raw text under a longer rule than the resolved URL. |
| Rule source prefixes | Canonicalized at startup, keeping the operator's trailing-slash form | Raw and canonical spellings of the same prefix (for example an uppercase host) keep matching. |
| Trailing `*` (policy-listing prefix filter) | Split off before canonicalization and re-appended | The wildcard stays recognizable downstream instead of becoming `%2A`. |
| Id that cannot be canonicalized (invalid percent-encoded UTF-8) | Not remapped | The downstream encoder rejects the id with `400`. |

## Trust boundaries

- **Untrusted:** resource ids in authorization, diagnostics, and policy-management requests, including their
  path segments.
- **Trusted:** the operator-authored rewrite rules.
- **Deployment assumptions:** resource servers resolve dot segments the way the WHATWG URL parser does before
  acting on an id. A resource server that serves the raw, unresolved path would still diverge from this
  service; that interpretation is outside this service.
- **Consequences when an assumption is violated:** if rules were matched against the raw text, a caller with
  rights under one rule's target could obtain `allow` for an object that another rule maps, and policies
  written on the canonical id would not match the dotted request.

## Threat model

The realistic attacker is an authenticated caller, or a resource server forwarding a caller-supplied id. They
send an id such as `omniverse://content.example.com/NVIDIA/../Projects/scene.usd`, which denotes
`omniverse://content.example.com/Projects/scene.usd`, hoping the raw `.../NVIDIA/` prefix selects the
`nvidia-bucket` rule.

Goals:

- The rule applied to an id is the rule for the URL it denotes after dot-segment resolution.
- Policy texts, stored policy scopes, list filters, and requests use the same matching form.

Non-goals:

- Detecting overlapping or duplicate rules after canonicalization; rule authoring is trusted.
- Normalizing ids that are not URLs. They are matched byte-exactly, as before.

## Design and rationale

**Decision: match rules against the canonical id.** Previously, `rewrite` ran `strip_prefix` on the caller's raw
string, and `encode_resource_url` resolved dot segments only afterwards. That ordering let the raw text and the
resolved URL select different rules. `rewrite` now canonicalizes the id with `encode_resource_url` first
(`url_remapper.rs` L168) and matches the result. The WHATWG URL standard treats `.`, `%2e`, `..`, `.%2e`,
`%2e.`, and `%2e%2e` (ASCII case-insensitive) as dot segments
([URL Standard — single-dot and double-dot path segments](https://url.spec.whatwg.org/#single-dot-path-segment)),
and the tests below confirm that the parser resolves them for the non-special `omniverse` scheme as well. Fixing
this once in `rewrite` covers every consumer listed in Scope, instead of reordering each call site.

**Decision: canonicalize source prefixes at startup.** Without this, matching a canonical id against a raw source
prefix would stop remapping ids whose prefix spelling changes under canonicalization (for example an uppercase
host). `canonical_source_prefix` (L265–271) applies the same encoder, but drops the `/` the serializer adds to a
host-only URL when the operator's prefix has none. That keeps the startup check that source and target trailing
slashes match meaningful.

**Decision: preserve a trailing `*`.** The policy-listing `resource` filter uses a trailing `*` as a prefix
marker, and `encode_resource_id_prefix` expects it to be literal. `rewrite` splits it off before canonicalizing
and appends it to the result (L164–167, L171). For single ids the result is unchanged, because the downstream
encoder percent-encodes the `*` either way.

## Accepted trade-offs

- **Remapped output is canonical.** A remapped id is returned in canonical form, not the caller's spelling.
  Every consumer encodes ids canonically afterwards anyway, so stored and evaluated ids are unchanged.
- **Non-URL ids are not normalized.** They pass through the encoder unchanged and are matched as raw text, as
  before.
- **Extra fields in object-form rules are ignored.** A rule value may be a target string or an object with a
  `target` field; any other object fields are dropped. Because the rules are
  trusted operator input, silently dropping unknown fields carries no caller-facing risk. An object without a
  string `target` still fails startup, so a misspelled `target` key cannot produce an empty mapping.

## Tests that lock this in

- `crates/permission-service/src/url_remapper.rs`: `rewrite_resolves_dot_segments_before_matching_nested_rule`,
  `rewrite_resolves_percent_encoded_dot_segments_before_matching`,
  `rewrite_applies_nested_rule_reached_through_dot_segments`, `rewrite_preserves_trailing_wildcard`,
  `rewrite_policy_text_resolves_dot_segments_before_matching`,
  `rewrite_matches_source_prefix_with_non_canonical_host`, and `rewrite_matches_source_prefix_without_path`.
  `yaml_object_rule_ignores_unknown_fields`, `yaml_object_rule_without_target_is_rejected`, and
  `yaml_object_rule_with_non_string_target_is_rejected` cover the object-form rule values.
- `crates/permission-service/src/tests.rs`: `test_url_remapper_authorization_resolves_dot_segments_before_matching`
  shows that a permit on the `nvidia-bucket` object does not allow a dotted id that denotes the `exampleaccount`
  object.

## See also

- Service documentation: *Deployment → URL remapper*.
- [URL Standard — path state and dot segments](https://url.spec.whatwg.org/#single-dot-path-segment)
- [RFC 3986 §5.2.4 — Remove Dot Segments](https://datatracker.ietf.org/doc/html/rfc3986#section-5.2.4)
