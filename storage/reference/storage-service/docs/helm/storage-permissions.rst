Permission service integration
==============================

The Storage Service can be configured to delegate authorization decisions to an external permission service. Permission settings live under the ``config.permissions`` object.

``config.permissions.enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Whether to authorize storage operations through the permission service.

``config.permissions.client``
  :Required: no
  :Type: object
  :Default: ``{grpc: {endpoint: http://permission-service:3010}}``

  Permission service client settings; either ``rest`` or ``grpc`` (see below). When omitted and ``enabled`` is ``true``, defaults to gRPC at ``http://permission-service:3010``.

``config.permissions.batchSize``
  :Required: no
  :Type: integer
  :Default: ``100``

  Maximum number of permission checks sent in a single batch request.

``config.permissions.includeAclInAllKeys``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Whether to include the ``acl`` key in ``get_metadata`` responses when no keys are requested.

``config.permissions.actionNames``
  :Required: no
  :Type: map[:ref:`action name <permission-action-names>`, string]

  Override the mapping of Storage Service operations to permission action names.

Configure the incoming headers forwarded to permission queries with ``observability.headers.forward``; see :doc:`observability`.

REST client
-----------

The REST client connects to a single permission service endpoint over HTTP. Provide settings under ``config.permissions.client.rest``.

``config.permissions.client.rest.endpoint``
  :Required: no
  :Type: string

  Permission service endpoint address, no trailing slash.

``config.permissions.client.rest.retry.initialBackoff``
  :Required: no
  :Type: duration
  :Default: ``100ms``

  Base delay before the first retry of a transient failure. The delay doubles on later retries, up to ``maxBackoff``. Random jitter can affect the actual delay.

``config.permissions.client.rest.retry.maxBackoff``
  :Required: no
  :Type: duration
  :Default: ``10s``

  Maximum base delay between retries.

``config.permissions.client.rest.retry.attempts``
  :Required: no
  :Type: positive integer
  :Default: ``5``

  Maximum number of retries after the initial request.

An example REST configuration could look as follows:

.. code-block:: yaml

  config:
    permissions:
      enabled: true
      client:
        rest:
          endpoint: "http://permissions-service.default.svc.cluster.local:3010"

gRPC client
-----------

The gRPC client opens a small pool of connections per resolved backend address and balances requests across them per call. Pointing the endpoint at a `headless Service <https://kubernetes.io/docs/concepts/services-networking/service/#headless-services>`_ spreads requests across all permission service pods; a regular ClusterIP endpoint resolves to a single backend. The client periodically re-resolves DNS to track scale up/down without a restart. The ``:authority`` pseudo-header on outgoing requests is derived from ``endpoint`` (not the per-pod IP that is dialled), so a reverse proxy that routes by host keeps working without extra configuration. Both ``http://`` (plaintext) and ``https://`` (TLS) endpoint schemes are supported; the scheme in ``endpoint`` selects the connection security.

``config.permissions.client.grpc.endpoint``
  :Required: no
  :Type: string
  :Default: ``http://permission-service:3010``

  Permission service endpoint address.

``config.permissions.client.grpc.connectionsPerHost``
  :Required: no
  :Type: integer
  :Default: ``4``

  Number of TCP connections opened to each resolved backend instance.

``config.permissions.client.grpc.dnsRefreshInterval``
  :Required: no
  :Type: duration
  :Default: ``5s``

  How often to re-resolve the endpoint DNS to discover new pods; keep at or below the cluster's CoreDNS TTL.

``config.permissions.client.grpc.tcpKeepalive``
  :Required: no
  :Type: duration
  :Default: ``30s``

  TCP keep-alive interval on each per-pod subchannel.

``config.permissions.client.grpc.http2KeepAliveInterval``
  :Required: no
  :Type: duration
  :Default: ``30s``

  HTTP/2 keep-alive ping interval on each per-pod subchannel.

``config.permissions.client.grpc.retry.initialBackoff``
  :Required: no
  :Type: duration
  :Default: ``100ms``

  Base delay before the first retry of a transient failure. The delay doubles on later retries, up to ``maxBackoff``. Random jitter can affect the actual delay.

``config.permissions.client.grpc.retry.maxBackoff``
  :Required: no
  :Type: duration
  :Default: ``10s``

  Maximum base delay between retries.

``config.permissions.client.grpc.retry.attempts``
  :Required: no
  :Type: positive integer
  :Default: ``5``

  Maximum number of retries after the initial request.

A configuration pointing at a headless Service could look as follows:

.. code-block:: yaml

  config:
    permissions:
      enabled: true
      client:
        grpc:
          endpoint: "http://permissions-service-headless.default.svc.cluster.local:3010"
          connectionsPerHost: 4
          dnsRefreshInterval: "5s"

Address listing filtering
-------------------------

When permissions are enabled, the address-discovery listings are filtered per caller:

* ``ListTopLevelAddresses`` only returns addresses the caller is explicitly allowed to read (action: ``list_top_level_addresses``).
* ``ListRoutes`` follows the same rule (action: ``list_routes``); wildcard routes cannot name a resource and are always advertised.

Url-remapper aliases in both listings are evaluated against their remapped (original) address, which is the address permissions are recorded for.

Endpoints that opted out of permission enforcement (``permissionsEnabled: false``) are always listed, as are aliases whose remapped address resolves to such an endpoint.

.. _permission-action-names:

Action name mapping
-------------------

By default, read-style storage operations map to the ``read`` permission action and write-style operations map to ``write``. To customize the mapping, set ``config.permissions.actionNames`` to an object whose keys are storage operations and whose values are the corresponding permission action names. Recognized operations are: ``stat``, ``enumerate_versions``, ``read``, ``read_from_address``, ``write``, ``upload_part``, ``complete_multipart_upload``, ``abort_multipart_upload``, ``complete_redirect_upload``, ``copy_from``, ``copy_to``, ``delete``, ``enumerate``, ``list``, ``list_stat``, ``fetch_write_type_info``, ``list_top_level_addresses``, ``list_routes``, ``get_metadata``, ``update_metadata``, ``delete_metadata``, and ``query_metadata``.

.. code-block:: yaml

  config:
    permissions:
      enabled: true
      actionNames:
        stat: "read"
        read: "read"
        write: "write"
        delete: "admin"
        update_metadata: "write"
