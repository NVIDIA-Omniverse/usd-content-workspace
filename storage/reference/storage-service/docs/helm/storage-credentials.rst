Storage Service credentials
===========================

The Helm chart allows you to specify an access token file that is shared across application components when making API calls.

.. _access-token-file-credentials:

Access token file credentials
-----------------------------

Access token file credentials delegate token acquisition to a user-supplied sidecar container. The sidecar writes a token to a file, and the Storage Service reads it from there. When enabled, the chart automatically creates and injects an in-memory shared volume ``token-exchange`` into both the sidecar and the main container. The token file path defaults to ``/etc/secrets/access-token/token`` and is configurable via the ``path`` parameter.

The credentials file has the following structure:

.. code-block:: json

  {
    "expires_on": "Token expiration timestamp in RFC 3339 format",
    "access_token": "eyJhbG.SflKxwRJSMeKK.dsF354gwwweW"
  }

``serviceIdentity.accessTokenFile.enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Whether to use access token file credentials.

``serviceIdentity.accessTokenFile.provisionerSidecar.container``
  :Required: when ``serviceIdentity.accessTokenFile.enabled`` is set to ``true``
  :Type: object
  :Default: ``{resources: {requests: {cpu: 100m, memory: 16Mi}, limits: {cpu: 100m, memory: 32Mi}}}``

  Full specification for the sidecar container.

``serviceIdentity.accessTokenFile.path``
  :Required: no
  :Type: string
  :Default: ``/etc/secrets/access-token/token``

  Token file path.

A configuration with Access token file credentials provider could look as follows:

.. code-block:: yaml

  serviceIdentity:
    accessTokenFile:
      enabled: true
      provisionerSidecar:
        container:
          name: "storage-service-access-token"
          image: "debian:bookworm-slim"
          command:
            - /bin/bash
            - -ce
            - |
              apt update && apt install --no-install-recommends -y curl jq
              while true; do
                response=$(curl \
                  -fsSL -XPOST \
                  -H"content-type: application/x-www-form-urlencoded" \
                  -d "grant_type=client_credentials&client_id=$$OAUTH_CLIENT_ID&client_secret=$$OAUTH_CLIENT_SECRET") \
                  "https://myhost.mydomain/oauth/token"
                access_token=$(echo "$response" | jq -r .access_token)
                echo "{\"access_token\":\"$access_token\",\"expires_on\":\"2999-09-09T10:10:10Z\"}" > /etc/secrets/access-token/token

                sleep 120
              done
          envFrom:
            - secretRef:
                name: "storage-service-access-token-fetch-secrets"

Client Secret credentials
-------------------------

.. warning::

  The Client Secret credential settings are deprecated. Use :ref:`access token file credentials <access-token-file-credentials>` instead.

The Client Secret credentials are obtained by calling an OAuth2 token endpoint that supports the Client Credentials flow. The following parameters are supported:

``serviceIdentity.clientCredentials.enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Whether to use OAuth2 client credentials.

``serviceIdentity.clientCredentials.openIdConfigurationUri``
  :Required: when ``serviceIdentity.clientCredentials.enabled`` is set to ``true``
  :Type: string

  OIDC discovery URI.

``serviceIdentity.clientCredentials.clientId``
  :Required: when ``serviceIdentity.clientCredentials.enabled`` is set to ``true``
  :Type: string

  The client ID used for client credentials flow to obtain a service token.

``serviceIdentity.clientCredentials.clientSecretRef.name``
  :Required: when ``serviceIdentity.clientCredentials.enabled`` is set to ``true``
  :Type: string

  The name of a Kubernetes Secret with the client secret value.

``serviceIdentity.clientCredentials.clientSecretRef.key``
  :Required: when ``serviceIdentity.clientCredentials.enabled`` is set to ``true``
  :Type: string

  Client Secret key name in the Kubernetes Secret.

``serviceIdentity.clientCredentials.clientScope``
  :Required: no
  :Type: string

  The value for the ``scope`` field passed to the identity provider during the client credentials flow.

``serviceIdentity.clientCredentials.extraHeaders``
  :Required: no
  :Type: map[string, string]
  :Default: ``{}``

  Additional HTTP headers to include in the token exchange request.

``serviceIdentity.clientCredentials.extraBodyParams``
  :Required: no
  :Type: map[string, string]
  :Default: ``{}``

  Additional body parameters to include in the token exchange request.

A configuration with Client Secret credentials could look as follows:

.. code-block:: yaml

  serviceIdentity:
    clientCredentials:
      enabled: true
      openIdConfigurationUri: "https://login.microsoftonline.com/92616562-c449-40a4-9a58-fab4fab41786/v2.0/.well-known/openid-configuration"
      clientScope: "214243fe-6be2-4d72-bf25-a0d5c40fa25a/.default"
      clientId: "c6543e05-4fff-473e-9f4f-9168efa07e8a"
      clientSecretRef:
        name: "storage-service-service-identity"
        key: "CLIENT_SECRET"
