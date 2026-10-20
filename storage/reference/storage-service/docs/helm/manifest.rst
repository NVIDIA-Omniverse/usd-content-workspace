Manifest file support
=====================

The manifest layer overlays read-only files from other resource identities onto a workspace folder. A workspace is a folder whose name ends with ``.wkspc``. Its optional ``.manifest`` file defines the overlay; paths not listed in the manifest continue to use the corresponding object in the workspace folder. The manifest file itself remains a regular backend object that can be read, written, or deleted through the Storage API.

The manifest layer is enabled with ``config.manifest.enabled``. When ``config.manifest.errorOnNonManifestObjects`` is true, requests for objects outside a workspace are rejected instead of being passed through to the storage backend. The following settings are located under the ``config.manifest`` object:

``config.manifest.enabled``
  :Required: yes
  :Type: boolean
  :Default: ``false``

  Enables manifest file support.

``config.manifest.l2Enabled``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Enables the optional Redis/Valkey L2 tier for the manifest cache. Requires the :doc:`Redis client <redis-client>` to be enabled.

``config.manifest.errorOnNonManifestObjects``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Rejects requests for objects outside a workspace when true; when false, those requests pass through to the storage backend.

``config.manifest.cacheCapacity``
  :Required: no
  :Type: size
  :Default: ``1GiB``

  Maximum memory used by the manifest cache before eviction. This value is included when calculating the container memory limit as described in :doc:`scalability`.

``config.manifest.cacheTimeToLive``
  :Required: no
  :Type: duration
  :Default: ``24h``

  Maximum time a manifest remains in the cache.

``config.manifest.cacheCapacityWarnPct``
  :Required: no
  :Type: integer
  :Default: ``95``

  Cache utilization percentage at which the service logs a warning. Must be an integer from 0 to 100.

``config.manifest.pageSizeLimit``
  :Required: no
  :Type: size
  :Default: ``3MiB``

  Maximum response size before the service begins a new streamed response page.

For example, the following configuration enables the manifest layer and its shared Redis/Valkey cache tier:

.. code-block:: yaml

  config:
    redis:
      enabled: true
      address: "redis://valkey.infra.svc.cluster.local:6379"
    manifest:
      enabled: true
      l2Enabled: true

Manifest file format
--------------------

A JSON manifest must be located at ``<backend-base>/<workspace>.wkspc/.manifest`` for the overlay to be active. The file uses format version ``1.0`` and contains one entry for every overlaid file. Each entry specifies a workspace-relative path and exactly one target: either ``resource_identity`` or ``resource_info``. A resource identity is obtained from the target object's ``Stat`` response.

.. code-block:: json

  {
    "format_version": {
      "version_string": "1.0"
    },
    "storage_api_version": "v1beta",
    "files": [
      {
        "relative_path": "pkg/example.usd",
        "resource_identity": "https://store.example/objects/example.usd?etag=%22abc%22"
      }
    ]
  }

``relative_path`` must be a unique, non-empty relative path. It cannot start with ``/``, contain ``.`` or ``..`` path segments, contain a ``.wkspc`` segment, or be named ``.manifest``. A malformed manifest disables access to the overlay paths until the manifest is corrected; the ``.manifest`` file itself remains available for repair.
