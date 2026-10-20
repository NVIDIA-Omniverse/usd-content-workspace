# Security

This file is the entry point for understanding this service's security model. A reviewer (human or agent) who
suspects a security issue should read the model below, then consult the linked design document that covers the
affected area **before** raising a finding — many behaviors are intentional, documented decisions.

## Security model

The permission service is a centralized authorization service: it evaluates preconfigured Cedar policies and
returns allow/deny/skip decisions over equivalent REST and gRPC APIs. It does not manage users, roles, or
resources. Inbound requests are authenticated by verifying bearer tokens against an OpenID identity provider
(JWT signature verification against the IdP JWKS, and/or `userinfo` introspection); authentication can be
disabled for trusted in-cluster deployments (`openId.enabled`). Beyond the `aud` and `iss` checks, operators can
require verified tokens to satisfy additional fail-closed claim rules (`openId.requiredJwtClaims`), for example
to accept only approved client applications. Verified and rejected tokens are cached as
keyed-hash fingerprints rather than plaintext (see the token-caching design document). The service authorizes
callers for its own management endpoints via meta-permissions.

The primary trust boundary is the authenticated API surface: callers outside the cluster reach the service only
through the optional Contour `HTTPProxy` ingress, while in-cluster peers use the `ClusterIP` Service (REST on
port 3000, gRPC on port 3010). Outbound calls to the Notification Service (event publishing and consumption)
carry the service's own identity, provisioned through `serviceIdentity` (client-credentials flow or a rotating
access-token file). Policies and metadata are persisted in PostgreSQL (or read-only config files); database
credentials are supplied through a static password file or a rotating token file, optionally over TLS. Sensitive
data — bearer tokens, database credentials, and client secrets — is held only in Kubernetes Secrets and mounted
files, never rendered into ConfigMaps or logs. The container runs as a non-root user on a distroless base with a
hardened `securityContext` (read-only root filesystem, dropped capabilities, no privilege escalation).

## Security design documents

Each document records the assumptions, scope, trust boundaries, and rationale behind a specific decision. Before
raising a finding in one of these areas, check whether the behavior is an intentional, documented decision.

| Document | Covers |
|----------|--------|
| [security/jwt-audience.md](security/jwt-audience.md) | Accepted `aud` values for inbound tokens: a single explicit `openId.allowedJwtAudience` allow-list with no placeholder default, and fail-closed startup when JWT verification (`jwt` or `jwtuserinfo`) is enabled without an audience; not required for `opaque`. Tokens without `aud` (or without `iss` when issuers are configured) are rejected, and `nbf` is enforced. |
| [security/required-claims.md](security/required-claims.md) | Operator-configured claim rules enforced on verified tokens in `permission-auth`: fail-closed handling of missing, mistyped, and mis-shaped claims, startup validation, and log redaction. |
| [security/token-caching.md](security/token-caching.md) | Bearer-token caching in `permission-auth`: keyed-hash fingerprint keys, the split valid/invalid caches, and the heap-dump threat model behind them. |
| [security/url-remapper.md](security/url-remapper.md) | URL remapper rule selection: rules are matched against the canonical, dot-segment-resolved id, with source prefixes canonicalized at startup and the listing wildcard preserved. |

## Reporting a vulnerability

NVIDIA is dedicated to the security and trust of our software products and services, including all source code repositories managed through our organization.

If you need to report a security issue, please use the appropriate contact points outlined below. **Please do not report security vulnerabilities through GitHub/GitLab.**

## Reporting Potential Security Vulnerability in an NVIDIA Product

To report a potential security vulnerability in any NVIDIA product:
- Web: [Security Vulnerability Submission Form](https://www.nvidia.com/object/submit-security-vulnerability.html)
- E-Mail: psirt@nvidia.com
    - We encourage you to use the following PGP key for secure email communication: [NVIDIA public PGP Key for communication](https://www.nvidia.com/en-us/security/pgp-key)
    - Please include the following information:
   	 - Product/Driver name and version/branch that contains the vulnerability
   	 - Type of vulnerability (code execution, denial of service, buffer overflow, etc.)
   	 - Instructions to reproduce the vulnerability
   	 - Proof-of-concept or exploit code
   	 - Potential impact of the vulnerability, including how an attacker could exploit the vulnerability

While NVIDIA currently does not have a bug bounty program, we do offer acknowledgement when an externally reported security issue is addressed under our coordinated vulnerability disclosure policy. Please visit our [Product Security Incident Response Team (PSIRT)](https://www.nvidia.com/en-us/security/psirt-policies/) policies page for more information.

## NVIDIA Product Security

For all security-related concerns, please visit NVIDIA's Product Security portal at https://www.nvidia.com/en-us/security
