Storage backend
===============

Backend settings allow you to configure one or more storage backends, such as S3, MinIO, and Azure Blob, that the Storage Service deployment can reach.

In the Helm chart, storage backend settings are located under the ``config.storage`` element.

Azure Blob backend
------------------

The Azure Blob backend allows you to configure multiple Azure Blob storage endpoints.

The following settings configure the Azure Blob backend:

``config.storage.azureBlob.readRedirectSize``
  :Required: no
  :Type: size
  :Default: ``256KiB``

  Minimum object size to use redirect (presigned URL) for reads.

``config.storage.azureBlob.writeBodyMaxSize``
  :Required: no
  :Type: size
  :Default: ``2MiB``

  Maximum size for write requests through service body.

``config.storage.azureBlob.writeRedirectMaxSize``
  :Required: no
  :Type: size
  :Default: ``64MiB``

  Maximum size for write redirects (presigned URLs).

``config.storage.azureBlob.timeouts.connect``
  :Required: no
  :Type: duration
  :Default: ``60s``

  Connection establishment timeout.

``config.storage.azureBlob.timeouts.read``
  :Required: no
  :Type: duration
  :Default: ``60s``

  Read operation timeout.

``config.storage.azureBlob.timeouts.operation``
  :Required: no
  :Type: duration
  :Default: ``60s``

  Overall operation timeout.

``config.storage.azureBlob.timeouts.operationAttempt``
  :Required: no
  :Type: duration
  :Default: ``60s``

  Single operation attempt timeout.

``config.storage.azureBlob.timeouts.keepalive``
  :Required: no
  :Type: duration
  :Default: ``60s``

  Connection keep-alive timeout.

``config.storage.azureBlob.expiration.presignedExpiration``
  :Required: no
  :Type: duration
  :Default: ``24h``

  How long presigned URLs remain valid.

``config.storage.azureBlob.allowUnconfiguredEndpoints``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Allow requests to arbitrary, undeclared storage accounts within the ``*.blob.core.windows.net`` domain.

``config.storage.azureBlob.nativeMetadataEnabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Serve blob metadata. Azure returns it inline with a listing, so this backend has no fan-out setting.

``config.storage.azureBlob.poolMaxIdlePerHost``
  :Required: no
  :Type: integer

  Max idle HTTP connections per host in the reqwest pool; unset uses the reqwest default, ``0`` disables idle pooling.

``config.storage.azureBlob.endpoints``
  :Required: no
  :Type: object

  Endpoint-specific settings, see below.

Blob path validation
^^^^^^^^^^^^^^^^^^^^

Ambiguous blob paths are rejected, including path-traversal segments such as ``.`` and ``..``, and path segments ending with one or more dots, such as ``name.`` or ``name...``. Backslashes and control characters are also rejected, including their percent-encoded forms.

Use the :doc:`URL normalizer <url-normalizer>` with ``config.urlNormalizer.enabled`` set to ``true`` for consistent handling of path-traversal characters. It resolves ``.`` and ``..`` path segments before blob path validation.

Endpoint settings
^^^^^^^^^^^^^^^^^

If you want to expose one or more Azure Blob storage accounts, you will need to declare them under the ``config.storage.azureBlob.endpoints`` object, using the storage account address as the key. The address must include a scheme and the storage account's full domain name, for example ``https://<account>.blob.core.windows.net``.

Endpoints support the following configuration parameters:

``config.storage.azureBlob.endpoints.<name>.credentials``
  :Required: yes
  :Type: object

  Provide authentication credentials for this backend, see below.

``config.storage.azureBlob.endpoints.<name>.storageAccount``
  :Required: no
  :Type: string

  Override the storage account name in the API calls.

``config.storage.azureBlob.endpoints.<name>.metadataBackend``
  :Required: no
  :Type: string

  Bind the storage account to a metadata backend. See :ref:`Azure Blob metadata backend <azure-blob-metadata-backend>`.

``config.storage.azureBlob.endpoints.<name>.frontDoor``
  :Required: no
  :Type: object

  Route reads through Azure Front Door, see below.

``config.storage.azureBlob.endpoints.<name>.versioningEnabled``
  :Required: no
  :Type: boolean
  :Default: ``true``

  Explicitly enable or disable versioning support for this storage account. When omitted, versioning support is enabled.

``config.storage.azureBlob.endpoints.<name>.readOnly``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Declare this storage account read-only, so that clients present its contents without offering to modify them.

``config.storage.azureBlob.endpoints.<name>.listInTopLevelAddresses``
  :Required: no
  :Type: boolean
  :Default: ``true``

  List this backend in ``ListTopLevelAddresses``.

``config.storage.azureBlob.endpoints.<name>.listInRoutes``
  :Required: no
  :Type: boolean
  :Default: ``true``

  List this backend in ``ListRoutes`` (and, being a superset, in ``ListTopLevelAddresses``).

``config.storage.azureBlob.endpoints.<name>.permissionsEnabled``
  :Required: no
  :Type: boolean
  :Default: ``true``

  Whether to enforce permission checks for this endpoint,.

.. code-block:: yaml

  config:
    storage:
      azureBlob:
        endpoints:
          "https://mystorageacc.blob.core.windows.net":
            # Place your endpoint settings here.

.. _azure-blob-metadata-backend:

Metadata backend
""""""""""""""""

Use the ``config.storage.azureBlob.endpoints.<name>.metadataBackend`` configuration key to specify a binding between a storage account and a :doc:`metadata backend <storage-metadata>`:

.. code-block:: yaml

  config:
    storage:
      azureBlob:
        endpoints:
          "https://mystorageacc.blob.core.windows.net":
            metadataBackend: my-metadata-backend-id
            # Account-specific settings...
    metadata:
      my-metadata-backend-id:
        # Metadata backend settings...

Authentication
""""""""""""""

Backend authentication configuration is provided per storage account. For each storage account, you must either configure anonymous access explicitly or provide settings for one of the supported authentication flows.

Anonymous authentication
~~~~~~~~~~~~~~~~~~~~~~~~

Set the below parameter to ``true`` if you want to access the storage account without credentials.

``config.storage.azureBlob.endpoints.<name>.credentials.anonymous``
  :Required: yes
  :Type: boolean

  Whether to use anonymous authentication for this storage account.

Anonymous authentication grants no rights to modify the storage account, so the storage account is
always read-only, whatever ``config.storage.azureBlob.endpoints.<name>.readOnly`` is set to.

A backend configuration with anonymous authentication could look as follows:

.. code-block:: yaml

  config:
    storage:
      azureBlob:
        endpoints:
          "https://mystorageacc.blob.core.windows.net":
            credentials:
              anonymous: true

Storage key authentication
~~~~~~~~~~~~~~~~~~~~~~~~~~

You can use a storage key to authenticate against your storage account, although this is not recommended in production deployments as it provides unrestricted access to the storage account.

``config.storage.azureBlob.endpoints.<name>.credentials.storageKey.storageKey``
  :Required: yes
  :Type: string

  A key for the storage account.

A backend configuration with storage key authentication could look as follows:

.. code-block:: yaml

  config:
    storage:
      azureBlob:
        endpoints:
          # Inject the storage key from the environment variable called
          # STORAGE_SERVICE_AZURE_STORAGE_KEY.
          "https://mystorageacc.blob.core.windows.net":
            credentials:
              storageKey:
                storageKey: "${STORAGE_SERVICE_AZURE_STORAGE_KEY}"

Service principal authentication
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

As a more secure alternative, you can use client credentials of a service principal in Azure Entra ID.

``config.storage.azureBlob.endpoints.<name>.credentials.clientSecret.tenantId``
  :Required: yes
  :Type: string

  Azure Entra ID Tenant ID for service principal authentication.

``config.storage.azureBlob.endpoints.<name>.credentials.clientSecret.clientId``
  :Required: yes
  :Type: string

  Azure Entra ID Client ID for service principal authentication.

``config.storage.azureBlob.endpoints.<name>.credentials.clientSecret.clientSecret``
  :Required: yes
  :Type: string

  Azure Entra ID client secret for service principal authentication.

``config.storage.azureBlob.endpoints.<name>.credentials.clientSecret.authorityHost``
  :Required: no
  :Type: string

  Override the Azure Entra ID authority host.

A minimal configuration with service principal credentials could look as follows:

.. code-block:: yaml

  config:
    storage:
      azureBlob:
        endpoints:
          "https://mystorageacc.blob.core.windows.net":
            credentials:
              clientSecret:
                tenantId: "94e63603-d604-480a-a221-557a203d1d1e"
                clientId: "b866fb03-aca5-4278-bd84-b0fd4af748cd"
                clientSecret: "${STORAGE_SERVICE_AZURE_ENTRA_ID_CLIENT_SECRET}"

Federated Credentials (Workload Identity) authentication
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Federated Credentials authentication allows the application to exchange an access token issued by an external OIDC provider for an Azure Entra ID access token.

``config.storage.azureBlob.endpoints.<name>.credentials.federated.tenantId``
  :Required: yes
  :Type: string

  Azure Entra ID Tenant ID for federated authentication.

``config.storage.azureBlob.endpoints.<name>.credentials.federated.clientId``
  :Required: yes
  :Type: string

  Azure Entra ID Client ID for federated authentication.

``config.storage.azureBlob.endpoints.<name>.credentials.federated.federatedTokenFile``
  :Required: yes
  :Type: string

  Path to the file containing the federated identity token.

``config.storage.azureBlob.endpoints.<name>.credentials.federated.authorityHost``
  :Required: no
  :Type: string

  The authority endpoint to request tokens from.

A configuration for the `Workload Identity authentication <https://azure.github.io/azure-workload-identity/docs/>`_ could look as follows:

.. code-block:: yaml

  serviceAccount:
    create: true
    annotations:
      azure.workload.identity/client-id: "<client-id-for-the-app-registration-in-azure-entra-id>"
      azure.workload.identity/tenant-id: "<tenant-id-in-azure-entra-id>"
  podLabels:
    azure.workload.identity/use: "true"
  config:
    storage:
      azureBlob:
        endpoints:
          "https://mystorageacc.blob.core.windows.net":
            credentials:
              federated:
                # Don't inject any of these environment variables - the Workload
                # Identity addon injects them automatically.
                clientId: "${AZURE_CLIENT_ID}"
                tenantId: "${AZURE_TENANT_ID}"
                federatedTokenFile: "${AZURE_FEDERATED_TOKEN_FILE}"
                authorityHost: "${AZURE_AUTHORITY_HOST}"

Delegated token credentials
~~~~~~~~~~~~~~~~~~~~~~~~~~~

If none of the above credential flows meet your needs, you can delegate token acquisition to a separate process - for example, one running in a sidecar container. That process writes the credentials to a file with the following structure:

.. code-block:: json

  {
    "expires_on": "Token expiration timestamp in RFC 3339 format",
    "access_token": "eyJhbG.SflKxwRJSMeKK.dsF354gwwweW"
  }

The token must be a bearer token scoped to Azure Storage (scope ``https://storage.azure.com/.default``), refreshed no later than 60 seconds before expiry.

The token file provider supports the following configuration parameters:

``config.storage.azureBlob.endpoints.<name>.credentials.accessTokenFile.path``
  :Required: yes
  :Type: string

  Path to the file containing the access token.

A configuration for token file authentication could look as follows:

.. code-block:: yaml

  config:
    storage:
      azureBlob:
        endpoints:
          "https://mystorageacc.blob.core.windows.net":
            credentials:
              accessTokenFile:
                path: "/etc/secrets/azure-access-token"

Azure Front Door CDN
""""""""""""""""""""

If you have an Azure Front Door CDN distribution for your storage account, you can reference it in the endpoint settings. When configured, the Storage Service redirects blob read requests to the distribution rather than sending them to the storage account.

``config.storage.azureBlob.endpoints.<name>.frontDoor.url``
  :Required: yes
  :Type: string

  Azure Front Door endpoint URL.

A configuration referencing an Azure Front Door CDN distribution could look as follows:

.. code-block:: yaml

  config:
    storage:
      azureBlob:
        endpoints:
          "https://mystorageacc.blob.core.windows.net":
            frontDoor:
              url: https://myfrontdoordistro-jSiZFrNbYlyye4n4Me90ipT8.a03.azurefd.net

Permissions
^^^^^^^^^^^

You will need to configure the following policy for your service principal to read and write in the storage account:

.. code-block:: json

  {
    "Name": "Storage Service Blob Access",
    "Description": "Minimal permissions required for Storage Service to operate on Azure Blob Storage",
    "Actions": [
      "Microsoft.Storage/storageAccounts/blobServices/containers/read",
      "Microsoft.Storage/storageAccounts/blobServices/generateUserDelegationKey/action"
    ],
    "NotActions": [],
    "DataActions": [
      "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/read",
      "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/write",
      "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/delete",
      "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/add/action",
      "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/move/action",
      "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/tags/read",
      "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/tags/write"
    ],
    "NotDataActions": [],
    "AssignableScopes": [
      "/subscriptions/<subscription-id>/resourceGroups/<resource-group>/providers/Microsoft.Storage/storageAccounts/<storage-account>"
    ]
  }

Alternatively, you can use the built-in role of `Storage Blob Data Contributor <https://learn.microsoft.com/en-us/azure/role-based-access-control/built-in-roles/storage#storage-blob-data-contributor>`_ for the storage account. Note that this grants the service principal full control over storage containers.

S3 backend
----------

The S3 backend allows you to configure multiple S3-compatible storage endpoints, including AWS S3, MinIO, and others. The table below provides an overview of top-level settings.

``config.storage.s3.readRedirectSize``
  :Required: no
  :Type: size
  :Default: ``256KiB``

  Minimum object size to use redirect (presigned URL) for reads.

``config.storage.s3.writeBodyMaxSize``
  :Required: no
  :Type: size
  :Default: ``2MiB``

  Maximum size for write requests through service body.

``config.storage.s3.writeRedirectMaxSize``
  :Required: no
  :Type: size
  :Default: ``64MiB``

  Maximum size for write redirects (presigned URLs).

``config.storage.s3.timeouts.connect``
  :Required: no
  :Type: duration
  :Default: ``60s``

  Connection establishment timeout.

``config.storage.s3.timeouts.read``
  :Required: no
  :Type: duration
  :Default: ``60s``

  Read operation timeout.

``config.storage.s3.timeouts.operation``
  :Required: no
  :Type: duration
  :Default: ``60s``

  Overall operation timeout.

``config.storage.s3.timeouts.operationAttempt``
  :Required: no
  :Type: duration
  :Default: ``60s``

  Single operation attempt timeout.

``config.storage.s3.timeouts.keepalive``
  :Required: no
  :Type: duration
  :Default: ``60s``

  Connection keep-alive timeout.

``config.storage.s3.expiration.presignedExpiration``
  :Required: no
  :Type: duration
  :Default: ``24h``

  How long presigned URLs remain valid.

``config.storage.s3.allowUnconfiguredEndpoints``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Allow requests to arbitrary, undeclared buckets within AWS S3 domains.

``config.storage.s3.nativeMetadataEnabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Serve object ``x-amz-meta-*`` metadata.

``config.storage.s3.nativeMetadataListFanoutEnabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Fetch native metadata for each object listed by ``ListStat``. S3 does not return it with a listing, so
  this costs one request per entry.

  Enable it together with ``nativeMetadataEnabled`` unless both caches are disabled: the caches are filled
  from ``ListStat``, and the service refuses to start on the inconsistent combination.

``config.storage.s3.endpoints``
  :Required: no
  :Type: object

  Endpoint-specific settings, see below.

Endpoint settings
^^^^^^^^^^^^^^^^^

If you want to expose one or more S3 buckets, you will need to declare them under the ``config.storage.s3.endpoints`` object, using the bucket address as the key. Bucket addresses must include a scheme and hostname, and may include a path. They are formatted as follows:

* When the bucket name is encoded in the hostname of the bucket, for example: ``https://<bucketname>.s3.<region-name>.amazonaws.com``.
* When the bucket name is encoded in the first path segment, for example: ``https://s3.<region-name>.amazonaws.com/<bucket-name>``.

Endpoints support the following configuration parameters:

``config.storage.s3.endpoints.<name>.credentials``
  :Required: yes
  :Type: object

  Provide authentication credentials for this backend.

``config.storage.s3.endpoints.<name>.bucket``
  :Required: no
  :Type: string

  Override the bucket name for this bucket in API calls.

``config.storage.s3.endpoints.<name>.region``
  :Required: no
  :Type: string

  Override the region name for this bucket in API calls.

``config.storage.s3.endpoints.<name>.endpointUrl``
  :Required: no
  :Type: string

  Override the S3 API endpoint URL.

``config.storage.s3.endpoints.<name>.addressingStyle``
  :Required: no
  :Type: string
  :Default: inferred from the endpoint name

  S3 addressing style. When omitted, the style is inferred from the endpoint name: an endpoint name with a non-empty path uses path addressing; otherwise, virtual addressing is used. Supported values are:

  * ``virtual`` - Encode the bucket name in the hostname, for example ``https://<bucket>.s3.<region>.amazonaws.com/<key>``.
  * ``path`` - Encode the bucket name in the URL path, for example ``https://<host>/<bucket>/<key>``.

``config.storage.s3.endpoints.<name>.metadataBackend``
  :Required: no
  :Type: string

  Bind the bucket to a metadata backend. See :ref:`S3 metadata backend <s3-metadata-backend>`.

``config.storage.s3.endpoints.<name>.versioningEnabled``
  :Required: no
  :Type: boolean
  :Default: ``true``

  Explicitly enable or disable versioning support for this bucket. When omitted, versioning support is enabled.

``config.storage.s3.endpoints.<name>.readOnly``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Declare this bucket read-only, so that clients present its contents without offering to modify them.

``config.storage.s3.endpoints.<name>.permissionsEnabled``
  :Required: no
  :Type: boolean
  :Default: ``true``

  Whether to enforce permission checks for this endpoint,.

``config.storage.s3.endpoints.<name>.listInTopLevelAddresses``
  :Required: no
  :Type: boolean
  :Default: ``true``

  List this backend in ``ListTopLevelAddresses``.

``config.storage.s3.endpoints.<name>.listInRoutes``
  :Required: no
  :Type: boolean
  :Default: ``true``

  List this backend in ``ListRoutes`` (and, being a superset, in ``ListTopLevelAddresses``).

A configuration could look as follows:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "https://mybucket.s3.us-west-2.amazonaws.com":
            credentials:
              accessKey:
                accessKeyId: "${STORAGE_SERVICE_AWS_ACCESS_KEY_ID}"
                secretAccessKey: "${STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY}"
            # Other bucket-specific settings...
          "http://localhost.localdomain/privatebucket":
            region: "us-east-1"
            endpointUrl: "http://localhost.localdomain"
            addressingStyle: "path"
            credentials:
              accessKey:
                accessKeyId: "${STORAGE_SERVICE_MINIO_ACCESS_KEY_ID}"
                secretAccessKey: "${STORAGE_SERVICE_MINIO_SECRET_ACCESS_KEY}"
          "https://public-bucket.s3.us-east-1.amazonaws.com":
            credentials:
              anonymous: true

.. _s3-metadata-backend:

Metadata backend
""""""""""""""""

Use the ``config.storage.s3.endpoints.<name>.metadataBackend`` configuration key to specify a binding between a bucket and a :doc:`metadata backend <storage-metadata>`:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "https://mybucket.s3.us-east-1.amazonaws.com":
            metadataBackend: my-metadata-backend-id
            # Account-specific settings...
    metadata:
      my-metadata-backend-id:
        # Metadata backend settings...

Authentication
""""""""""""""

Backend authentication settings are provided per bucket. For each bucket, you must either configure anonymous access explicitly or provide settings for one of the supported authentication flows.

Anonymous authentication
~~~~~~~~~~~~~~~~~~~~~~~~

Set the below parameter to ``true`` if you want to access the bucket without credentials.

``config.storage.s3.endpoints.<name>.credentials.anonymous``
  :Required: yes
  :Type: boolean

  Whether to use anonymous authentication for this bucket.

Anonymous authentication grants no rights to modify the bucket, so the bucket is always read-only,
whatever ``config.storage.s3.endpoints.<name>.readOnly`` is set to.

A backend configuration with anonymous authentication could look as follows:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "https://publicbucket.s3.us-east-1.amazonaws.com":
            credentials:
              anonymous: true

Access Key credentials
~~~~~~~~~~~~~~~~~~~~~~

The following settings allow you to inject custom Access Key credentials to authenticate requests.

``config.storage.s3.endpoints.<name>.credentials.accessKey.accessKeyId``
  :Required: yes
  :Type: string

  AWS access key id.

``config.storage.s3.endpoints.<name>.credentials.accessKey.secretAccessKey``
  :Required: yes
  :Type: string

  AWS secret access key.

``config.storage.s3.endpoints.<name>.credentials.accessKey.sessionToken``
  :Required: no
  :Type: string

  AWS session token.

Note that like any other configuration parameters, these also support environment variable interpolation - you can always reference an environment variable instead of hardcoding the value:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "https://privatebucket.s3.us-east-1.amazonaws.com":
            credentials:
              accessKey:
                accessKeyId: "AKIAIOSFODNN7EXAMPLE"
                secretAccessKey: "${STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY}"

Assume Role credentials
~~~~~~~~~~~~~~~~~~~~~~~

If your bucket belongs to a different AWS account, you will not normally be able to access it with your service credentials. When you provide the following settings, the Storage Service will use your service credentials to assume a role in the target AWS account.

``config.storage.s3.endpoints.<name>.credentials.assumeRole.roleArn``
  :Required: yes
  :Type: string

  AWS Role ARN.

``config.storage.s3.endpoints.<name>.credentials.assumeRole.externalId``
  :Required: no
  :Type: string

  External ID for the AssumeRole credentials.

``config.storage.s3.endpoints.<name>.credentials.assumeRole.sessionName``
  :Required: no
  :Type: string

  Session name for the AssumeRole credentials.

``config.storage.s3.endpoints.<name>.credentials.assumeRole.sessionLength``
  :Required: no
  :Type: duration

  Session length for the AssumeRole credentials.

A configuration could look as follows:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          # Will assume role arn:aws:iam::111122223333:role/myrole using the
          # provided access key credentials
          "https://bucket-a.s3.us-east-1.amazonaws.com":
            credentials:
              accessKey:
                accessKeyId: "AKIAIOSFODNN7EXAMPLE"
                secretAccessKey: "${STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY}"
              assumeRole:
                roleArn: "arn:aws:iam::111122223333:role/myrole"

Web Identity Token (AWS IRSA) authentication
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

The Web Identity Token flow allows the application to exchange an access token issued by an external OIDC provider for an AWS access key.

``config.storage.s3.endpoints.<name>.credentials.webIdentityToken.webIdentityTokenFile``
  :Required: yes
  :Type: string

  Access token file location.

``config.storage.s3.endpoints.<name>.credentials.webIdentityToken.roleArn``
  :Required: yes
  :Type: string

  AWS IAM role ARN for the application.

``config.storage.s3.endpoints.<name>.credentials.webIdentityToken.roleSessionName``
  :Required: no
  :Type: string

  Session name for the token.

A configuration for the `AWS IRSA <https://docs.aws.amazon.com/eks/latest/userguide/iam-roles-for-service-accounts.html>`_ authentication could look as follows:

.. code-block:: yaml

  serviceAccount:
    create: true
    name: storage-service
    annotations:
      eks.amazonaws.com/role-arn: arn:aws:iam::<aws-account-id>:role/<aws-iam-role-name>
  config:
    storage:
      s3:
        endpoints:
          "https://myprivatebucket.s3.eu-north-1.amazonaws.com":
            credentials:
              webIdentityToken:
                # Don't inject any of the environment variables below - they
                # are injected by the IRSA addon automatically.
                webIdentityTokenFile: "${AWS_WEB_IDENTITY_TOKEN_FILE}"
                roleArn: "${AWS_ROLE_ARN}"

Delegated access key credentials
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

If none of the above credential flows meet your needs, you can delegate access key acquisition to a separate process - for example, one running in a sidecar container. That process writes the credentials to a file with the following structure:

.. code-block:: json

  {
    "expires_on": "Credentials expiration timestamp in RFC 3339 format",
    "access_key_id": "AKIAIOSFODNN7EXAMPLE",
    "secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
    "session_token": "Optional, AQoDYXdzEJr..."
  }

Credentials must be refreshed no later than 60 seconds before expiry.

The credentials file provider supports the following configuration parameters:

``config.storage.s3.endpoints.<name>.credentials.credentialsFile.path``
  :Required: yes
  :Type: string

  Path to the file containing the access key credentials.

A configuration for credentials file authentication could look as follows:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "https://mybucket.s3.us-west-1.amazonaws.com":
            credentials:
              credentialsFile:
                path: "/mnt/shared/aws-credentials"

CloudFront CDN
""""""""""""""

If you have a configured CloudFront CDN distribution for your bucket, you can reference it in the settings. If configured, the Storage Service will redirect object read requests to the CDN rather than sending them to the bucket. If your distribution is not publicly accessible, you can provide the signing key as well.

``config.storage.s3.endpoints.<name>.cloudfront.distributionUrl``
  :Required: yes
  :Type: string

  CloudFront distribution URL.

``config.storage.s3.endpoints.<name>.cloudfront.keyPairId``
  :Required: no
  :Type: string

  CloudFront key pair ID used for signing requests.

``config.storage.s3.endpoints.<name>.cloudfront.privateKeyPem``
  :Required: no
  :Type: string

  Private key encoded in PEM format.

A configuration with CloudFront CDN for a bucket that uses a private key for signing requests to the CDN could look as follows:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "https://bucket-a.s3.us-east-1.amazonaws.com":
            credentials:
              webIdentityToken:
                webIdentityTokenFile: "${AWS_WEB_IDENTITY_TOKEN_FILE}"
                roleArn: "${AWS_ROLE_ARN}"
            cloudfront:
              distributionUrl: "https://kV3gMLHdNLvEKqo8z9ezzxLC.cloudfront.net"
              keyPairId: "wXv4ZvSPL8ITIZ9s4wgvLBrb"
              privateKeyPem: "${STORAGE_SERVICE_CLOUD_FRONT_PRIVATE_KEY_PEM}"

Permissions
^^^^^^^^^^^

You will need to configure the following policy for the service role to read and write in the bucket:

.. code-block:: json

  {
    "Version": "2012-10-17",
    "Statement": [
      {
        "Sid": "BucketLevelPermissions",
        "Effect": "Allow",
        "Action": [
          "s3:ListBucket",
          "s3:ListBucketVersions"
        ],
        "Resource": "arn:aws:s3:::<bucket-name>"
      },
      {
        "Sid": "ObjectLevelPermissions",
        "Effect": "Allow",
        "Action": [
          "s3:GetObject",
          "s3:GetObjectVersion",
          "s3:PutObject",
          "s3:DeleteObject",
          "s3:DeleteObjectVersion",
          "s3:AbortMultipartUpload",
          "s3:ListMultipartUploadParts"
        ],
        "Resource": "arn:aws:s3:::<bucket-name>/*"
      }
    ]
  }
