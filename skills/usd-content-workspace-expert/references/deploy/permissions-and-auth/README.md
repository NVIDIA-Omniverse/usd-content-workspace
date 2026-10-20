# Authentication and authorization — deployment pages

> **Applies to:** Envoy auth extension, Permission Service, Permission UI, Storage Permission UI, OIDC wiring · **Released:** envoy-auth-extension 2.4.0, permission-service 0.0.53, permission-ui 0.0.22 (usd-content-workspace Developer Guide 1.3.0) · **This repo's snapshot:** envoy-auth-extension 2.4.1, permission-service 0.0.54, permission-ui 0.0.22/0.0.23, storage-permission-ui 0.0.24 · **Sources:** see each page

The snapshot Envoy auth extension and Permission Service changed token-audience, issuer, and Secret values relative to the released versions; each page labels which applies.

Start with the end-to-end order in [../permissions-and-auth.md](../permissions-and-auth.md). Then:

| Page | Use it to |
|---|---|
| [oidc-and-token-audiences.md](oidc-and-token-audiences.md) | Register Entra apps, choose audiences, issuers, and principal claims that agree across charts; secrets reference; service identities |
| [envoy-auth-extension.md](envoy-auth-extension.md) | Install the edge authenticator and enable it on each service's `HTTPProxy` |
| [permission-service.md](permission-service.md) | Install the Permission Service with Postgres, credentials, notifications, Discovery, and validation |
| [cedar-policies.md](cedar-policies.md) | Seed least-privilege policies: administrators, storage subtrees, service principals, User Info |
| [permission-ui.md](permission-ui.md) | Install the admin console; embed the Storage Permission UI panel |

Related: User Info deployment [../userinfo.md](../userinfo.md); API contracts [../../apis/permissions.md](../../apis/permissions.md), [../../apis/userinfo.md](../../apis/userinfo.md); writing your own authorization backend [../../develop/custom-permissions-adapter.md](../../develop/custom-permissions-adapter.md); component maps [../../components/permission-service.md](../../components/permission-service.md), [../../components/envoy-auth-extension.md](../../components/envoy-auth-extension.md), [../../components/permission-ui.md](../../components/permission-ui.md).
