Storage response caching
========================

Stat Cache
----------

The Stat Cache is used to cache the response for the Stat request.

The responses for successful requests or those failing with a code from ``config.statCache.errors`` will be cached.

``config.statCache.enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Whether to enable the cache.

``config.statCache.l2Enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Whether to enable the Redis/Valkey L2 cache tier.

``config.statCache.capacity``
  :Required: no
  :Type: size
  :Default: ``1GiB``

  Maximum capacity for the cached metadata.

``config.statCache.timeToLive``
  :Required: no
  :Type: duration
  :Default: ``15m``

  Cache entry time-to-live.

``config.statCache.invalidateOnUpdate``
  :Required: no
  :Type: boolean
  :Default: ``true``

  Invalidate cache on object updates.

``config.statCache.errors``
  :Required: no
  :Type: list[string]
  :Default: ``[AccessDenied, NotFound]``

  `Error types <#cache-error-types>`_ to cache.

When this cache is enabled, its ``config.statCache.capacity`` value must be considered when configuring the container memory limit.

An example configuration with the Stat Cache enabled could look as follows:

.. code-block:: yaml

  config:
    statCache:
      enabled: true

If you don't want to cache errors, set the ``config.statCache.errors`` property to an empty list:

.. code-block:: yaml

  config:
    statCache:
      enabled: true
      errors: []

List Cache
----------

The List Cache is used to cache the response for the List and ListStat requests. This cache also caches and prefetches Stat responses. When a resource address is Stat'd, this cache will perform a ListStat for the parent folder address and cache the result. The stat response is then served from the cache.

The responses for successful requests or those failing with a code from ``config.listCache.errors`` will be cached.

``config.listCache.enabled``
  :Required: no
  :Type: boolean
  :Default: ``true``

  Whether to enable the cache.

``config.listCache.l2Enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Whether to enable the Redis/Valkey L2 cache tier.

``config.listCache.capacity``
  :Required: no
  :Type: size
  :Default: ``1GiB``

  Maximum capacity for the cached metadata.

``config.listCache.pageSizeLimit``
  :Required: no
  :Type: size
  :Default: ``3MiB``

  Response page size limit in that the cache can return before sending a new page.

``config.listCache.timeToLive``
  :Required: no
  :Type: duration
  :Default: ``15m``

  Cache entry time-to-live.

``config.listCache.invalidateOnUpdate``
  :Required: no
  :Type: boolean
  :Default: ``true``

  Invalidate cache on object updates.

``config.listCache.errors``
  :Required: no
  :Type: list[string]
  :Default: ``[AccessDenied, NotFound]``

  `Error types <#cache-error-types>`_ to cache.

When this cache is enabled, its ``config.listCache.capacity`` value must be considered when configuring the container memory limit.

An example configuration with the List Cache enabled could look as follows:

.. code-block:: yaml

  config:
    listCache:
      enabled: true

Small Object Cache
------------------

The Small Object Cache is used to cache small objects in the FileObject API calls.

The responses for successful requests or those failing with a code from ``config.smallObjectCache.errors`` will be cached.

``config.smallObjectCache.enabled``
  :Required: no
  :Type: boolean
  :Default: ``true``

  Whether to enable the cache.

``config.smallObjectCache.l2Enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Whether to enable the Redis/Valkey L2 cache tier.

``config.smallObjectCache.capacity``
  :Required: no
  :Type: size
  :Default: ``1GiB``

  Maximum capacity for the cached metadata.

``config.smallObjectCache.smallObjectSize``
  :Required: no
  :Type: size
  :Default: ``256KiB``

  Maximum size of individual objects to cache.

``config.smallObjectCache.timeToLive``
  :Required: no
  :Type: duration
  :Default: ``15m``

  Cache entry time-to-live.

``config.smallObjectCache.inFlightReadChannelCapacity``
  :Required: no
  :Type: integer
  :Default: ``30``

  Channel capacity for concurrent read operations.

``config.smallObjectCache.errors``
  :Required: no
  :Type: list[string]
  :Default: ``[AccessDenied, NotFound]``

  `Error types <#cache-error-types>`_ to cache.

When this cache is enabled, its ``config.smallObjectCache.capacity`` value must be considered when configuring the container memory limit.

An example configuration with the Small Object Cache enabled could look as follows:

.. code-block:: yaml

  config:
    smallObjectCache:
      enabled: true

Cache error types
-----------------

The table below provides an overview of supported cached error types:

.. list-table::
  :header-rows: 1

  * - Name
    - Description
  * - ``Cancelled``
    - The operation was cancelled
  * - ``Unknown``
    - Unknown error
  * - ``InvalidArgument``
    - Client specified an invalid argument
  * - ``DeadlineExceeded``
    - Deadline expired before operation could complete
  * - ``NotFound``
    - Some requested entity was not found
  * - ``AlreadyExists``
    - Some entity that we attempted to create already exists
  * - ``PermissionDenied``
    - The caller does not have permission to execute the specified operation
  * - ``AccessDenied``
    - Alias to ``PermissionDenied``
  * - ``ResourceExhausted``
    - Some resource has been exhausted
  * - ``FailedPrecondition``
    - The system is not in a state required for the operation's execution
  * - ``Aborted``
    - The operation was aborted
  * - ``OutOfRange``
    - Operation was attempted past the valid range
  * - ``Unimplemented``
    - Operation is not implemented or not supported
  * - ``Internal``
    - Internal error
  * - ``Unavailable``
    - The service is currently unavailable
  * - ``DataLoss``
    - Unrecoverable data loss or corruption
  * - ``Unauthenticated``
    - The request does not have valid authentication credentials
