Request throttling
==================

Ingress request controls
------------------------

Ingress request throttling limits the number of concurrent inbound requests processed by the service. Ingress load shedding rejects requests when the configured concurrency limit has been reached instead of allowing them to wait for capacity. Shed REST requests are cancelled with HTTP status ``429 Too Many Requests``; shed gRPC requests return ``RESOURCE_EXHAUSTED``. These controls apply to both REST and gRPC requests.

The following settings are located under the ``config.ingressThrottle`` and ``config.ingressLoadShed`` objects:

``config.ingressThrottle.enabled``
  :Required: no
  :Type: boolean
  :Default: ``true``

  Enables the ingress request throttling middleware.

``config.ingressThrottle.maxConcurrency``
  :Required: no
  :Type: integer
  :Default: ``8192``

  Maximum number of concurrently processed inbound requests.

``config.ingressLoadShed.enabled``
  :Required: no
  :Type: boolean
  :Default: ``true``

  Enables the ingress load-shedding middleware.

For example, the following configuration allows up to 1,024 concurrent inbound requests and rejects additional requests while capacity is unavailable:

.. code-block:: yaml

  config:
    ingressThrottle:
      enabled: true
      maxConcurrency: 1024
    ingressLoadShed:
      enabled: true

Egress request throttling
-------------------------

Egress request throttling limits the number of concurrent requests made to storage and metadata backends. It can be used to prevent the service from overwhelming a downstream backend.

.. warning::

  When egress request throttling is enabled, requests can remain queued indefinitely, increasing the application's memory working set. Ingress request throttling should always be enabled when egress request throttling is enabled.

The following settings are located under the ``config.egressThrottle`` object:

``config.egressThrottle.enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Enables the egress request throttling middleware.

``config.egressThrottle.maxConcurrency``
  :Required: no
  :Type: integer
  :Default: ``4096``

  Maximum number of concurrent requests to storage and metadata backends.

For example, the following configuration allows up to 256 concurrent backend requests:

.. code-block:: yaml

  config:
    egressThrottle:
      enabled: true
      maxConcurrency: 256
