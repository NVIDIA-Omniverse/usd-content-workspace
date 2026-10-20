Environment variable interpolation
==================================

Only configuration properties under the ``config`` section that accept string values can be populated from environment variables at startup. This allows deployment-specific configuration, including credentials from Kubernetes Secrets, to be supplied without being hardcoded in the Helm chart.

An environment variable reference is declared as ``${NAME}``. The reference must occupy the complete property value; partial variable substitution is not supported. For example, ``bucket-${STORAGE_SERVICE_IDX}-id`` is not interpolated.

For example, the following configuration declares a Redis client, imports its address from a Kubernetes Secret, and supplies the address through an environment variable reference:

.. code-block:: yaml

  extraEnvs:
    - name: STORAGE_SERVICE_REDIS_ADDRESS
      valueFrom:
        secretKeyRef:
          name: storage-service-redis
          key: address
  config:
    redis:
      enabled: true
      address: "${STORAGE_SERVICE_REDIS_ADDRESS}"

For example, the following configuration is invalid:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "${STORAGE_SERVICE_S3_BUCKET}": # See the explanation below.
            anonymous: true
    statCache:
      enabled: "${STORAGE_SERVICE_STAT_CACHE_ENABLED}" # Only string configuration properties support interpolation.
    listCache:
      capacity: "${STORAGE_SERVICE_LIST_CACHE_CAPACITY}MiB" # Partial variable substitution is not supported.

Known issues
------------

S3 bucket endpoints and Azure Blob storage account endpoints cannot be replaced with environment variable placeholders. Their names must be specified verbatim.
