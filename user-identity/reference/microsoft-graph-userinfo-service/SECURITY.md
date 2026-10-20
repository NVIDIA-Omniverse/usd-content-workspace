# Security

This file is the entry point for understanding this service's security model. A reviewer (human or agent) who suspects a security issue should read the model below, then consult the linked design document that covers the affected area **before** raising a finding — many behaviors are intentional, documented decisions.

## Security model

The User Info Service is a read-only directory API. It exposes equivalent REST (`:8080`) and gRPC (`:50051`) surfaces that serve a synchronized replica of an upstream Microsoft Entra ID directory held in PostgreSQL; it never owns or mutates the source-of-truth directory. Callers authenticate with a bearer token that an upstream API gateway or service mesh is trusted to validate; the service treats the token's subject as the caller identity and forwards the same token to the Permission Service for per-endpoint and per-item authorization decisions. It handles three classes of sensitive material: caller bearer tokens, the Microsoft Graph application credential (client secret or workload-identity federated token), and PostgreSQL credentials.

The service runs in-cluster in a split reader/writer topology and ships as a distroless container (non-root user, read-only root filesystem, all Linux capabilities dropped) with the runtime security context enforced by the Helm chart. The primary trust boundaries are the inbound caller/gateway edge, the outbound Microsoft Graph and Entra ID calls, the Permission Service authorization call, and the PostgreSQL connection. The detailed reasoning behind each decision lives in the design documents below.

## Security design documents

Each document records the assumptions, scope, trust boundaries, and rationale behind a specific decision. Before raising a finding in one of these areas, check whether the behavior is an intentional, documented decision.

| Document | Covers |
|----------|--------|
| [security/docker.md](security/docker.md) | Runtime container image: distroless base, non-root user, read-only filesystem, and the vendored-source tarball shipped for OSRB. |
| [security/entra.md](security/entra.md) | Microsoft Entra ID / Graph integration: outbound query-injection defenses and the inbound caller-JWT trust boundary. |

## Security

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
