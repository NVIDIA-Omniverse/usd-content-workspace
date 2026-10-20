Storage Service deployment guide
================================

This guide walks through an example Storage Service deployment targeting Azure Blob Storage or Amazon S3 in a Kubernetes cluster. It is intended for developers and administrators, but serves as an example rather than a deployment blueprint.

.. attention::

   This guide is not the source of truth for deployment configuration. We make every effort to keep it in sync with the Helm chart documentation, but if information is inconsistent or unclear, consult the :doc:`Helm chart reference <helm/index>`.

.. _service-identity:

Configuring service identity
----------------------------

The Storage Service uses its service identity to authenticate outbound requests, for example when publishing storage notifications or accessing blob contents.

The service identity supports an access token file mode, in which you deploy the Storage Service alongside a token provisioner sidecar, such as a workload identity helper, a script, or another application that can produce tokens. The sidecar runs in the same Pod and shares an in-memory volume called ``token-exchange``, mounted at ``/etc/secrets/access-token`` in both containers. The sidecar writes a file named ``token`` with the following format:

.. code-block:: json

  {
    "expires_on": "Token expiration timestamp in RFC 3339 format",
    "access_token": "eyJhbG.SflKxwRJSMeKK.a43@EXAMPLE"
  }

.. note::

  Your provisioner sidecar may produce additional tokens for authenticating other clients. Store them in the ``/etc/secrets/access-token`` directory. For details, see the documentation for the relevant components.

The Storage Service reads ``/etc/secrets/access-token/token`` when credentials are first requested and caches the token until it is within 60 seconds of expiry. It then reads the file again. The provisioner sidecar is responsible for writing a renewed token before that point.

To enable the Service Identity, update the values file as follows:

.. code-block:: yaml

  serviceIdentity:
    accessTokenFile:
      enabled: true
      provisionerSidecar:
        # The container spec is replaced verbatim, so you can inject any
        # arbitrary secrets, environment variables, and so on.
        container:
          name: "storage-service-access-token"
          image: "<replace-with-your-image>"
          # Replace with your actual command, if needed, or omit altogether.
          command:
            - write-token-file
            - --target
            - /etc/secrets/access-token/token


Deploying in AWS
----------------

Declaring S3 buckets
^^^^^^^^^^^^^^^^^^^^

Declare S3 buckets under the ``/config/storage/s3/endpoints`` object in the values file, using their URL as the endpoint key. Configure an explicit authentication method through the ``credentials`` property when required. The remaining parameters are inferred from the URL whenever possible. The example below defines two Amazon S3 buckets, each of which uses static credentials injected through the ``STORAGE_SERVICE_AWS_ACCESS_KEY_ID_*`` and ``STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY_*`` environment variables.

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          # Virtual-hosted bucket.
          "https://otherbucket.s3.us-west-1.amazonaws.com":
            credentials:
              accessKey:
                accessKeyId: "${STORAGE_SERVICE_AWS_ACCESS_KEY_ID_1}"
                secretAccessKey: "${STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY_1}"
          # Path-hosted bucket.
          "https://s3.us-west-1.amazonaws.com/mybucket":
            credentials:
              accessKey:
                accessKeyId: "${STORAGE_SERVICE_AWS_ACCESS_KEY_ID_2}"
                secretAccessKey: "${STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY_2}"

  # Assume that a Secret injects the STORAGE_SERVICE_AWS_ACCESS_KEY_ID_* and
  # STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY_* variables.
  extraEnvFrom:
    - secretRef:
        name: storage-service-secrets-aws

Required permissions for AWS S3 buckets
"""""""""""""""""""""""""""""""""""""""

The Storage Service's role needs the following permissions to access AWS S3 buckets:

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

CORS rules for AWS S3 buckets
"""""""""""""""""""""""""""""

Browser clients, such as Navigator, may need to access the underlying bucket directly. Configure the bucket CORS policy to allow the following headers and methods:

.. code-block:: json

  [
    {
      "AllowedHeaders": ["*"],
      "AllowedMethods": ["GET", "PUT", "POST", "DELETE", "HEAD"],
      "AllowedOrigins": ["https://your-app-domain.com", "https://*.your-domain.com"],
      "ExposeHeaders": ["ETag", "Content-Length"],
      "MaxAgeSeconds": 3600
    }
  ]

Anonymous bucket access
"""""""""""""""""""""""

When declaring buckets that must be accessed without credentials, specify the ``anonymous`` access policy explicitly:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "https://mybucket.s3.us-west-1.amazonaws.com":
            credentials:
              anonymous: true

Other S3 API providers
""""""""""""""""""""""

This guide generally assumes that you are running the application against AWS services. However, the S3 storage backend can call any S3-compatible implementation. In this scenario, provide additional settings that cannot be inferred from the bucket's hostname:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "https://myhost.mydomain:1234/mybucket":
            # Specify the bucket name to use in the S3 API calls to this bucket.
            bucket: "mybucket"
            # Specify the region name to use in the S3 API calls to this bucket.
            region: "us-east-1"
            # Specify the S3 API endpoint URL.
            endpointUrl: "https://myhost.mydomain:1234"
            # Use the Path addressing style for this bucket.
            addressingStyle: "path"
          "http://mybucket.otherhost.otherdomain:8000":
            bucket: "mybucket"
            region: "us-east-1"
            endpointUrl: "https://otherhost.otherdomain:8000"
            # Use the Virtual host name addressing style for this bucket.
            addressingStyle: "virtual"

Subscribing to AWS S3 storage events
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

The Storage Service can consume ``s3:ObjectCreated:*`` and ``s3:ObjectRemoved:*`` events from an SQS queue. These events are used to invalidate caches and, when configured, publish notifications to the Omniverse Notification API.

First, configure the S3 bucket to send object events to an SQS queue. The queue policy must allow the S3 service to publish messages:

.. code-block:: json

  {
    "Version": "2012-10-17",
    "Statement": [
      {
        "Sid": "AllowS3ToPublishEvents",
        "Effect": "Allow",
        "Principal": {
          "Service": "s3.amazonaws.com"
        },
        "Action": "sqs:SendMessage",
        "Resource": "arn:aws:sqs:<region>:<account-id>:<queue-name>",
        "Condition": {
          "ArnEquals": {
            "aws:SourceArn": "arn:aws:s3:::<bucket-name>"
          },
          "StringEquals": {
            "aws:SourceAccount": "<account-id>"
          }
        }
      }
    ]
  }

Configure the bucket notification to publish object creation and removal events to the queue:

.. code-block:: json

  {
    "QueueConfigurations": [
      {
        "QueueArn": "arn:aws:sqs:<region>:<account-id>:<queue-name>",
        "Events": [
          "s3:ObjectCreated:*",
          "s3:ObjectRemoved:*"
        ]
      }
    ]
  }

The Storage Service's role must be allowed to consume messages from the queue:

.. code-block:: json

  {
    "Version": "2012-10-17",
    "Statement": [
      {
        "Sid": "ConsumeS3Events",
        "Effect": "Allow",
        "Action": [
          "sqs:ReceiveMessage",
          "sqs:DeleteMessage",
          "sqs:GetQueueAttributes",
          "sqs:GetQueueUrl"
        ],
        "Resource": "arn:aws:sqs:<region>:<account-id>:<queue-name>"
      }
    ]
  }

Finally, declare the SQS subscriber in the values file:

.. code-block:: yaml

  config:
    storageEvents:
      # You can use any name for the subscriber ID. The backend will use it to
      # identify itself in logs, metrics, and traces.
      sqs-s3-events-1:
        sqs:
          queueUrl: "https://sqs.<region>.amazonaws.com/<account-id>/<queue-name>"
          region: "<region>"
          # Use static credentials for the client. Read on for alternative
          # authentication methods.
          credentials:
            accessKey:
              accessKeyId: "${STORAGE_SERVICE_SQS_AWS_ACCESS_KEY_ID}"
              secretAccessKey: "${STORAGE_SERVICE_SQS_AWS_SECRET_ACCESS_KEY}"
      # You can define multiple subscribers.
      sqs-s3-events-2:
        # ...

.. important::

  Storage events are consumed only when cache invalidation or the notification client is enabled.

Alternative authentication methods
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

In addition to static credentials, AWS API clients, such as S3 and SQS, support alternative authentication modes.

AWS IRSA authentication
"""""""""""""""""""""""

To use `AWS IRSA`_ authentication for SQS and S3 clients, configure the following values:

.. code-block:: yaml

  # Instruct the chart to create a service account for the Storage Service.
  # It will be automatically mounted to all containers in the Pod.
  serviceAccount:
    create: true
    annotations:
      "eks.amazonaws.com/role-arn": "arn:aws:iam::<replace-with-your-account-id>:role/<replace-with-your-role-name>"

  config:
    # Configure AWS IRSA, also known as web identity token credentials, for your S3 buckets.
    storage:
      s3:
        endpoints:
          "https://mybucket.s3.us-west-1.amazonaws.com":
            credentials:
              webIdentityToken:
                # The environment variables below will be injected by the
                # EKS admission webhook!
                webIdentityTokenFile: "${AWS_WEB_IDENTITY_TOKEN_FILE}"
                roleArn: "${AWS_ROLE_ARN}"

    # Configure AWS IRSA credentials for your SQS consumers.
    storageEvents:
      my-sqs-event-subscriber-1:
        sqs:
          queueUrl: "https://sqs.<region>.amazonaws.com/<account-id>/<queue-name>"
          region: "<region>"
          credentials:
            webIdentityToken:
              # The environment variables below will be injected by the
              # EKS admission webhook!
              webIdentityTokenFile: "${AWS_WEB_IDENTITY_TOKEN_FILE}"
              roleArn: "${AWS_ROLE_ARN}"

.. important::

  This configuration instructs the Helm chart to create a service account named ``storage-service`` in the same namespace. To verify the service account name, run ``helm template <release-name> <chart> --values <path-to-values-yaml>`` and inspect the output.

Assuming a role in a different AWS account
""""""""""""""""""""""""""""""""""""""""""

Use the ``assumeRole`` credentials to additionally assume a role in a different AWS account using the configured client credentials:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "https://foreignbucket.s3.us-west-1.amazonaws.com":
            credentials:
              accessKey:
                accessKeyId: "${STORAGE_SERVICE_OWN_AWS_ACCESS_KEY_ID}"
                secretAccessKey: "${STORAGE_SERVICE_OWN_AWS_SECRET_ACCESS_KEY}"
              # Use the access key credentials above to assume a role in a
              # different AWS account.
              assumeRole:
                roleArn: "arn:aws:iam::<foreign-account>:role/<foreign-role-name>"
                # Omit the properties below, if not needed.
                externalId: "<optional-external-id>"
                sessionName: "<optional-session-name>"
    storageEvents:
      my-sqs-event-subscriber-x:
        sqs:
          queueUrl: "https://sqs.<region>.amazonaws.com/<foreign-account>/<queue-name>"
          region: "<region>"
          credentials:
            webIdentityToken:
              webIdentityTokenFile: "${AWS_WEB_IDENTITY_TOKEN_FILE}"
              roleArn: "${AWS_ROLE_ARN}"
            # Use the AWS IRSA credentials to assume a role in a different AWS account.
            assumeRole:
              roleArn: "arn:aws:iam::<foreign-account>:role/<foreign-role-name>"
              # Omit the properties below if they are not needed.
              externalId: "<optional-external-id>"
              sessionName: "<optional-session-name>"

.. important::

  ``assumeRole`` credentials require primary static, web identity token, or credentials-file credentials. They are applied last, so multiple role assumptions cannot be chained. For nonstandard authentication flows, use :ref:`Credentials file authentication <credentials-file-auth>`.

.. _credentials-file-auth:

Credentials file authentication
"""""""""""""""""""""""""""""""

The credentials-file authentication method allows you to inject Amazon S3 or Amazon SQS API credentials into the application by using a JSON-formatted file:

.. code-block:: json

  {
    "expires_on": "Credentials expiration timestamp in RFC 3339 format",
    "access_key_id": "AKIAIOSFODNN7EXAMPLE",
    "secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
    "session_token": "Optional, AQoDYXdzEJr..."
  }

You can use a shared volume to exchange token credentials between a provisioner sidecar and the application:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "https://bucket.s3.us-east-1.amazonaws.com":
            credentials:
              credentialsFile:
                path: "/etc/secrets/aws-secrets/storage-credentials"
    storageEvents:
      event-subscriber-a:
        sqs:
          queueUrl: "https://sqs.<region>.amazonaws.com/<account-id>/<queue-name>"
          region: "<region>"
          credentials:
            credentialsFile:
              path: "/etc/secrets/aws-secrets/sqs-credentials"

  # Declare a shared volume called "aws-secrets".
  extraVolumes:
    - name: "aws-secrets"
      emptyDir:
        medium: "Memory"
        sizeLimit: "4Mi"

  # Declare a volume mount for the Storage Service container.
  extraVolumeMounts:
    - name: "aws-secrets"
      mountPath: "/etc/secrets/aws-secrets"

  # Declare a token sidecar. For S3 and SQS, it does not have to be an init container.
  extraContainers:
    - name: "inject-aws-secrets"
      image: "your-image-name"
      # Mount the secrets volume in the sidecar.
      volumeMounts:
        - name: "aws-secrets"
          mountPath: "/etc/secrets/aws-secrets"
      # Provide other settings for the sidecar container. They will be replaced
      # verbatim, for example:
      env: # ...
      envFrom: # ...
      command: # ...

.. important::

  Make sure that the credentials are renewed no later than 60 seconds before the expiration specified by their ``expires_on`` property.

Credential files can be injected by a sidecar container or a :ref:`provisioner sidecar <service-identity>` if the latter supports multiple credentials and the credential format shown above. In that case, the configuration can be simplified further:

.. code-block:: yaml

  serviceIdentity:
    accessTokenFile:
      enabled: true
      # ...

  config:
    storage:
      s3:
        endpoints:
          "https://bucket.s3.us-east-1.amazonaws.com":
            credentials:
              credentialsFile:
                path: "/etc/secrets/access-token/storage-credentials"
    storageEvents:
      event-subscriber-a:
        sqs:
          queueUrl: "https://sqs.<region>.amazonaws.com/<account-id>/<queue-name>"
          region: "<region>"
          credentials:
            credentialsFile:
              path: "/etc/secrets/access-token/sqs-credentials"


.. _AWS IRSA: https://docs.aws.amazon.com/eks/latest/userguide/iam-roles-for-service-accounts.html
.. _AWS S3 Event Notifications: https://docs.aws.amazon.com/AmazonS3/latest/userguide/EventNotifications.html


Deploying in Azure
------------------

Declaring Azure Blob storage accounts
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Azure Blob storage accounts must be declared under the ``/config/storage/azureBlob/endpoints`` object in the values file, using their URL as the endpoint key. The storage account name is inferred from the URL whenever possible. The example below defines two storage accounts that use account keys injected through the ``STORAGE_SERVICE_AZURE_STORAGE_KEY_*`` environment variables.

.. code-block:: yaml

  config:
    storage:
      azureBlob:
        endpoints:
          "https://mystorageacc.blob.core.windows.net":
            credentials:
              storageKey:
                storageKey: "${STORAGE_SERVICE_AZURE_STORAGE_KEY_1}"
          "https://otherstorageacc.blob.core.windows.net":
            credentials:
              storageKey:
                storageKey: "${STORAGE_SERVICE_AZURE_STORAGE_KEY_2}"

  # Assume that a Secret injects the STORAGE_SERVICE_AZURE_STORAGE_KEY_* variables.
  extraEnvFrom:
    - secretRef:
        name: storage-service-secrets-azure

Required permissions for Azure Blob storage accounts
""""""""""""""""""""""""""""""""""""""""""""""""""""

When using Microsoft Entra credentials, the Storage Service's service principal or managed identity needs permission to read and write blobs and to generate user delegation keys. You can assign the built-in `Storage Blob Data Contributor`_ role at the storage account scope, or create a custom role with the following permissions:

.. code-block:: json

  {
    "Name": "Storage Service Blob Access",
    "IsCustom": true,
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

CORS rules for Azure Blob storage accounts
""""""""""""""""""""""""""""""""""""""""""

Browser clients, such as Navigator, may need to access the underlying storage account directly. Configure the Blob service CORS policy to allow the following headers and methods:

Use the Azure CLI to add a CORS rule to the Blob service properties of the storage account:

.. code-block:: shell

  az storage account blob-service-properties cors-rule add \
    --account-name "<storage-account>" \
    --resource-group "<resource-group>" \
    --allowed-origins "https://your-app-domain.com" "https://*.your-domain.com" \
    --allowed-methods GET PUT POST DELETE HEAD \
    --allowed-headers "*" \
    --exposed-headers "ETag" "Content-Length" \
    --max-age 3600

Run ``az storage account blob-service-properties cors-rule list --account-name "<storage-account>" --resource-group "<resource-group>"`` to inspect the configured rules.

Anonymous storage account access
""""""""""""""""""""""""""""""""

When declaring storage accounts that must be accessed without credentials, specify the ``anonymous`` access policy explicitly:

.. code-block:: yaml

  config:
    storage:
      azureBlob:
        endpoints:
          "https://publicstorageacc.blob.core.windows.net":
            credentials:
              anonymous: true

The storage account and container must also permit anonymous blob access.

Subscribing to Azure Blob storage events
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

The Storage Service can consume ``Microsoft.Storage.BlobCreated`` and ``Microsoft.Storage.BlobDeleted`` events from an Azure Service Bus queue. These events are used to invalidate caches and, when configured, publish notifications to the Omniverse Notification API.

First, create an Event Grid system topic for the storage account:

.. code-block:: shell

  az eventgrid system-topic create \
    --resource-group "<resource-group>" \
    --name "<system-topic-name>" \
    --location "<location>" \
    --topic-type "Microsoft.Storage.StorageAccounts" \
    --source "/subscriptions/<subscription-id>/resourceGroups/<resource-group>/providers/Microsoft.Storage/storageAccounts/<storage-account>"

Then configure an event subscription that sends events to the Service Bus queue. The event subscription must use the CloudEvents 1.0 delivery schema:

.. code-block:: shell

  az eventgrid system-topic event-subscription create \
    --resource-group "<resource-group>" \
    --system-topic-name "<system-topic-name>" \
    --name "<event-subscription-name>" \
    --endpoint-type "servicebusqueue" \
    --event-delivery-schema "cloudeventschemav1_0" \
    --included-event-types "Microsoft.Storage.BlobCreated" "Microsoft.Storage.BlobDeleted" \
    --endpoint "/subscriptions/<subscription-id>/resourceGroups/<resource-group>/providers/Microsoft.ServiceBus/namespaces/<namespace-name>/queues/<queue-name>"

When authenticating with Microsoft Entra credentials, assign the Storage Service's service principal or managed identity the built-in `Azure Service Bus Data Receiver`_ role on the queue. When authenticating with a shared access policy, the policy must have the ``Listen`` right.

Finally, declare the Service Bus subscriber in the values file:

.. code-block:: yaml

  config:
    storageEvents:
      # You can use any name for the subscriber ID. The backend will use it to identify itself in logs, metrics, and traces.
      azure-blob-events-1:
        azureServiceBus:
          queueNamespace: "<namespace-name>"
          queueName: "<queue-name>"
          credentials:
            sharedAccessPolicyKey:
              policyName: "<policy-name>"
              policyKey: "${STORAGE_SERVICE_AZURE_SERVICE_BUS_POLICY_KEY}"
      # You can define multiple subscribers.
      azure-blob-events-2:
        # ...

  # Assume that the environment variable above is injected from a secret.
  extraEnvFrom:
    - secretRef:
        name: storage-service-secrets-azure-service-bus

.. important::

  Storage events are consumed only when cache invalidation or the notification client is enabled.

Alternative authentication methods
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Aside from storage account keys and Service Bus shared access policies, Azure clients support alternative authentication modes.

Service identity authentication
"""""""""""""""""""""""""""""""

When an access-token-file service identity is configured under the ``/serviceIdentity/accessTokenFile`` object, Azure Blob and Azure Service Bus clients use its access token when the ``credentials`` property is omitted from their configuration. By default, the clients read the token from ``/etc/secrets/access-token/token``.

.. code-block:: yaml

  serviceIdentity:
    accessTokenFile:
      enabled: true
      # The default path is /etc/secrets/access-token/token.
      # ...

  config:
    storage:
      azureBlob:
        endpoints:
          # The credentials property is omitted, so this client uses the access-token-file service identity.
          "https://mystorageacc.blob.core.windows.net": {}
    storageEvents:
      azure-blob-events-1:
        azureServiceBus:
          queueNamespace: "<namespace-name>"
          queueName: "<queue-name>"
          # The credentials property is omitted, so this client uses the access-token-file service identity.

For instructions on configuring the provider, see :ref:`Configuring the Service Identity <service-identity>`.

Azure Workload Identity authentication
""""""""""""""""""""""""""""""""""""""

To use `Azure Workload Identity`_ authentication for Azure Blob and Service Bus clients, first create a federated identity credential for the Kubernetes service account. Then configure the following values:

.. code-block:: yaml

  serviceAccount:
    create: true
    annotations:
      "azure.workload.identity/client-id": "<client-id>"
      "azure.workload.identity/tenant-id": "<tenant-id>"

  podLabels:
    "azure.workload.identity/use": "true"

  config:
    storage:
      azureBlob:
        endpoints:
          "https://mystorageacc.blob.core.windows.net":
            credentials:
              federated:
                # The Azure Workload Identity webhook injects these environment variables and mounts the federated token file.
                clientId: "${AZURE_CLIENT_ID}"
                tenantId: "${AZURE_TENANT_ID}"
                federatedTokenFile: "${AZURE_FEDERATED_TOKEN_FILE}"
                authorityHost: "${AZURE_AUTHORITY_HOST}"
    storageEvents:
      azure-blob-events-1:
        azureServiceBus:
          queueNamespace: "<namespace-name>"
          queueName: "<queue-name>"
          credentials:
            federated:
              clientId: "${AZURE_CLIENT_ID}"
              tenantId: "${AZURE_TENANT_ID}"
              federatedTokenFile: "${AZURE_FEDERATED_TOKEN_FILE}"
              authorityHost: "${AZURE_AUTHORITY_HOST}"

.. important::

  This configuration instructs the Helm chart to create a service account named ``storage-service`` in the same namespace. To verify the service account name, run ``helm template <release-name> <chart> --values <path-to-values-yaml>`` and inspect the output.

Access token file authentication
""""""""""""""""""""""""""""""""

The Access Token File authentication method allows a separate process to inject Azure access tokens into the application. Each file must contain JSON in the following format:

.. code-block:: json

  {
    "expires_on": "Token expiration timestamp in RFC 3339 format",
    "access_token": "eyJhbG.SflKxwRJSMeKK.a43@EXAMPLE"
  }

Azure Blob and Service Bus require tokens for different audiences. Use a token scoped to ``https://storage.azure.com/.default`` for Blob storage and a token scoped to ``https://servicebus.azure.net/.default`` for Service Bus.

You can use an :ref:`access-token-file service identity <service-identity>` provisioner sidecar to write both tokens into the shared directory:

.. code-block:: yaml

  serviceIdentity:
    accessTokenFile:
      enabled: true
      provisionerSidecar:
        container:
          name: "inject-azure-access-tokens"
          image: "<replace-with-your-image>"
          # Configure the sidecar to write storage-token and service-bus-token into /etc/secrets/access-token.

  config:
    storage:
      azureBlob:
        endpoints:
          "https://mystorageacc.blob.core.windows.net":
            credentials:
              accessTokenFile:
                path: "/etc/secrets/access-token/storage-token"
    storageEvents:
      azure-blob-events-1:
        azureServiceBus:
          queueNamespace: "<namespace-name>"
          queueName: "<queue-name>"
          credentials:
            accessTokenFile:
              path: "/etc/secrets/access-token/service-bus-token"

.. important::

  Make sure that access tokens are renewed no later than 60 seconds before the expiration specified by their ``expires_on`` property.


.. _Azure Service Bus Data Receiver: https://learn.microsoft.com/en-us/azure/role-based-access-control/built-in-roles/integration#azure-service-bus-data-receiver
.. _Azure Workload Identity: https://azure.github.io/azure-workload-identity/docs/
.. _Storage Blob Data Contributor: https://learn.microsoft.com/en-us/azure/role-based-access-control/built-in-roles/storage#storage-blob-data-contributor


Configuring metadata backends
-----------------------------

The Storage Service uses metadata backends to implement the Metadata facet of the Omniverse Storage API. The Storage Service exposes the Metadata API when at least one storage endpoint is bound to a metadata backend.

The Storage Service can use zero or more metadata backends. Declare each backend under the ``/config/metadata`` object with a unique name:

.. code-block:: yaml

  config:
    metadata:
      metadata-backend-id-1:
        postgres: # ...
      metadata-2:
        postgres: # ...

.. tip::

  Metadata backends use their IDs to identify themselves in log messages, metrics, and traces.

PostgreSQL
^^^^^^^^^^

A PostgreSQL metadata backend allows you to use a PostgreSQL-compatible database to persist metadata for resources in one or more buckets or storage accounts:

.. code-block:: yaml

  config:
    metadata:
      # You can use any name for the metadata backend ID. The backend will use
      # it to identify itself in logs, metrics, and traces.
      metadata-x:
        postgres:
          host: "myhost.mydomain"
          dbname: "replace-with-database-name"
          credentials:
            password:
              user: "my-user"
              # Will interpolate the value of the STORAGE_SERVICE_POSTGRES_PASSWORD
              # environment variable.
              password: "${STORAGE_SERVICE_POSTGRES_PASSWORD}"
  extraEnv:
    # Inject the storage-service-secrets:postgres-password secret value in the
    # STORAGE_SERVICE_POSTGRES_PASSWORD environment variable.
    - name: "STORAGE_SERVICE_POSTGRES_PASSWORD"
      valueFrom:
        secretKeyRef:
          name: "storage-service-secrets"
          key: "postgres-password"

Access token authentication
"""""""""""""""""""""""""""

If your database passwords are short-lived or must be issued by a separate authority, such as when using Microsoft Entra ID authentication, you can inject credentials through access token files. Each file must contain JSON in the following format:

.. code-block:: json

  {
    "expires_on": "Token expiration timestamp in RFC 3339 format",
    "access_token": "eyJhbG.SflKxwRJSMeKK.a43@EXAMPLE"
  }

To enable the access token authentication, modify the metadata backend declaration as follows:

.. code-block:: yaml

  config:
    metadata:
      # Replace with your metadata backend ID.
      metadata-x:
        postgres:
          credentials:
            accessTokenFile:
              user: "my-user"
              # The access token will be used as the password.
              path: "/etc/secrets/metadata/metadata-x-token"

  # Declare a shared volume called "metadata-secrets".
  extraVolumes:
    - name: "metadata-secrets"
      emptyDir:
        medium: "Memory"
        sizeLimit: "4Mi"

  # Declare a volume mount for the Storage Service container.
  extraVolumeMounts:
    - name: "metadata-secrets"
      mountPath: "/etc/secrets/metadata"

  # For PostgreSQL metadata backends, declare the native sidecar as an init container.
  extraInitContainers:
    - name: "inject-metadata-secrets"
      image: "your-image-name"
      # Mount the secrets volume in the sidecar.
      volumeMounts:
        - name: "metadata-secrets"
          mountPath: "/etc/secrets/metadata"
      # Set the restart policy to Always if your sidecar should keep running
      # in the background, alongside the rest of the containers.
      restartPolicy: Always
      # Provide other settings for the sidecar container. They will be replaced
      # verbatim, for example:
      env: # ...
      envFrom: # ...
      command: # ...

.. important::

  Make sure that access tokens are renewed no later than 60 seconds before the expiration specified by their ``expires_on`` property.

If you use an :ref:`access-token-file service identity <service-identity>` whose provider can also inject access tokens for the metadata backend, the configuration can be simplified further:

.. code-block:: yaml

  serviceIdentity:
    accessTokenFile:
      enabled: true
      # ...

  config:
    metadata:
      # Replace with your metadata backend ID.
      metadata-x:
        postgres:
          credentials:
            accessTokenFile:
              user: "my-user"
              path: "/etc/secrets/access-token/metadata-x-token"

Metadata backend binding
^^^^^^^^^^^^^^^^^^^^^^^^

Once declared, a metadata backend can be attached to one or more buckets or storage accounts so that metadata reads and writes against URIs from that bucket or storage account are performed through that backend.

Specify the name of a metadata backend under the ``metadataBackend`` property to attach it to a bucket or storage account:

.. code-block:: yaml

  config:
    storage:
      azureBlob:
        endpoints:
          "https://mystorageacc.blob.core.windows.net":
            # Use /config/metadata/metadata-backend-a for the metadata backend.
            metadataBackend: "metadata-backend-a"
          "https://otherstorageacc.blob.core.windows.net":
            # Use /config/metadata/metadata-backend-b for the metadata backend.
            metadataBackend: "metadata-backend-b"
      s3:
        endpoints:
          "https://mybucket.s3.us-west-1.amazonaws.com":
            # Use /config/metadata/metadata-backend-a for the metadata backend.
            metadataBackend: "metadata-backend-a"
    metadata:
      metadata-backend-a:
        postgres: # ...
      metadata-backend-b:
        postgres: # ...
      # The metadata-backend-c is never referenced and is never initialized.
      metadata-backend-c:
        postgres: # ...

.. warning::

  Unreferenced metadata backends are never initialized. Referencing an undeclared metadata backend causes the Storage Service application to terminate immediately after launch.


.. _permissions:

Configuring request authorization
---------------------------------

The Storage Service does not authorize requests on its own. It either disables authorization entirely or delegates authorization to the Omniverse Permission API.

To enable access validation, enable the permission client in the values file:

.. code-block:: yaml

  config:
    permissions:
      enabled: true
      client:
        grpc:
          # Use the HTTP scheme to establish insecure connections to the Permission API service,
          # or HTTPS for TLS connections.
          endpoint: "http://permission-service:3010"

Alternatively, if the Permission API backend does not support the gRPC protocol or it is not forwarded, the REST protocol can be used:

.. code-block:: yaml

  config:
    permissions:
      enabled: true
      client:
        rest:
          endpoint: "https://permission-service.myhost.mydomain"

When configured, the Storage Service authorizes each call through the Permission API backend using the caller's credentials from the ``authorization`` header.

Exempting a backend from authorization
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

By default, requests to every declared backend undergo authorization checks. To exempt an individual bucket or storage account, set ``permissionsEnabled`` to ``false`` in its declaration. Requests to that backend will then bypass authorization checks:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "https://mybucket.s3.us-west-1.amazonaws.com":
            permissionsEnabled: false
      azureBlob:
        endpoints:
          "https://mystorageacc.blob.core.windows.net":
            permissionsEnabled: false

Access evaluation rules
^^^^^^^^^^^^^^^^^^^^^^^

By default, non-mutating requests against an object or folder, such as reading an object or listing a folder, resolve to the ``read`` action for that entity. Mutating requests, such as writing an object or creating a folder, resolve to the ``write`` action.

Folders and objects correspond to the ``folder`` and ``object`` resource types, respectively.

Access to an entity is evaluated hierarchically, with the deepest rule taking precedence. For example, a read request for ``https://myhost.mydomain/path/to/object`` evaluates the following rules in order:

.. list-table::
  :header-rows: 1

  * - Order
    - Resource Type
    - Resource
  * - 1
    - ``object``
    - ``https://myhost.mydomain/path/to/object``
  * - 2
    - ``folder``
    - ``https://myhost.mydomain/path/to/``
  * - 3
    - ``folder``
    - ``https://myhost.mydomain/path/``
  * - 4
    - ``folder``
    - ``https://myhost.mydomain/``

Therefore, if the caller is allowed to read ``object`` ``https://myhost.mydomain/path/to/object`` but is forbidden to read ``folder`` ``https://myhost.mydomain/path/to/``, the read request is allowed.


Configuring URL remapping
-------------------------

The URL remapper exposes stable, client-facing addresses while storing data at different backend addresses. For each request, it replaces the matching client-facing prefix with the rule's target prefix before forwarding the request to the storage backend. It rewrites addresses in responses back to the client-facing prefix.

Configure remapping rules under ``config.urlRemapper.rewriteRules``. Both prefixes must end with ``/``.

.. code-block:: yaml

  config:
    urlRemapper:
      enabled: true
      rewriteRules:
        "omniverse://content.example.com/":
          target: "https://content.blob.core.windows.net/assets/"
        "omniverse://models.example.com/":
          target: "https://models.s3.us-west-1.amazonaws.com/"

Hiding storage backends from the top-level address listing
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

By default, every configured S3 and Azure Blob endpoint is advertised by ``ListTopLevelAddresses``. Set ``listInTopLevelAddresses`` to ``false`` on an endpoint to omit its address from the response. The endpoint remains configured and requests to its address continue to work.

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "https://models.s3.us-west-1.amazonaws.com":
            listInTopLevelAddresses: false
      azureBlob:
        endpoints:
          "https://content.blob.core.windows.net":
            listInTopLevelAddresses: false

Hiding remapped routes from the top-level address listing
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

By default, the client-facing prefix of every rewrite rule is advertised by ``ListTopLevelAddresses``. Set ``listInTopLevelAddresses`` to ``false`` on a rule to omit that prefix from the response. The rule remains active and the prefix remains routable.

.. code-block:: yaml

  config:
    urlRemapper:
      enabled: true
      rewriteRules:
        "omniverse://public.example.com/":
          target: "https://content.blob.core.windows.net/public/"
        "omniverse://internal.example.com/":
          target: "https://content.blob.core.windows.net/internal/"
          listInTopLevelAddresses: false

Permission evaluation rules for remapped addresses
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Permissions are evaluated after the client-facing address is remapped. The Permission API therefore receives the target backend address, not the address submitted by the client. For example, if ``omniverse://content.example.com/image.png`` is remapped to ``https://content.blob.core.windows.net/assets/image.png``, permission rules are evaluated for ``https://content.blob.core.windows.net/assets/image.png``.

For operations with source and destination addresses, each address is remapped and evaluated separately. The ``permissionsEnabled`` setting of the target S3 or Azure Blob endpoint determines whether authorization is required, as described in :ref:`request authorization <permissions>`.


Configuring notifications
-------------------------

When the Storage Service subscribes to events from at least one bucket or storage account, it can transform those events into notifications understood by other applications in the Omniverse stack. The notifications can then be published through the Omniverse Notification API.

To enable event propagation, enable the notification client in the values file:

.. code-block:: yaml

  config:
    notificationClient:
      enabled: true
      # The Storage Service will use the gRPC protocol. Use the HTTP scheme to
      # establish unencrypted gRPC connections and HTTPS for encrypted gRPC
      # connections.
      endpointUrl: "http://event-aggregation-service:50051"

Once enabled, the client uses the :ref:`Service Identity <service-identity>` credentials to authenticate requests through the ``authorization`` header. If the service identity is not configured, the client omits the ``authorization`` header.

Permissions for publishing
^^^^^^^^^^^^^^^^^^^^^^^^^^

When using the Event Aggregation Service as the Omniverse Notification API provider, the Storage Service will need the following permissions for publishing storage events:

.. list-table::
  :header-rows: 1

  * - Resource
    - Service
    - Action
  * - ``omni.storage.created``
    - ``event-aggregation-service``
    - ``publish-event``
  * - ``omni.storage.deleted``
    - ``event-aggregation-service``
    - ``publish-event``
  * - ``omni.storage.dir_created``
    - ``event-aggregation-service``
    - ``publish-event``
  * - ``omni.storage.dir_deleted``
    - ``event-aggregation-service``
    - ``publish-event``

The above permissions could be expressed using the following Cedar rules:

.. code-block:: rust

  permit(
    principal == Principal::"<principal-id>",
    action == Action::"event-aggregation-service:publish-event",
    resource == EventType::"omni.storage.created"
  );
  permit(
    principal == Principal::"<principal-id>",
    action == Action::"event-aggregation-service:publish-event",
    resource == EventType::"omni.storage.deleted"
  );
  permit(
    principal == Principal::"<principal-id>",
    action == Action::"event-aggregation-service:publish-event",
    resource == EventType::"omni.storage.dir_deleted"
  );
  permit(
    principal == Principal::"<principal-id>",
    action == Action::"event-aggregation-service:publish-event",
    resource == EventType::"omni.storage.dir_created"
  );


Configuring caching
-------------------

Stat cache
^^^^^^^^^^

The stat cache caches responses to Stat requests. As a rule of thumb, enable it when using versioned storage backends or when the list cache is disabled. Otherwise, it has little or no effect on runtime performance.

To enable the cache, update the values file as follows:

.. code-block:: yaml

  config:
    statCache:
      enabled: true
      # It's recommended that you enable this setting regardless of whether
      # the Redis client is configured.
      l2Enabled: true

Small object cache
^^^^^^^^^^^^^^^^^^

The small object cache caches objects downloaded directly from the Storage Service through FileObject API calls when their size does not exceed the ``smallObjectSize`` property.


To enable the cache, update the values file as follows:

.. code-block:: yaml

  config:
    smallObjectCache:
      enabled: true
      # It's recommended that you enable this setting regardless of whether
      # the Redis client is configured.
      l2Enabled: true
      # Override the property if you want to cache larger responses.
      smallObjectSize: "256KiB"

List cache
^^^^^^^^^^

The list cache caches responses to List and ListStat requests. It also caches and prefetches Stat responses. When the Storage Service receives a Stat request for a resource address, the cache performs a ListStat request for the parent folder, caches the result, and serves the Stat response from the cache.

.. warning::

  The list cache is sensitive to the layout of the underlying storage. Folders with many direct children may significantly affect the Storage Service's performance. In addition, write operations against files in a folder invalidate cached content for every file in that folder. Repeated read-write cycles may increase request latency.

To enable the cache, update the values file as follows:

.. code-block:: yaml

  config:
    listCache:
      enabled: true
      # It's recommended that you enable this setting regardless of whether
      # the Redis client is configured.
      l2Enabled: true

Metadata cache
^^^^^^^^^^^^^^

The metadata cache holds individual Metadata API key values and query results in memory for a short time.

To enable the cache, update the values file as follows:

.. code-block:: yaml

  config:
    metadataCache:
      enabled: true
      # It's recommended that you enable this setting regardless of whether
      # the Redis client is configured.
      l2Enabled: true

Distributed caching/invalidation with Redis/Valkey
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

In deployments with multiple replicas, each instance uses its own in-memory cache. Because these caches are not synchronized with one another, the Storage Service relies on Redis or Valkey for centralized invalidation. The caches also use Redis or Valkey as a shared tier-2 cache.

.. attention::

  The Storage Service requires Redis 8.4 or later, or Valkey 8.1 or later.

To enable the Redis client, update the service values as follows:

.. code-block:: yaml

  config:
    redis:
      enabled: true
      # This will tell the client to establish an unencrypted TCP connection to
      # a server running on myhost.mydomain:6379. Alternatively, you can use:
      # - redis://myhost.mydomain:1234 - to establish an unencrypted TCP
      #   connection to a server running on myhost.mydomain:1234
      # - redis://myhost.mydomain:1234/1 - to establish an unencrypted TCP
      #   connection to database 1 on a server running on myhost.mydomain:1234
      # - rediss://somehost:1234 - to establish an encrypted TLS connection to
      #   a server running on somehost:1234
      address: "redis://myhost.mydomain"
      # When sharing the same Redis/Valkey instance across multiple
      # deployments, make sure to provide each a unique namespace prefix to
      # avoid keyspace collisions.
      namespace: "storage-service-a"


If not already enabled, configure the caches to use the tier-2 cache:

.. code-block:: yaml

  config:
    statCache:
      l2Enabled: true
    listCache:
      l2Enabled: true
    smallObjectCache:
      l2Enabled: true
    metadataCache:
      l2Enabled: true


Configuring telemetry
---------------------

The Storage Service uses OpenTelemetry to emit metrics and traces. Use the following settings to configure the exporter endpoints:

.. code-block:: yaml

  observability:
    metrics:
      enabled: true
      exportIntervalMs: 15000
      endpoint:
        grpc: "http://exporter.observability.svc.cluster.local:4317"
    tracing:
      enabled: true
      endpoint:
        grpc: "http://exporter.observability.svc.cluster.local:4317"

Structured logging
^^^^^^^^^^^^^^^^^^

To enable structured logging (emitting logs as JSON), set the ``/observability/logging/structured/enabled`` values property to ``true``:

.. code-block:: yaml

  observability:
    logging:
      structured:
        enabled: true
