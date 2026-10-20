---
name: userinfo-service-docs-map
description: Locate the right Omniverse User Info Service documentation fast. Use when deploying, configuring or debugging the User Info Service — the read-only identity service that answers who a user is, what groups they belong to and which user or group matches a name — covering Microsoft Entra ID integration, directory synchronization and scoping, the static directory overlay, database, authorization, scaling, observability and troubleshooting. Routes to the authoritative docs rather than restating them.
version: 0.0.8
---

# User Info Service — documentation map

**This skill is a table of contents, not a copy of the documentation.** It exists so an agent can
find the authoritative page in one hop. Read the linked page before answering; do not answer
configuration questions from this file alone.

The documentation is split into the **API** contract (`docs/api/`) and the **service** that
implements it (`docs/service/`).

## What this component is

A centralized, **read-only** identity service. Other services ask it "who is this user?", "what
groups do they belong to?", "which user or group matches this name?" — instead of each one
integrating with the organization's identity provider separately.

It does **not own** the directory. It serves a continuously synchronized replica of the
organization's provider (currently Microsoft Entra ID). That matters when reasoning about
staleness: a change made in Entra ID appears here only after synchronization.

This repository is the **reference implementation** of the published API.

## Route by task

| If the question is about… | Go to |
|---|---|
| What the service is, and the doc index | `docs/service/docs/index.md` |
| Which chart value does what, deploying the service | `docs/service/docs/deployment.md` |
| Connecting to Microsoft Entra ID | `docs/service/docs/identity-provider.md` |
| How the replica is kept current, and how stale it can be | `docs/service/docs/directory-synchronization.md` |
| Limiting which part of the directory is served | `docs/service/docs/directory-scoping.md` |
| Serving entries that are not in the provider | `docs/service/docs/static-directory.md` |
| Database setup and configuration | `docs/service/docs/database.md` |
| Authorization via the Permission Service | `docs/service/docs/authorization.md` |
| Replicas, high availability, sizing | `docs/service/docs/scaling.md` |
| Metrics, tracing, structured logging | `docs/service/docs/observability.md` |
| An error, or unexpected lookup behaviour | `docs/service/docs/troubleshooting.md` |
| The published API contract, and where to start | `docs/api/docs/index.md` |
| REST surface | `docs/api/docs/rest-api.md` |
| gRPC surface | `docs/api/docs/grpc.md` |
| Authenticating and authorizing a caller | `docs/api/docs/authentication-and-authorization.md` |
| What callers actually do with it | `docs/api/docs/use-cases.md` |
| Related specifications | `docs/api/docs/references.md` |
| How it is built internally | `docs/sdd.md` |

## Scope

Identity **lookup**. Deciding whether a principal may do something is the `permission-service`
skill — this service answers who they are, not what they may do. Storage-side identity
configuration — the access-token sidecar, OAuth2 client credentials — is in `storage-service`
under credentials.
