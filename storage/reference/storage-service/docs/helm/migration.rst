Migration guide
===============

This guide describes the configuration changes required when upgrading between chart versions.

1.2.1+
------

The ``serviceIdentity.clientCredentials`` configuration has been deprecated. To inject service identity access tokens into the pod, configure the :doc:`access token file credentials <storage-credentials>` provisioner sidecar instead.

The ``enablePermissions`` endpoint setting has been renamed to ``permissionsEnabled``. Update the parameter name for both S3 and Azure Blob storage backends:

``config.storage.s3.endpoints.<name>.enablePermissions``
  :Replacement: ``config.storage.s3.endpoints.<name>.permissionsEnabled``

``config.storage.azureBlob.endpoints.<name>.enablePermissions``
  :Replacement: ``config.storage.azureBlob.endpoints.<name>.permissionsEnabled``

1.2.0+
------

The legacy flat AWS credential parameters are no longer supported. For S3 storage endpoints, use the following replacements in the :doc:`credentials <storage-backend>` object:

``config.storage.s3.endpoints.<name>.accessKeyId``
  :Replacement: ``config.storage.s3.endpoints.<name>.credentials.accessKey.accessKeyId``

``config.storage.s3.endpoints.<name>.secretAccessKey``
  :Replacement: ``config.storage.s3.endpoints.<name>.credentials.accessKey.secretAccessKey``

``config.storage.s3.endpoints.<name>.sessionToken``
  :Replacement: ``config.storage.s3.endpoints.<name>.credentials.accessKey.sessionToken``

``config.storage.s3.endpoints.<name>.assumedRoleArn``
  :Replacement: ``config.storage.s3.endpoints.<name>.credentials.assumeRole.roleArn``

``config.storage.s3.endpoints.<name>.assumedRoleSessionName``
  :Replacement: ``config.storage.s3.endpoints.<name>.credentials.assumeRole.sessionName``

``config.storage.s3.endpoints.<name>.assumedRoleSessionLength``
  :Replacement: ``config.storage.s3.endpoints.<name>.credentials.assumeRole.sessionLength``

``config.storage.s3.endpoints.<name>.externalId``
  :Replacement: ``config.storage.s3.endpoints.<name>.credentials.assumeRole.externalId``
