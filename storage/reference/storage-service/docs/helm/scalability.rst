Scalability
===========

Resource requests and limits
----------------------------

Use the ``resources`` object to set Kubernetes resource requests and limits for the Storage Service container. The chart applies the same resources to the bootstrap container when that container is enabled. Custom init containers and sidecars do not inherit these settings. They must declare their resource requirements explicitly in each ``extraInitContainers`` or ``extraContainers`` entry.

``resources.requests.cpu``
  :Required: no
  :Type: Kubernetes CPU quantity
  :Default: ``8``

  CPU requested for scheduling.

``resources.requests.memory``
  :Required: no
  :Type: Kubernetes memory quantity
  :Default: ``2Gi``

  Memory requested for scheduling.

``resources.requests.ephemeral-storage``
  :Required: no
  :Type: Kubernetes storage quantity
  :Default: ``256Mi``

  Temporary local storage requested for scheduling.

``resources.limits.cpu``
  :Required: no
  :Type: Kubernetes CPU quantity
  :Default: ``8``

  Maximum CPU available to the container.

``resources.limits.memory``
  :Required: no
  :Type: Kubernetes memory quantity
  :Default: ``4Gi``

  Maximum memory available to the container.

``resources.limits.ephemeral-storage``
  :Required: no
  :Type: Kubernetes storage quantity
  :Default: ``1Gi``

  Maximum temporary local storage available to the container.

For example, the following configuration requests 4 CPU cores, 4 GiB of memory, and 512 MiB of temporary local storage. It allows the container to use up to 8 CPU cores, 8 GiB of memory, and 2 GiB of temporary local storage:

.. code-block:: yaml

  resources:
    requests:
      cpu: 4
      memory: 4Gi
      ephemeral-storage: 512Mi
    limits:
      cpu: 8
      memory: 8Gi
      ephemeral-storage: 2Gi

Memory limit calculation
------------------------

Set ``resources.limits.memory`` to at least the sum of the memory limits for all enabled caches plus 1 GiB of headroom for request handling. Each cache defines its own limit with either its ``capacity`` or ``cacheCapacity`` property:

* ``config.smallObjectCache.capacity``
* ``config.statCache.capacity``
* ``config.listCache.capacity``
* ``config.metadataCache.cacheCapacity``
* ``config.manifest.cacheCapacity``

For example, when the enabled small object, stat, and list caches each have a capacity of 1 GiB, set the memory limit to at least 4 GiB: 3 GiB for the caches and 1 GiB for request handling. Increase the request-handling headroom when expected request concurrency or response sizes require it.

Multiple replica scaling
------------------------

When deploying multiple replicas with any :doc:`cache enabled <storage-backend-caching>`, we recommend configuring the shared :doc:`Redis client <redis-client>`. Enable ``config.redis.enabled`` and the ``l2Enabled`` setting for every cache configuration that is enabled. This gives replicas a shared L2 cache and allows them to exchange cache invalidation messages.

For example, the following configuration enables the Redis client and the L2 tier for all cache configurations. Configure the endpoint, credentials, namespace, and connection settings as described in the :doc:`Redis client configuration <redis-client>` documentation.

.. code-block:: yaml

  config:
    redis:
      enabled: true
      address: "redis://valkey.infra.svc.cluster.local:6379"
    metadataCache:
      l2Enabled: true
    smallObjectCache:
      l2Enabled: true
    statCache:
      l2Enabled: true
    listCache:
      l2Enabled: true
    manifest:
      l2Enabled: true

Event consumer considerations
-----------------------------

Each Storage Service replica starts an event consumer for every configured :doc:`storage event subscription <storage-backend-events>`. The ``workers`` value for an SQS or Azure Service Bus consumer applies independently in every replica. Consequently, a subscription configured with ``N`` workers and deployed to ``R`` replicas can run up to ``N * R`` concurrent consumers.

When scaling the service horizontally, reduce the configured worker count per consumer when necessary to keep the total number of concurrent consumers within the capacity of the downstream event queues. For example, configure two workers per consumer when deploying four replicas to limit a subscription to up to eight concurrent consumers:

.. code-block:: yaml

  config:
    storageEvents:
      s3-events:
        sqs:
          workers: 2
      azure-blob-events:
        azureServiceBus:
          workers: 2
