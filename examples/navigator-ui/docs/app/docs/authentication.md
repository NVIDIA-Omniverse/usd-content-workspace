(storage-navigator-app-authentication-authentication)=
# Authentication

Storage Navigator authenticates against whichever identity provider a discovered storage stack advertises, using the OpenID Connect authorization code flow with Proof Key for Code Exchange (PKCE). Authentication is **on by default for any stack that advertises an auth configuration**, and is a per-stack property: a stack that advertises no auth configuration is treated as unauthenticated, and Storage Navigator calls its APIs without a bearer token. There is no application-level setting to force authentication on or off independent of what the stack advertises.

(storage-navigator-app-authentication-discovering-the-auth-configuration)=
## Discovering the Auth Configuration

Before authenticating, Storage Navigator asks the discovery service for that stack's auth configuration. This request can return one of two outcomes:

- **An auth configuration.** An OpenID Connect discovery document URL, plus one or more named OAuth client registrations. Storage Navigator proceeds with the authorization flow below.
- **Not found.** The discovery service reports the endpoint does not exist. Storage Navigator treats the stack as unauthenticated: no redirect happens, and API calls are sent without a bearer token.

Each client registration in the auth configuration is keyed by a client name. Storage Navigator looks up the registration named `navigator`; if the discovered stack has no `navigator`-named client, it falls back to a registration named `default`. A discovery deployment that wants to hand Storage Navigator its own OAuth client (a distinct client ID or scope from other consumers of the same stack) does so by registering a `navigator` client; otherwise every unnamed client falls through to `default`.

(storage-navigator-app-authentication-authorization-code-flow-with-pkce)=
## Authorization Code Flow with PKCE

Once an auth configuration is found, Storage Navigator:

1. Fetches the identity provider's OpenID Connect discovery document (from the URL in the auth configuration) to learn the authorization, token, userinfo, and end-session endpoints.
2. Generates a PKCE code verifier and its SHA-256 code challenge, and stores the verifier in the browser tab's session storage until the redirect returns.
3. Redirects the browser to the identity provider's authorization endpoint with `response_type=code`, the resolved client ID and scope, a random nonce, the PKCE code challenge (`S256` method), and a redirect URI pointing back at Storage Navigator's OpenID callback route. The discovery URL for the stack being authenticated is carried through this redirect as opaque state, so the callback can resume against the same stack.
4. On return, exchanges the authorization code and the PKCE code verifier for tokens at the identity provider's token endpoint, using the `authorization_code` grant.

The result is an access token (attached to every API request as a bearer token), an id token (parsed locally to display the signed-in user's email, never sent to another service), and, when the identity provider issues one, a refresh token.

```{note}
The application only decodes the id token locally to read display claims; it does not verify the token's signature. Signature verification is the identity provider's and the resource server's responsibility.
```

(storage-navigator-app-authentication-token-refresh)=
## Token Refresh

Storage Navigator refreshes the access token automatically, using the `refresh_token` grant against the identity provider's token endpoint, in two situations:

- **Proactively**, before making a request, when the current access token is within two minutes of its recorded expiry.
- **Reactively**, when a request to a discovered API returns `401 Unauthorized`. A response of `403 Forbidden` is retried once without a refresh, since some deployments return `403` transiently for reasons unrelated to token validity; requests are retried up to three times per URL before the failure is surfaced.

If a refresh attempt itself fails with `401` (the refresh token is no longer valid), Storage Navigator clears the stored session and restarts the authorization code flow described above. Concurrent requests that need a refresh at the same time share a single in-flight refresh rather than each triggering their own.

A refresh token is only usable this way when the identity provider is configured with **refresh token rotation**; without it, letting a long-lived, single-use refresh token sit in browser storage is not considered secure.

(storage-navigator-app-authentication-logout)=
## Logout

Signing out performs RP-initiated logout: Storage Navigator redirects the browser to the identity provider's end-session endpoint, passing the current id token as an authentication hint and a post-logout redirect back to the application root, so the session is ended at the identity provider rather than only locally. When the identity provider's discovery document does not advertise an end-session endpoint, Storage Navigator has no session to end there and simply returns the user to the application root, clearing only its local session state.

(storage-navigator-app-authentication-references)=
## References

- [OpenID Connect Core 1.0](https://openid.net/specs/openid-connect-core-1_0.html): the authorization code flow and the discovery document fields used above
- [RFC 7636: Proof Key for Code Exchange (PKCE)](https://www.rfc-editor.org/rfc/rfc7636): the PKCE code verifier and code challenge mechanism
- [OpenID Connect RP-Initiated Logout 1.0](https://openid.net/specs/openid-connect-rpinitiated-1_0.html): the end-session flow used for logout

(storage-navigator-app-authentication-see-next)=
## See next

- {ref}`Deployment <storage-navigator-app-deployment-deployment>`. Deploying the container and the chart's network access points.
