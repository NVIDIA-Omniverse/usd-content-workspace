(storage-navigator-app-troubleshooting-troubleshooting)=
# Troubleshooting

Diagnosing Storage Navigator problems is mostly about identifying which of three layers is failing: runtime configuration, discovery, or authentication. Each fails independently and surfaces differently to the user. This page covers failure modes with a verifiable cause; unless noted otherwise, none of these require the deployed stack itself to be broken.

(storage-navigator-app-troubleshooting-configuration-file-problems)=
## Configuration File Problems

Storage Navigator fetches its runtime {ref}`configuration file <storage-navigator-app-configuration-configuration>` once when the page loads.

- **Missing file.** A missing `config.json`, or a network failure fetching it, is not an error: the application falls back to built-in defaults (no default discovery, no reference links, default request timeout and batch size) and boots normally. This is expected in local development without a deployed configuration.
- **Malformed file.** A `config.json` that is present but is not valid JSON, or fails schema validation (for example a `references` entry missing a required field, or an unsafe `url` scheme), surfaces as a configuration error listing every offending field, rather than being silently ignored. Check the rendered ConfigMap content against the {ref}`Configuration Settings <storage-navigator-app-configuration-configuration-settings>` table.
- **Non-2xx response other than 404.** Any other failure fetching the file, for example a `500` from the web server, is also surfaced as a configuration error rather than falling back to defaults, so a serving problem is visible instead of silently degrading.

(storage-navigator-app-troubleshooting-discovery-problems)=
## Discovery Problems

When Storage Navigator connects to a discovery URL, whether from {ref}`default discovery <storage-navigator-app-configuration-configuration>` or entered manually, it probes for a newer discovery document format first and falls back to an older stack-listing format if that probe fails for any reason (the endpoint not existing, a transport or authentication error, or a response that does not match the expected shape). This fallback is expected behavior for a discovery service that only implements the older format. It becomes a problem only if **neither** probe succeeds, which points at the discovery URL itself being wrong or unreachable rather than at Storage Navigator.

(storage-navigator-app-troubleshooting-authentication-problems)=
## Authentication Problems

- **Sign-in never starts, or the app behaves as if authentication is disabled.** The connected stack's discovery service returned no auth configuration, that is, a `404` from its auth-configuration endpoint. Confirm the stack's discovery deployment is actually configured for authentication, if it is expected to be.
- **Sign-in fails with a client-not-found error.** The identity provider configuration returned by discovery has no OAuth client registration named `navigator`, and no `default` client to fall back to either. See {ref}`Discovering the Auth Configuration <storage-navigator-app-authentication-discovering-the-auth-configuration>`.
- **Redirected back to sign-in repeatedly, or "login results are expired".** The browser tab's session storage lost the PKCE code verifier between the redirect to the identity provider and the redirect back, for example if the callback was opened in a different tab or session storage was cleared. Restart sign-in from the beginning of the flow.
- **Session drops after being idle.** The refresh token was rejected on the next proactive or reactive refresh attempt. This is expected once a refresh token is no longer valid; the application restarts the sign-in flow automatically. See {ref}`Token Refresh <storage-navigator-app-authentication-token-refresh>`.
- **The OpenID callback route itself fails to load.** Confirm the identity provider's client registration for Storage Navigator uses a redirect URI of the deployed origin plus `/openid`, matching exactly what the application requests during sign-in.

(storage-navigator-app-troubleshooting-see-next)=
## See next

This is the last page in the reading order. Return to {ref}`Overview <storage-navigator-app-index-omniverse-storage-navigator>` for the full table of contents.
