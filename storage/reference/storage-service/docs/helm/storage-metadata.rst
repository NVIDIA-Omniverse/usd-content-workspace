Storage metadata backend
========================

Configure the metadata backend that persists key-value metadata pairs for objects and folders.

Metadata backend settings are located under the ``config.metadata`` object. Object keys serve as the application-wide backend identifiers and are used to establish a binding between storage and metadata backends. For example, the configuration below declares two metadata backends ``metadata-backend-a`` and ``metadata-backend-b`` backed by a DynamoDB table and a PostgreSQL database, respectively.

The Azure storage account ``mystorageacc.blob.core.windows.net`` and the S3 bucket ``mybucket.s3.us-west-1.amazonaws.com`` use the same DynamoDB metadata backend, while the Azure storage account ``otherstorageacc.blob.core.windows.net`` uses the PostgreSQL metadata backend. The metadata backends ``metadata-backend-c`` and ``metadata-backend-d`` are not referenced and are not configured at application startup even though they are declared.

.. code-block:: yaml

  config:
    storage:
      azureBlob:
        endpoints:
          "https://mystorageacc.blob.core.windows.net":
            credentials: ...
            metadataBackend: metadata-backend-a
          "https://otherstorageacc.blob.core.windows.net":
            credentials: ...
            metadataBackend: metadata-backend-b
      s3:
        endpoints:
          "https://mybucket.s3.us-west-1.amazonaws.com":
            credentials: ...
            metadataBackend: metadata-backend-a
    metadata:
      metadata-backend-a:
        dynamodb: ...
      metadata-backend-b:
        postgres: ...
      # You can declare and use multiple backends of the same type.
      metadata-backend-c:
        postgres: ...
      metadata-backend-d:
        cosmosdb: ...

Azure Cosmos DB backend
-----------------------

.. warning::

  The Cosmos DB metadata backend is currently non-functional. Declaring it in the configuration causes Storage Service to terminate during startup.

The Cosmos DB backend allows you to persist and query metadata in a Cosmos DB SQL container.

Prerequisites
^^^^^^^^^^^^^

Create a Cosmos DB account, a database within it, and a container within that database if they do not already exist.

When configuring the Cosmos DB account resource, make sure to use the ``Strong`` consistency level. Global distribution provides no benefit for the Storage Service and should not be enabled.

When configuring the Cosmos DB container resource, make sure:

* the partition key path is set to ``/resourceUriHash``.
* indexing excludes everything except ``/resourceUriHash``, by setting the indexing policy to ``{"includedPaths":[{"path":"/resourceUriHash/?"}],"excludedPaths":[{"path":"/*"}]}``.

Settings
^^^^^^^^

The table below provides an overview of top-level settings.

``config.metadata.<id>.cosmosdb.account``
  :Required: yes
  :Type: string

  Cosmos DB account name.

``config.metadata.<id>.cosmosdb.databaseId``
  :Required: yes
  :Type: string

  Cosmos DB database name.

``config.metadata.<id>.cosmosdb.containerId``
  :Required: yes
  :Type: string

  Cosmos DB container name.

``config.metadata.<id>.cosmosdb.credentials``
  :Required: yes
  :Type: object

  Client credentials.

``config.metadata.<id>.cosmosdb.queryBatchSize``
  :Required: no
  :Type: integer
  :Default: ``100``

  Maximum number of keys to retrieve in a query.

``config.metadata.<id>.cosmosdb.queryAllThreshold``
  :Required: no
  :Type: integer
  :Default: ``100``

  Minimum number of metadata keys, after which a non-selective query operation will be used.

``config.metadata.<id>.cosmosdb.maxConcurrency``
  :Required: no
  :Type: integer
  :Default: ``1000``

  Maximum number of simultaneous requests to the backend.

``config.metadata.<id>.cosmosdb.consistencyLevels.query``
  :Required: no
  :Type: string
  :Default: ``strong``

  Consistency level for query operations.

``config.metadata.<id>.cosmosdb.consistencyLevels.create``
  :Required: no
  :Type: string
  :Default: ``strong``

  Consistency level for create operations.

``config.metadata.<id>.cosmosdb.consistencyLevels.delete``
  :Required: no
  :Type: string
  :Default: ``strong``

  Consistency level for delete operations.

Consistency levels
^^^^^^^^^^^^^^^^^^

By default, the backend will use the `strong consistency level <https://learn.microsoft.com/en-us/azure/cosmos-db/consistency-levels#strong-consistency>`_ for all database operations. When strong consistency is not required, the consistency level for that operation can be overridden with the following values:

* ``strong`` - `strong consistency <https://learn.microsoft.com/en-us/azure/cosmos-db/consistency-levels#strong-consistency>`_.
* ``bounded`` - `bounded staleness consistency <https://learn.microsoft.com/en-us/azure/cosmos-db/consistency-levels#bounded-staleness-consistency>`_.
* ``consistent_prefix`` - `consistent prefix consistency <https://learn.microsoft.com/en-us/azure/cosmos-db/consistency-levels#consistent-prefix-consistency>`_.
* ``eventual`` - `eventual consistency <https://learn.microsoft.com/en-us/azure/cosmos-db/consistency-levels#eventual-consistency>`_.

Authentication
^^^^^^^^^^^^^^

You must provide a set of valid client credentials in order to use the backend. Client credential settings are stored under the ``config.metadata.<id>.cosmosdb.credentials`` object.

Primary key authentication
""""""""""""""""""""""""""

You can use the database primary key to authenticate your client. This method is not recommended because it provides unrestricted access to the entire database.

``config.metadata.<id>.cosmosdb.credentials.primaryKey.key``
  :Required: yes
  :Type: string

  Cosmos DB primary key value.

A backend configuration with primary key authentication could look like this:

.. code-block:: yaml

  config:
    metadata:
      metadata-backend-a:
        cosmosdb:
          account: "myaccountname"
          databaseId: "mydatabaseid"
          containerId: "mycontainerid"
          credentials:
            primaryKey:
              # Load the primary key from the environment variable
              # called STORAGE_SERVICE_COSMOSDB_PRIMARY_KEY.
              key: "${STORAGE_SERVICE_COSMOSDB_PRIMARY_KEY}"

Service principal authentication
""""""""""""""""""""""""""""""""

As a more secure alternative, you can use client credentials for a service principal in Azure Entra ID.

``config.metadata.<id>.cosmosdb.credentials.clientSecret.tenantId``
  :Required: yes
  :Type: string

  Azure Entra ID Tenant ID for service principal authentication.

``config.metadata.<id>.cosmosdb.credentials.clientSecret.clientId``
  :Required: yes
  :Type: string

  Azure Entra ID Client ID for service principal authentication.

``config.metadata.<id>.cosmosdb.credentials.clientSecret.clientSecret``
  :Required: yes
  :Type: string

  Azure Entra ID client secret for service principal authentication.

``config.metadata.<id>.cosmosdb.credentials.clientSecret.authorityHost``
  :Required: no
  :Type: string

  Override the Azure Entra ID authority host.

A minimal configuration with service principal credentials could look like this:

.. code-block:: yaml

  config:
    metadata:
      metadata-backend-a:
        cosmosdb:
          account: "myaccountname"
          databaseId: "mydatabaseid"
          containerId: "mycontainerid"
          credentials:
            clientSecret:
              tenantId: "94e63603-d604-480a-a221-557a203d1d1e"
              clientId: "b866fb03-aca5-4278-bd84-b0fd4af748cd"
              clientSecret: "${STORAGE_SERVICE_AZURE_ENTRA_ID_CLIENT_SECRET}"

Federated credentials (Workload Identity) authentication
""""""""""""""""""""""""""""""""""""""""""""""""""""""""

Federated credentials authentication allows the application to exchange an access token issued by an external OIDC provider for an Azure Entra ID access token.

``config.metadata.<id>.cosmosdb.credentials.federated.tenantId``
  :Required: yes
  :Type: string

  Azure Entra ID Tenant ID for federated authentication.

``config.metadata.<id>.cosmosdb.credentials.federated.clientId``
  :Required: yes
  :Type: string

  Azure Entra ID Client ID for federated authentication.

``config.metadata.<id>.cosmosdb.credentials.federated.federatedTokenFile``
  :Required: yes
  :Type: string

  Path to the file containing the federated identity token.

``config.metadata.<id>.cosmosdb.credentials.federated.authorityHost``
  :Required: no
  :Type: string

  The authority endpoint to request tokens from.

A configuration for `Workload Identity authentication <https://azure.github.io/azure-workload-identity/docs/>`_ could look as follows:

.. code-block:: yaml

  serviceAccount:
    create: true
    annotations:
      azure.workload.identity/client-id: "<client-id-for-the-app-registration-in-azure-entra-id>"
      azure.workload.identity/tenant-id: "<tenant-id-in-azure-entra-id>"
  podLabels:
    azure.workload.identity/use: "true"
  config:
    metadata:
      metadata-backend-a:
        cosmosdb:
          account: "myaccountname"
          databaseId: "mydatabaseid"
          containerId: "mycontainerid"
          credentials:
            federated:
              # Don't inject any of these environment variables - the Workload
              # Identity addon injects them automatically.
              clientId: "${AZURE_CLIENT_ID}"
              tenantId: "${AZURE_TENANT_ID}"
              federatedTokenFile: "${AZURE_FEDERATED_TOKEN_FILE}"
              authorityHost: "${AZURE_AUTHORITY_HOST}"

Delegated token credentials
"""""""""""""""""""""""""""

If none of the above credential flows meet your needs, you can delegate token acquisition to a separate process - for example, one running in a sidecar container. That process writes the credentials to a file with the following structure:

.. code-block:: json

  {
    "expires_on": "Token expiration timestamp in RFC 3339 format",
    "access_token": "eyJhbG.SflKxwRJSMeKK.dsF354gwwweW"
  }

The token must be a bearer token scoped to the Cosmos DB Account (scope ``https://<account>.documents.azure.com/.default``), refreshed no later than 60 seconds before expiry.

The token file provider supports the following configuration parameters:

``config.metadata.<id>.cosmosdb.credentials.accessTokenFile.path``
  :Required: yes
  :Type: string

  Path to the file containing the access token.

A configuration for token file authentication could look as follows:

.. code-block:: yaml

  config:
    metadata:
      metadata-backend-a:
        cosmosdb:
          account: "myaccountname"
          databaseId: "mydatabaseid"
          containerId: "mycontainerid"
          credentials:
            accessTokenFile:
              path: "/etc/secrets/azure-access-token"

Permissions
^^^^^^^^^^^

If you authenticate the client with a service principal, it needs the following permissions to read and write in the Container:

.. code-block:: json

  {
    "name": "Storage Service Cosmos DB ReadWrite",
    "type": "BuiltInRole",
    "assignableScopes": [
      "/subscriptions/<subscription>/resourceGroups/<resource-group>/providers/Microsoft.DocumentDB/databaseAccounts/<account>"
    ],
    "permissions": [
      {
        "dataActions": [
          "Microsoft.DocumentDB/databaseAccounts/readMetadata",
          "Microsoft.DocumentDB/databaseAccounts/sqlDatabases/containers/items/*",
          "Microsoft.DocumentDB/databaseAccounts/sqlDatabases/containers/*"
        ],
        "notDataActions": []
      }
    ]
  }

Alternatively, you can use the built-in role ``Cosmos DB Built-in Data Contributor`` if you use the database account exclusively for the Storage Service.

AWS DynamoDB backend
--------------------

.. warning::

  The DynamoDB metadata backend is currently non-functional. Declaring it in the configuration causes Storage Service to terminate during startup.

The DynamoDB backend allows you to persist and query metadata in a DynamoDB table.

Prerequisites
^^^^^^^^^^^^^

You will need to provision a DynamoDB table with the following attributes:

.. list-table::
  :header-rows: 1

  * - Attribute Name
    - Attribute Type
  * - ``ResourceUriHash``
    - String
  * - ``MetadataKey``
    - String

And the following key schema:

.. list-table::
  :header-rows: 1

  * - Attribute Name
    - Key Type
  * - ``ResourceUriHash``
    - HASH
  * - ``MetadataKey``
    - RANGE

Settings
^^^^^^^^

The table below provides an overview of top-level settings.

``config.metadata.<id>.dynamodb.table``
  :Required: yes
  :Type: string

  DynamoDB table name.

``config.metadata.<id>.dynamodb.region``
  :Required: yes
  :Type: string

  AWS region name for the Table.

``config.metadata.<id>.dynamodb.endpointUrl``
  :Required: no
  :Type: string

  DynamoDB API endpoint URL override.

``config.metadata.<id>.dynamodb.credentials``
  :Required: yes
  :Type: object

  Client credentials.

``config.metadata.<id>.dynamodb.queryThreshold``
  :Required: no
  :Type: integer
  :Default: ``100``

  Maximum number of keys in a request after which a query operation will be used instead of a batch get.

``config.metadata.<id>.dynamodb.maxConcurrency``
  :Required: no
  :Type: integer
  :Default: ``1000``

  Maximum number of simultaneous requests to the backend.

A minimal configuration with AWS IRSA credentials could look like this:

.. code-block:: yaml

  config:
    metadata:
      metadata-backend-a:
        dynamodb:
          table: "mytablename"
          region: "us-east-1"

Authentication
^^^^^^^^^^^^^^

Use the ``config.metadata.<id>.dynamodb.credentials`` object to configure client credentials for a table.

Access Key credentials
""""""""""""""""""""""

The following settings allow you to inject custom Access Key credentials that authenticate your application.

``config.metadata.<id>.dynamodb.credentials.accessKey.accessKeyId``
  :Required: yes
  :Type: string

  AWS access key id.

``config.metadata.<id>.dynamodb.credentials.accessKey.secretAccessKey``
  :Required: yes
  :Type: string

  AWS secret access key.

``config.metadata.<id>.dynamodb.credentials.accessKey.sessionToken``
  :Required: no
  :Type: string

  AWS session token.

Like all other parameters, these support environment variable interpolation:

.. code-block:: yaml

  config:
    metadata:
      metadata-backend-a:
        dynamodb:
          table: "mytablename"
          region: "us-east-1"
          credentials:
            accessKey:
              accessKeyId: "AKIAIOSFODNN7EXAMPLE"
              secretAccessKey: "${STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY}"

Web Identity Token (AWS IRSA) authentication
""""""""""""""""""""""""""""""""""""""""""""

The Web Identity Token flow allows the application to exchange an access token issued by an external OIDC provider for AWS credentials.

``config.metadata.<id>.dynamodb.credentials.webIdentityToken.webIdentityTokenFile``
  :Required: yes
  :Type: string

  Access token file location.

``config.metadata.<id>.dynamodb.credentials.webIdentityToken.roleArn``
  :Required: yes
  :Type: string

  AWS IAM role ARN for the application.

``config.metadata.<id>.dynamodb.credentials.webIdentityToken.roleSessionName``
  :Required: no
  :Type: string

  Session name for the token.

A configuration for `AWS IRSA <https://docs.aws.amazon.com/eks/latest/userguide/iam-roles-for-service-accounts.html>`_ authentication could look as follows:

.. code-block:: yaml

  serviceAccount:
    create: true
    name: storage-service
    annotations:
      eks.amazonaws.com/role-arn: arn:aws:iam::<aws-account-id>:role/<aws-iam-role-name>
  config:
    metadata:
      metadata-backend-a:
        dynamodb:
          table: "mytablename"
          region: "us-east-1"
          credentials:
            webIdentityToken:
              # Don't inject any of the environment variables below - they
              # are injected by the IRSA addon automatically.
              webIdentityTokenFile: "${AWS_WEB_IDENTITY_TOKEN_FILE}"
              roleArn: "${AWS_ROLE_ARN}"

Delegated access key credentials
""""""""""""""""""""""""""""""""

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

``config.metadata.<id>.dynamodb.credentials.credentialsFile.path``
  :Required: yes
  :Type: string

  Path to the file containing the access key credentials.

A configuration for credentials file authentication could look as follows:

.. code-block:: yaml

  config:
    metadata:
      metadata-backend-a:
        dynamodb:
          table: "mytablename"
          region: "us-east-1"
          credentials:
            credentialsFile:
              path: "/etc/secrets/aws-credentials"

Permissions
^^^^^^^^^^^

You will need to configure the following policy for the service role to access the table:

.. code-block:: json

  {
      "Version": "2012-10-17",
      "Statement": [
          {
              "Effect": "Allow",
              "Resource": "arn:aws:dynamodb:${Region}:${Account}:table/${TableName}",
              "Action": [
                  "dynamodb:BatchGetItem",
                  "dynamodb:DeleteItem",
                  "dynamodb:UpdateItem"
              ]
          }
      ]
  }

PostgreSQL backend
------------------

The PostgreSQL backend allows you to persist metadata in a PostgreSQL-compatible database.

Initialization
^^^^^^^^^^^^^^

Before starting the application, make sure that the configured user and schema both exist, and that the user has access to the schema.

Each non-dangling PostgreSQL metadata backend (that is, one that is configured and associated with at least one storage backend) will initialize the database at startup, including the creation of the necessary tables and indexes.

Settings
^^^^^^^^

The table below provides an overview of top-level settings.

``config.metadata.<id>.postgres.host``
  :Required: yes
  :Type: string

  Database host.

``config.metadata.<id>.postgres.dbname``
  :Required: yes
  :Type: string

  Database name.

``config.metadata.<id>.postgres.credentials``
  :Required: yes
  :Type: object

  Database client credentials.

``config.metadata.<id>.postgres.port``
  :Required: no
  :Type: integer
  :Default: ``5432``

  Database port.

``config.metadata.<id>.postgres.schema``
  :Required: no
  :Type: string
  :Default: ``public``

  Custom schema name.

``config.metadata.<id>.postgres.sslMode``
  :Required: no
  :Type: string
  :Default: ``prefer``

  SSL mode controls TLS usage. ``disable`` does not use TLS, ``prefer`` uses TLS when available, and ``require`` requires TLS.

``config.metadata.<id>.postgres.connectionPoolSize``
  :Required: no
  :Type: integer
  :Default: ``10``

  Database connection pool size.

``config.metadata.<id>.postgres.queryBatchSize``
  :Required: no
  :Type: integer
  :Default: ``100``

  Maximum number of metadata keys to query in a single batch.

``config.metadata.<id>.postgres.queryAllThreshold``
  :Required: no
  :Type: integer
  :Default: ``100``

  Minimum number of metadata keys required before a non-selective query operation is used.

``config.metadata.<id>.postgres.queryByteSizeBudget``
  :Required: no
  :Type: size
  :Default: ``16MiB``

  Combined metadata key, value, and ETag byte-size budget for a query response page. The budget limits response pages alongside the requested maximum number of entries. A page always includes its first entry, even when that entry alone exceeds the configured budget.

``config.metadata.<id>.postgres.maxMetadataValueSize``
  :Required: no
  :Type: size
  :Default: ``2MiB``

  Maximum encoded value size for a single metadata entry written to PostgreSQL.

A minimal configuration could look as follows:

.. code-block:: yaml

  config:
    metadata:
      metadata-backend-a:
        postgres:
          host: "somehost.somedomain"
          dbname: "mydatabase"
          credentials:
            password:
              user: "myuser"
              password: "${STORAGE_SERVICE_METADATA_BACKEND_A_POSTGRES_PASSWORD}"

Authentication
""""""""""""""

Password authentication
~~~~~~~~~~~~~~~~~~~~~~~

Provide the following settings to use static username + password pair for authentication against the database:

``config.metadata.<id>.postgres.credentials.password.user``
  :Required: yes
  :Type: string

  Database user.

``config.metadata.<id>.postgres.credentials.password.password``
  :Required: yes
  :Type: string

  Database password.

Access token file authentication
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Alternatively, you can deletgate password acquisition to a separate process - for example, one running in a sidecar container. That process writes the credentials to a file with the following structure:

.. code-block:: json

  {
    "expires_on": "Password expiration timestamp in RFC 3339 format",
    "access_token": "Access token string"
  }

Credentials must be refreshed no later than 60 seconds before expiry.

The access token file credentials provider supports the following configuration parameters:

``config.metadata.<id>.postgres.credentials.accessTokenFile.user``
  :Required: yes
  :Type: string

  Database user.

``config.metadata.<id>.postgres.credentials.accessTokenFile.path``
  :Required: yes
  :Type: string

  Path to the credentials file.

A minimal configuration with access token file credentials could look like this:

.. code-block:: yaml

  config:
    metadata:
      metadata-backend-a:
        postgres:
          host: "myhost.mydomain"
          dbname: "mydb"
          credentials:
            accessTokenFile:
              user: "myuser"
              path: "/etc/secrets/pg-password"

Metadata cache
--------------

The metadata cache holds individual metadata key values and query results in memory for a short time. When it is enabled, metadata API responses from all configured backends are cached.

Metadata cache settings are located under the ``config.metadataCache`` object:

``config.metadataCache.enabled``
  :Required: yes
  :Type: boolean
  :Default: ``false``

  Whether to enable the cache.

``config.metadataCache.cacheCapacity``
  :Required: no
  :Type: size
  :Default: ``1GiB``

  Cache capacity.

``config.metadataCache.cacheTimeToLive``
  :Required: no
  :Type: duration
  :Default: ``24h``

  Cache entry time-to-live.

``config.metadataCache.pageSizeLimit``
  :Required: no
  :Type: size
  :Default: ``3MiB``

  Maximum page size for streamed responses.

``config.metadataCache.l2Enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Whether to enable the Redis L2 tier for this cache.

``config.metadataCache.l2ClaimTimeToLive``
  :Required: no
  :Type: duration
  :Default: ``30s``

  L2 write slot expiration.

When this cache is enabled, its ``config.metadataCache.cacheCapacity`` value must be considered when configuring the container memory limit.

A minimal configuration with the cache enabled could look like this:

.. code-block:: yaml

  config:
    metadataCache:
      enabled: true

Redis/Valkey L2 cache tier
^^^^^^^^^^^^^^^^^^^^^^^^^^

The optional Redis/Valkey L2 cache tier requires the `Redis client <./redis-client.md>`_ to be configured. When enabled, the shared cache is used to exchange operation results between replicas and invalidate individual cache entries when mutating metadata operations are called.

Database migrations
-------------------

Database migrations are applied automatically whenever a metadata backend initializes. When a PostgreSQL metadata backend is configured, the chart also renders an init container that runs the service image in bootstrap mode (``STORAGE_SERVICE_BOOTSTRAP_ONLY=true``): it initializes all components - applying the migrations - and exits without serving, so replicas start only after the schema is migrated.

All PostgreSQL credential kinds work in the init container. The access-token provisioner runs as a native sidecar (an init container with ``restartPolicy: Always``, requiring Kubernetes 1.29 or later) ahead of the bootstrap container, so token files are available during the init phase. The kubelet only guarantees that the provisioner has *started* before the bootstrap container runs; if the token file has not been written yet, the bootstrap container fails and is restarted until it succeeds. To make the ordering strict, give the provisioner container a ``startupProbe`` that checks the token file, e.g.:

.. code-block:: yaml

  serviceIdentity:
    accessTokenFile:
      enabled: true
      provisionerSidecar:
        container:
          # ...
          startupProbe:
            exec:
              command: ["test", "-s", "/etc/secrets/access-token/token"]
            periodSeconds: 1
            failureThreshold: 60
