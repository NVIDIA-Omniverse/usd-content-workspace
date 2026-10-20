Contour HTTP Proxy
==================

The Helm chart can expose the Storage API through Contour ``HTTPProxy`` custom resources. Configure the proxy under the ``httpProxy`` object when Contour is installed in the Kubernetes cluster.

``httpProxy.enabled``
  :Required: yes
  :Type: boolean
  :Default: ``false``

  Whether to create the ``HTTPProxy`` resources that expose Storage Service.

``httpProxy.fqdn.domain``
  :Required: yes
  :Type: string
  :Default: ``nodomain.none``

  Domain portion of the REST and gRPC virtual-host names.

``httpProxy.fqdn.host``
  :Required: no
  :Type: string
  :Default: ``storage``

  Host portion of the REST and gRPC virtual-host names.

``httpProxy.path``
  :Required: no
  :Type: string

  The path prefix for REST requests.

``httpProxy.corsPolicy``
  :Required: no
  :Type: object
  :Default: ``{allowCredentials: false, allowOrigin: [], allowHeaders: ["*"], allowMethods: ["GET", "PUT", "POST", "HEAD", "PATCH", "DELETE"], exposeHeaders: ["*"]}``

  CORS policy applied to both virtual hosts.

``httpProxy.responseHeadersPolicy``
  :Required: no
  :Type: object

  Response-header policy applied to proxy routes.

``httpProxy.timeoutPolicy.response``
  :Required: no
  :Type: `google.protobuf.Duration <https://protobuf.dev/reference/protobuf/google.protobuf/#duration>`_
  :Default: ``90s``

  Upstream timeout for standard routes. See Envoy's `timeout field <https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/route/v3/route_components.proto#envoy-v3-api-field-config-route-v3-routeaction-timeout>`_.

``httpProxy.timeoutPolicy.idle``
  :Required: no
  :Type: `google.protobuf.Duration <https://protobuf.dev/reference/protobuf/google.protobuf/#duration>`_
  :Default: ``90s``

  Idle timeout for standard routes. See Envoy's `idle_timeout field <https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/route/v3/route_components.proto#envoy-v3-api-field-config-route-v3-routeaction-idle-timeout>`_.

``httpProxy.streamTimeoutPolicy.response``
  :Required: no
  :Type: `google.protobuf.Duration <https://protobuf.dev/reference/protobuf/google.protobuf/#duration>`_
  :Default: ``90s``

  Upstream timeout for streaming routes. See Envoy's `timeout field <https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/route/v3/route_components.proto#envoy-v3-api-field-config-route-v3-routeaction-timeout>`_.

``httpProxy.streamTimeoutPolicy.idle``
  :Required: no
  :Type: `google.protobuf.Duration <https://protobuf.dev/reference/protobuf/google.protobuf/#duration>`_
  :Default: ``90s``

  Idle timeout for streaming routes. See Envoy's `idle_timeout field <https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/route/v3/route_components.proto#envoy-v3-api-field-config-route-v3-routeaction-idle-timeout>`_.

``httpProxy.annotations``
  :Required: no
  :Type: object

  Annotations added to both ``HTTPProxy`` resources.

``httpProxy.authExtension``
  :Required: no
  :Type: object

  Authorization extension settings applied to both virtual hosts. See :ref:`Authorization extension settings <http-proxy-auth-extension>` for details.

``httpProxy.grpc``
  :Required: no
  :Type: object

  Overrides for the gRPC ``HTTPProxy`` configuration.

``httpProxy.rest``
  :Required: no
  :Type: object

  Overrides for the ``HTTPProxy`` configuration for the REST service.

Service FQDN
------------

The chart creates one ``HTTPProxy`` resource for REST requests at ``<host>.<domain>`` and another for gRPC requests at ``grpc.<host>.<domain>``. The REST proxy routes requests under ``httpProxy.path``. The gRPC proxy always routes requests from ``/``.

TLS settings
------------

TLS terminates at Contour. When enabled, the chart references the same certificate secret from the REST and gRPC ``HTTPProxy`` resources. The secret must be in the release namespace and its certificate must cover both ``<host>.<domain>`` and ``grpc.<host>.<domain>``.

``httpProxy.tls.enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Whether to configure TLS for the virtual hosts.

``httpProxy.tls.secretName``
  :Required: no
  :Type: string
  :Default: ``tls-cert-<release-qualified chart name>``

  Name of the Kubernetes secret containing the TLS certificate. When omitted, the chart derives the name from the release-qualified chart name.

TLS certificate secret format
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

The TLS certificate secret must have the same format as a secret created by ``kubectl create secret tls``. For example:

.. code-block:: shell

  kubectl --namespace storage-apis create secret tls storage-service-tls-cert \
    --cert=public.pem --key=private.pem

CORS policy
-----------

The CORS policy is applied to both virtual hosts. Configure ``allowCredentials``, ``allowHeaders``, ``allowMethods``, ``allowOrigin``, and ``exposeHeaders`` using the corresponding Contour ``HTTPProxy`` CORS-policy fields. When ``allowOrigin`` is not configured, the chart derives an allowed-origin pattern from the configured domain.

Response-header policy
----------------------

Use ``responseHeadersPolicy`` to set or remove response headers on proxy routes. The policy has the Contour ``HTTPProxy`` response-header-policy format. See the `Contour HeadersPolicy API reference <https://projectcontour.io/docs/1.30/config/api/#projectcontour.io/v1.HeadersPolicy>`_ for its supported fields.

.. code-block:: yaml

  httpProxy:
    responseHeadersPolicy:
      set:
        - name: Strict-Transport-Security
          value: max-age=31536000
      remove:
        - X-Response-Id

.. _http-proxy-auth-extension:

Authorization extension settings
--------------------------------

Enable the authorization extension to delegate request authorization to an Envoy extension service.

``httpProxy.authExtension.enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Whether to configure the authorization extension for the virtual hosts.

``httpProxy.authExtension.extensionRef.name``
  :Required: no
  :Type: string
  :Default: ``auth-extension``

  Name of the Envoy extension service.

``httpProxy.authExtension.extensionRef.namespace``
  :Required: no
  :Type: string

  Namespace of the Envoy extension service. If omitted, the chart uses the release namespace.

``httpProxy.authExtension.responseTimeout``
  :Required: no
  :Type: `google.protobuf.Duration <https://protobuf.dev/reference/protobuf/google.protobuf/#duration>`_
  :Default: ``2s``

  Maximum time to wait for the extension service to respond.

Protocol overrides
------------------

The ``httpProxy.grpc`` and ``httpProxy.rest`` objects are deep-merged with the shared ``httpProxy`` settings. Use them to apply different CORS policies, annotations, authorization extensions, response-header policies, or timeout policies without duplicating shared configuration.

.. code-block:: yaml

  httpProxy:
    timeoutPolicy:
      response: 10s
      idle: 10s
    grpc:
      timeoutPolicy:
        response: 30s
    rest:
      annotations:
        projectcontour.io/ingress.class: internal

The REST route path always comes from ``httpProxy.path``, and the gRPC route path is always ``/``. Protocol overrides do not change either path. The TLS secret name is also shared by both virtual hosts.
