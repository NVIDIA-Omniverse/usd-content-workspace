Storage backend event subscriptions
===================================

When the storage service has one or more storage event subscriptions, it will use them to invalidate caches and to publish events in the Event Aggregation Service about the modifications.

Event subscriber settings are located under the ``config.storageEvents`` object. Object keys serve as the application-wide subscriber identifiers that are used for diagnostics and metrics. For example, the configuration below declares three event subscribers: two SQS subscribers (``event-subscriber-a`` and ``event-subscriber-b``) and one Azure Service Bus subscriber (``event-subscriber-c``). The notification client is configured to gather messages from them and publish to the Event Aggregation Service:

.. code-block:: yaml

  config:
    storageEvents:
      event-subscriber-a:
        sqs: ...
      event-subscriber-b:
        sqs: ...
      event-subscriber-c:
        azureServiceBus: ...
    notificationClient:
      enabled: true
      ...

Note that an event subscriber requires either the caches or the notification client to be enabled to operate. Otherwise, storage events will not be consumed.

Azure Blob events
-----------------

The Azure Blob event subscriber allows you to consume ``Microsoft.Storage.BlobCreated`` and ``Microsoft.Storage.BlobDeleted`` events produced by one or more storage accounts, enveloped in ``CloudEvents v1.0`` messages. The subscriber configuration is located under the ``config.storageEvents.<id>.azureServiceBus`` object and has the following parameters:

``config.storageEvents.<id>.azureServiceBus.queueNamespace``
  :Required: yes
  :Type: string

  Azure Service Bus namespace name.

``config.storageEvents.<id>.azureServiceBus.queueName``
  :Required: yes
  :Type: string

  Azure Service Bus queue name.

``config.storageEvents.<id>.azureServiceBus.credentials``
  :Required: yes
  :Type: object

  Authentication credentials for the subscriber.

``config.storageEvents.<id>.azureServiceBus.receiveBatchSize``
  :Required: no
  :Type: integer
  :Default: ``20``

  Number of messages to receive per batch.

``config.storageEvents.<id>.azureServiceBus.receiveMaxWaitTime``
  :Required: no
  :Type: duration
  :Default: ``100ms``

  Maximum duration of a receive call.

``config.storageEvents.<id>.azureServiceBus.workers``
  :Required: no
  :Type: integer
  :Default: ``10``

  Number of concurrent workers.

Authentication
^^^^^^^^^^^^^^

To consume messages from a queue, configure credentials through the ``config.storageEvents.<id>.azureServiceBus.credentials`` object.

SAS policy credentials
""""""""""""""""""""""

The following settings allow you to use a namespace- or queue-wide shared access policy to access the queue:

``config.storageEvents.<id>.azureServiceBus.credentials.sharedAccessPolicyKey.policyName``
  :Required: yes
  :Type: string

  SAS policy name.

``config.storageEvents.<id>.azureServiceBus.credentials.sharedAccessPolicyKey.policyKey``
  :Required: yes
  :Type: string

  SAS policy key.

A configuration using a SAS policy could look as follows:

.. code-block:: yaml

  config:
    storageEvents:
      event-subscriber-x:
        azureServiceBus:
          queueNamespace: "myservicebusqueuens"
          queueName: "myservicebusqueue"
          # Interpolate the policy key from the environment variable
          # called STORAGE_SERVICE_AZURE_SERVICE_BUS_POLICY_KEY.
          credentials:
            sharedAccessPolicyKey:
              policyName: "MySASPolicyName"
              policyKey: "${STORAGE_SERVICE_AZURE_SERVICE_BUS_POLICY_KEY}"

Service principal credentials
"""""""""""""""""""""""""""""

As a more secure alternative, you can use client credentials of a service principal in Azure Entra ID.

``config.storageEvents.<id>.azureServiceBus.credentials.clientSecret.tenantId``
  :Required: yes
  :Type: string

  Azure Entra ID Tenant ID for service principal authentication.

``config.storageEvents.<id>.azureServiceBus.credentials.clientSecret.clientId``
  :Required: yes
  :Type: string

  Azure Entra ID Client ID for service principal authentication.

``config.storageEvents.<id>.azureServiceBus.credentials.clientSecret.clientSecret``
  :Required: yes
  :Type: string

  Azure Entra ID client secret for service principal authentication.

``config.storageEvents.<id>.azureServiceBus.credentials.clientSecret.authorityHost``
  :Required: no
  :Type: string
  :Default: ``https://login.microsoftonline.com``

  Override the Azure Entra ID authority host.

A configuration that uses service principal credentials could look as follows:

.. code-block:: yaml

  config:
    storageEvents:
      event-subscriber-x:
        azureServiceBus:
          queueNamespace: "myservicebusqueuens"
          queueName: "myservicebusqueue"
          # Interpolate the client secret from the environment variable
          # called STORAGE_SERVICE_AZURE_ENTRA_ID_CLIENT_SECRET.
          credentials:
            clientSecret:
              tenantId: "94e63603-d604-480a-a221-557a203d1d1e"
              clientId: "b866fb03-aca5-4278-bd84-b0fd4af748cd"
              clientSecret: "${STORAGE_SERVICE_AZURE_ENTRA_ID_CLIENT_SECRET}"

Federated Credentials (Workload Identity) authentication
""""""""""""""""""""""""""""""""""""""""""""""""""""""""

Federated Credentials authentication allows the application to exchange an access token issued by an external OIDC provider for an Azure Entra ID access token.

``config.storageEvents.<id>.azureServiceBus.credentials.federated.tenantId``
  :Required: yes
  :Type: string

  Azure Entra ID Tenant ID for federated authentication.

``config.storageEvents.<id>.azureServiceBus.credentials.federated.clientId``
  :Required: yes
  :Type: string

  Azure Entra ID Client ID for federated authentication.

``config.storageEvents.<id>.azureServiceBus.credentials.federated.federatedTokenFile``
  :Required: yes
  :Type: string

  Path to the file containing the federated identity token.

``config.storageEvents.<id>.azureServiceBus.credentials.federated.authorityHost``
  :Required: no
  :Type: string
  :Default: ``https://login.microsoftonline.com``

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
    storageEvents:
      event-subscriber-x:
        azureServiceBus:
          queueNamespace: "myservicebusqueuens"
          queueName: "myservicebusqueue"
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

The token must be a bearer token scoped to Azure Service Bus (scope ``https://servicebus.azure.net/.default``), refreshed no later than 60 seconds before expiry.

The token file provider supports the following configuration parameters:

``config.storageEvents.<id>.azureServiceBus.credentials.accessTokenFile.path``
  :Required: yes
  :Type: string

  Path to the file containing the access token.

A configuration for token file authentication could look as follows:

.. code-block:: yaml

  config:
    storageEvents:
      event-subscriber-x:
        azureServiceBus:
          queueNamespace: "myservicebusqueuens"
          queueName: "myservicebusqueue"
          credentials:
            credentials:
              accessTokenFile:
                path: "/etc/secrets/azure-access-token"

Permissions
^^^^^^^^^^^

When you authenticate the client with a service principal, the service principal needs the following permissions to receive messages:

.. code-block:: json

  {
    "properties": {
      "roleName": "Storage Service Queue Listener",
      "description": "Listen for messages in a Service Bus Queue",
      "assignableScopes": [
        "/subscriptions/<subscription>/resourceGroups/<resource-group>/providers/Microsoft.ServiceBus/namespaces/<namespace-name>/queues/<queue-name>"
      ],
      "permissions": [
        {
          "actions": [
            "Microsoft.ServiceBus/*/queues/read",
            "Microsoft.ServiceBus/*/topics/read",
            "Microsoft.ServiceBus/*/topics/subscriptions/read"
          ],
          "notActions": [],
          "dataActions": [
            "Microsoft.ServiceBus/*/receive/action"
          ],
          "notDataActions": []
        }
      ]
    }
  }

Alternatively, you can assign the service principal the built-in ``Azure Service Bus Data Receiver`` role for the queue.

If you use SAS policy credentials, make sure the policy has the ``Listen`` right attached.

AWS S3 bucket events
--------------------

The AWS S3 event subscriber allows you to consume ``s3:ObjectCreated:*`` and ``s3:ObjectRemoved:*`` events produced by one or more AWS S3 buckets. The subscriber configuration is located under the ``config.storageEvents.<id>.sqs`` object and has the following parameters:

``config.storageEvents.<id>.sqs.queueUrl``
  :Required: yes
  :Type: string

  SQS queue URL.

``config.storageEvents.<id>.sqs.region``
  :Required: yes
  :Type: string

  AWS region name for the queue.

``config.storageEvents.<id>.sqs.endpointUrl``
  :Required: no
  :Type: string

  Custom SQS service endpoint URL.

``config.storageEvents.<id>.sqs.maxNumberOfMessages``
  :Required: no
  :Type: integer
  :Default: ``10``

  Maximum number of messages to receive per poll.

``config.storageEvents.<id>.sqs.visibilityTimeout``
  :Required: no
  :Type: duration
  :Default: ``30s``

  Message visibility timeout.

``config.storageEvents.<id>.sqs.waitTime``
  :Required: no
  :Type: duration
  :Default: ``20s``

  Message polling wait time.

``config.storageEvents.<id>.sqs.workers``
  :Required: no
  :Type: integer
  :Default: ``10``

  Number of concurrent consumers.

``config.storageEvents.<id>.sqs.credentials``
  :Required: yes
  :Type: object

  Custom credentials for the client.

A minimal configuration could look as follows:

.. code-block:: yaml

  config:
    storageEvents:
      event-subscriber-y:
        sqs:
          queueUrl: "https://sqs.us-east-1.amazonaws.com/177715257436/MyQueue"
          region: "us-east-1"

Authentication
^^^^^^^^^^^^^^

By default, the client will look for AWS IRSA credentials, identified by the presense of the ``AWS_ROLE_ARN`` and ``AWS_WEB_IDENTITY_TOKEN_FILE`` environment variables. If these credentials are missing or your queue requires a different set of credentials, they must be provided under the ``config.storageEvents.<id>.sqs.credentials`` object.

Access Key credentials
""""""""""""""""""""""

The following settings allow you to inject custom Access Key credentials that authenticate your application.

``config.storageEvents.<id>.sqs.credentials.accessKey.accessKeyId``
  :Required: yes
  :Type: string

  AWS access key id.

``config.storageEvents.<id>.sqs.credentials.accessKey.secretAccessKey``
  :Required: yes
  :Type: string

  AWS secret access key.

``config.storageEvents.<id>.sqs.credentials.accessKey.sessionToken``
  :Required: no
  :Type: string

  AWS session token.

A configuration that uses access key credentials could look as follows:

.. code-block:: yaml

  config:
    storageEvents:
      event-subscriber-y:
        sqs:
          queueUrl: "https://sqs.us-east-1.amazonaws.com/177715257436/MyQueue"
          region: "us-east-1"
          credentials:
            accessKey:
              accessKeyId: "AKIAIOSFODNN7EXAMPLE"
              secretAccessKey: "${STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY}"

Assume Role credentials
"""""""""""""""""""""""

If your queue is located in a different AWS account, you will need to assume a role in the target account. The following settings allow you to achieve this regardless of the service credentials acquisition method used (e.g. AWS IRSA or Access Key):

``config.storageEvents.<id>.sqs.credentials.assumeRole.roleArn``
  :Required: yes
  :Type: string

  AWS Role ARN.

``config.storageEvents.<id>.sqs.credentials.assumeRole.externalId``
  :Required: no
  :Type: string

  External ID for the AssumeRole credentials.

``config.storageEvents.<id>.sqs.credentials.assumeRole.sessionName``
  :Required: no
  :Type: string

  Session name for the AssumeRole credentials.

``config.storageEvents.<id>.sqs.credentials.assumeRole.sessionLength``
  :Required: no
  :Type: duration

  Session length for the AssumeRole credentials.

A configuration with Assume Role credentials could look as follows:

.. code-block:: yaml

  config:
    storageEvents:
      event-subscriber-y:
        sqs:
          queueUrl: "https://sqs.us-east-1.amazonaws.com/177715257436/MyQueue"
          region: "us-east-1"
          # Obtain AWS IRSA credentials first, then assume the "mirror" role in the
          # target account.
          credentials:
            assumeRole:
              roleArn: "arn:aws:iam::111122223333:role/myrole"

Web Identity Token (AWS IRSA) authentication
""""""""""""""""""""""""""""""""""""""""""""

The Web Identity Token flow allows the application to exchange an access token issued by an external OIDC provider for an AWS access key.

``config.storageEvents.<id>.sqs.credentials.webIdentityToken.webIdentityTokenFile``
  :Required: yes
  :Type: string

  Access token file location.

``config.storageEvents.<id>.sqs.credentials.webIdentityToken.roleArn``
  :Required: yes
  :Type: string

  AWS IAM role ARN for the application.

``config.storageEvents.<id>.sqs.credentials.webIdentityToken.roleSessionName``
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
    storageEvents:
      event-subscriber-y:
        sqs:
          queueUrl: "https://sqs.us-east-1.amazonaws.com/177715257436/MyQueue"
          region: "us-east-1"
          credentials:
            webIdentityToken:
              # Don't inject any of the environment variables below - they
              # are injected by the IRSA addon automatically.
              webIdentityTokenFile: "${AWS_WEB_IDENTITY_TOKEN_FILE}"
              roleArn: "${AWS_ROLE_ARN}"

Permissions
^^^^^^^^^^^

You will need to configure the following policy for the service role to consume messages:

.. code-block:: json

  {
    "Version": "2012-10-17",
    "Statement": [
      {
        "Sid": "<sid>",
        "Effect": "Allow",
        "Action": [
          "sqs:ReceiveMessage",
          "sqs:DeleteMessage",
          "sqs:GetQueueAttributes",
          "sqs:GetQueueUrl"
        ],
        "Resource": "arn:aws:sqs:${Region}:${Account}:${QueueName}"
      }
    ]
  }

Notification publishing
-----------------------

When an event subscription is configured, for example, `Azure Blob events <#azure-blob-events>`_ or `S3 Bucket events <#aws-s3-bucket-events>`_, it becomes possible to propagate them to the Event Aggregation Service so that clients that use the Storage Service can receive storage notifications. The notification client, when configured and enabled, will then gather events from all configured subscribers and publish them.

The client supports the following configuration parameters:

``config.notificationClient.enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Whether to enable event publishing.

``config.notificationClient.endpointUrl``
  :Required: no
  :Type: string
  :Default: ``http://event-aggregation-service:50051``

  Event Aggregation Service endpoint URL.

``config.notificationClient.batchSize``
  :Required: no
  :Type: integer
  :Default: ``100``

  Maximum notifications per ``BatchPublishEvents`` request.

``config.notificationClient.eventTypes.objectCreated``
  :Required: no
  :Type: string
  :Default: ``omni.storage.created``

  Event type for object creation.

``config.notificationClient.eventTypes.objectDeleted``
  :Required: no
  :Type: string
  :Default: ``omni.storage.deleted``

  Event type for object deletion.

``config.notificationClient.eventTypes.dirCreated``
  :Required: no
  :Type: string
  :Default: ``omni.storage.dir_created``

  Event type for synthetic directory creation (see `Synthetic directory events <#synthetic-directory-events>`_).

``config.notificationClient.eventTypes.dirDeleted``
  :Required: no
  :Type: string
  :Default: ``omni.storage.dir_deleted``

  Event type for synthetic directory deletion (see `Synthetic directory events <#synthetic-directory-events>`_).

``config.notificationClient.timeouts.request``
  :Required: no
  :Type: duration
  :Default: ``60s``

  Request timeout.

``config.notificationClient.timeouts.connect``
  :Required: no
  :Type: duration
  :Default: ``60s``

  Connect timeout.

``config.notificationClient.credentials``
  :Required: no
  :Type: object

  Client credentials.

``config.notificationClient.connectionsPerHost``
  :Required: no
  :Type: integer
  :Default: ``4``

  Number of TCP connections opened to each resolved backend instance.

``config.notificationClient.dnsRefreshInterval``
  :Required: no
  :Type: duration
  :Default: ``5s``

  How often to re-resolve the endpoint DNS to discover new pods.

``config.notificationClient.tcpKeepalive``
  :Required: no
  :Type: duration
  :Default: ``30s``

  TCP keep-alive interval on each per-pod subchannel.

``config.notificationClient.http2KeepAliveInterval``
  :Required: no
  :Type: duration
  :Default: ``30s``

  HTTP/2 keep-alive ping interval on each per-pod subchannel.

A minimal client configuration could look as follows:

.. code-block:: yaml

  config:
    notificationClient:
      enabled: true
      endpointUrl: "http://event-aggregation-service.storage.svc.cluster.local:1234"

Authentication
^^^^^^^^^^^^^^

When client credentials configuration is omitted, the notification client will use the `Service Identity credentials <./storage-credentials.rst>`_ from the ``/serviceIdentity`` configuration object to authenticate calls against the Event Aggregation Service.

Access token file credentials
"""""""""""""""""""""""""""""

You can delegate token acquisition to a separate process - for example, one running in a sidecar container. That process writes the credentials to a file with the following structure:

.. code-block:: json

  {
    "expires_on": "Token expiration timestamp in RFC 3339 format",
    "access_token": "eyJhbG.SflKxwRJSMeKK.dsF354gwwweW"
  }

The token must be a properly-scoped bearer token, refreshed no later than 60 seconds before expiry.

The token file provider supports the following configuration parameters:

``config.notificationClient.credentials.accessTokenFile.path``
  :Required: yes
  :Type: string

  Path to the file containing the access token.

A configuration for token file authentication could look as follows:

.. code-block:: yaml

  config:
    notificationClient:
      enabled: true
      endpointUrl: "http://event-aggregation-service.storage.svc.cluster.local:1234"
      credentials:
        accessTokenFile:
          path: "/etc/secrets/notification-client-token"

Permissions
^^^^^^^^^^^

You will need to grant the service principal access to the following actions:

.. list-table::
  :header-rows: 1

  * - Service
    - Action
    - Resource
  * - ``event-aggregation-service``
    - ``publish-event``
    - ``omni.storage.created``
  * - ``event-aggregation-service``
    - ``publish-event``
    - ``omni.storage.deleted``
  * - ``event-aggregation-service``
    - ``publish-event``
    - ``omni.storage.dir_created``
  * - ``event-aggregation-service``
    - ``publish-event``
    - ``omni.storage.dir_deleted``

If you have specified different event type values under the ``config.notificationClient.eventTypes`` object, make sure to use them as the resource type.

Synthetic directory events
--------------------------

The Storage Service can derive synthetic directory events from the file-level create/delete events it sees on a configured subscriber. For every ``omni.storage.created`` event the service emits an ``omni.storage.dir_created`` event for each ancestor directory of the file's URL; deletes mirror the same for ``omni.storage.dir_deleted``. Events are deduplicated within a configurable flush interval and published to the Notification Service.

This is a best-effort signal: per-event publish failures are logged and dropped. Subscribers that need exact directory mutation semantics should not rely on these events.

The feature is configured under ``config.syntheticDirEvents`` and supports the following parameters:

``config.syntheticDirEvents.enabled``
  :Required: no
  :Type: boolean
  :Default: ``true``

  Whether to derive and publish synthetic directory events.

``config.syntheticDirEvents.flushInterval``
  :Required: no
  :Type: duration
  :Default: ``6s``

  How often the accumulator drains to the notification service. Must be non-zero.

A minimal configuration could look as follows:

.. code-block:: yaml

  config:
    syntheticDirEvents:
      enabled: true
      flushInterval: "6s"

The event type strings emitted on the wire are controlled by ``config.notificationClient.eventTypes.dirCreated`` and ``config.notificationClient.eventTypes.dirDeleted`` - see the `Notification publishing <#notification-publishing>`_ section above.
