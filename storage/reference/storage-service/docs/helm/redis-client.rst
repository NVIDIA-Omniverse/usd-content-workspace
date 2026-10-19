Redis client
============

Storage Service uses Redis or Valkey as a shared L2 cache and to exchange cache invalidation messages across replicas. Redis client settings are located under the ``config.redis`` object:

``config.redis.enabled``
  :Required: yes
  :Type: boolean
  :Default: ``false``

  Whether to enable the Redis client.

``config.redis.address``
  :Required: when ``config.redis.enabled`` is set to ``true``
  :Type: string

  Address of the Redis or Valkey server. The following formats are supported:

  * ``redis://[[username:]password@]host[:port][/database]`` - An unencrypted TCP connection to a server. For example:

    * ``redis://myhost.mydomain`` - Connects to Redis database ``0`` at ``myhost.mydomain:6379``.
    * ``redis://myuser:mypassword@myhost.mydomain:1234/1`` - Connects to Redis database ``1`` at ``myhost.mydomain:1234``.

  * ``rediss://[[username:]password@]host[:port][/database]`` - A TLS-encrypted connection to a server. The ``username``, ``password``, ``host``, ``port``, and ``database`` components have the same meaning as in the ``redis`` format.

``config.redis.namespace``
  :Required: no
  :Type: string
  :Default: ``storage-service``

  Namespace prefix applied to every key the service writes. Use a unique prefix for each deployment.

``config.redis.commandTimeout``
  :Required: no
  :Type: duration
  :Default: ``100ms``

  Per-command timeout.

``config.redis.connectTimeout``
  :Required: no
  :Type: duration
  :Default: ``10s``

  Connection timeout.

``config.redis.broadcastChannelCapacity``
  :Required: no
  :Type: integer
  :Default: ``1024``

  Capacity of the subscriber broadcast channel.

``config.redis.poolSize``
  :Required: no
  :Type: integer
  :Default: ``8``

  Connection pool size.

``config.redis.reconnectMaxAttempts``
  :Required: no
  :Type: integer
  :Default: ``0``

  Maximum reconnect attempts after an established connection drops. Set to ``0`` to retry forever.

``config.redis.reconnectMinDelay``
  :Required: no
  :Type: duration
  :Default: ``100ms``

  Initial exponential reconnect delay.

``config.redis.reconnectMaxDelay``
  :Required: no
  :Type: duration
  :Default: ``30s``

  Maximum exponential reconnect delay.

``config.redis.reconnectBase``
  :Required: no
  :Type: integer
  :Default: ``2``

  Exponential reconnect multiplier. Must be greater than ``1``.

``config.redis.reconnectJitter``
  :Required: no
  :Type: duration
  :Default: ``50ms``

  Jitter added to each reconnect delay.

A minimal configuration looks like this:

.. code-block:: yaml

  config:
    redis:
      enabled: true
      address: "redis://valkey.infra.svc.cluster.local:6379"
      # Although it is not required, we recommend providing a unique namespace
      # prefix for each deployment.
      namespace: "storage-service-dev"

Version compatibility
---------------------

The client requires Redis 8.4 or later, or Valkey 8.1 or later.
