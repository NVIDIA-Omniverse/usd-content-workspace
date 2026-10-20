URL remapper
============

The URL remapper rewrites resource URLs at request time: a rule's *source* prefix in the user-visible URL space is replaced with the rule's *target* prefix when the request is forwarded to a backend, and responses are rewritten back. Configure rules under ``config.urlRemapper.rewriteRules``.

In the Helm chart, URL remapper settings are located under the ``config.urlRemapper`` element.

Top-level settings
------------------

``config.urlRemapper.enabled``
  :Required: no
  :Type: bool
  :Default: ``false``

  Enable URL remapping.

``config.urlRemapper.checkpointMetadataKey``
  :Required: no
  :Type: string
  :Default: ``checkpoints``

  Metadata key used to identify checkpoints when remapping versioned URLs.

``config.urlRemapper.rewriteRules``
  :Required: no
  :Type: object

  Map of mapped-URL prefixes to original-URL prefixes. See below.

``config.urlRemapper.exposeChildRulesAsFolders``
  :Required: no
  :Type: bool
  :Default: ``false``

  Expose rewrite-rule source URLs as synthetic subfolders in ``list`` / ``list_stat``. See below.

Rewrite rules
-------------

``config.urlRemapper.rewriteRules`` is a map keyed by the mapped-URL prefix. The corresponding value is the original-URL prefix that the mapped prefix translates to.

.. code-block:: yaml

  config:
    urlRemapper:
      enabled: true
      rewriteRules:
        "omniverse://your.custom.hostname.com/": "https://storage.blob.core.windows.net/content/"
        "omniverse://your.custom.hostname.com/assets/": "https://mybucket.s3.amazonaws.com/assets/"
        "omniverse://models/": "https://mybucket.s3.amazonaws.com/models/"

Both prefixes must end with ``/``. Rules are applied longest-source-prefix first so deeper mounts win over shallower ones at the same path level.

Per-alias visibility
^^^^^^^^^^^^^^^^^^^^

A rule value may instead be an object carrying per-alias options. Aliases are always listed in ``ListRoutes``; set ``listInTopLevelAddresses: false`` to hide an alias from ``ListTopLevelAddresses`` while keeping it routable. The plain-string rule form is retained only for compatibility and will likely be removed in the next release.

``config.urlRemapper.rewriteRules.<alias>.target``
  :Required: yes
  :Type: string

  The original-URL prefix the mapped prefix translates to.

``config.urlRemapper.rewriteRules.<alias>.listInTopLevelAddresses``
  :Required: no
  :Type: boolean
  :Default: ``true``

  List this alias in ``ListTopLevelAddresses``.

.. code-block:: yaml

  config:
    urlRemapper:
      enabled: true
      rewriteRules:
        "omniverse://models/": "https://mybucket.s3.amazonaws.com/models/"
        "omniverse://internal/":
          target: "https://mybucket.s3.amazonaws.com/internal/"
          listInTopLevelAddresses: false

Synthetic-folder injection
--------------------------

When ``config.urlRemapper.exposeChildRulesAsFolders`` is ``true``, ``list`` and ``list_stat`` responses include synthetic subfolder entries for every rewrite-rule source URL that is a *direct child* of the requested folder. This makes mounted child buckets discoverable through plain folder navigation rather than requiring clients to know mount-point URLs out of band.

Parent URL shape
^^^^^^^^^^^^^^^^

The synthetic injection works for any parent URL shape. The parent does not have to use the ``omniverse://`` scheme; native cloud URLs work too. For example, with the rules:

.. code-block:: yaml

  config:
    urlRemapper:
      enabled: true
      exposeChildRulesAsFolders: true
      rewriteRules:
        "https://account.blob.core.windows.net/bucket-a/":
          target: "https://account.blob.core.windows.net/bucket-a-target/"
        "https://account.blob.core.windows.net/bucket-a/Folder/":
          target: "https://account.blob.core.windows.net/bucket-b/"

a ``list`` on ``https://account.blob.core.windows.net/bucket-a/`` returns the real contents of bucket A plus the synthetic subfolder ``https://account.blob.core.windows.net/bucket-a/Folder/``. Reads and writes under that synthetic folder transparently target bucket B.

Mount hierarchy: parent must resolve to a real backend
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

When ``exposeChildRulesAsFolders`` is ``true``, every non-top-level rewrite rule must nest under something that resolves to a real backend, so users never see a clickable synthetic folder that points nowhere. That parent is either:

* another rewrite rule whose source is the immediate parent path (one segment shallower), or
* a configured backend endpoint (an ``azureBlob`` or ``s3`` endpoint from the storage config). A rule whose source sits anywhere under a configured endpoint is a mount hosted directly by that endpoint and needs no covering rule.

Configurations that satisfy neither -- a mount under an unconfigured host, or one that skips an intermediate level under an alias -- are rejected at startup.

Valid (each child mount sits one segment below an existing rule):

.. code-block:: yaml

  config:
    urlRemapper:
      enabled: true
      exposeChildRulesAsFolders: true
      rewriteRules:
        "omniverse://main/":   "https://some.bucket.com/bucket1/"
        "omniverse://main/1/": "https://some.bucket.com/bucket2/"
        "omniverse://main/2/": "https://some.bucket.com/bucket3/"

Also valid (the mount sits directly under a configured Azure storage account endpoint, so the endpoint is its parent -- no alias is required):

.. code-block:: yaml

  config:
    storage:
      azureBlob:
        endpoints:
          "https://account.blob.core.windows.net/": { ... }
    urlRemapper:
      enabled: true
      exposeChildRulesAsFolders: true
      rewriteRules:
        "https://account.blob.core.windows.net/MountedBucket/": 
          target: "https://bucket.s3.us-west-2.amazonaws.com/"

The Azure adapter lists a storage account's containers, not the account root itself, so ``https://account.blob.core.windows.net/`` is never returned as a browsable top-level address. The mount therefore keeps its own top-level entry so it stays discoverable -- it is *not* suppressed, because there is no browsable endpoint root to reach it through.

Suppression only applies when the hosting endpoint's own root URL is itself a listed top-level address. That is the case for an ``s3`` bucket endpoint, whose bucket URL is advertised verbatim:

.. code-block:: yaml

  config:
    storage:
      s3:
        endpoints:
          "https://bucket.s3.us-west-2.amazonaws.com/": { ... }
    urlRemapper:
      enabled: true
      exposeChildRulesAsFolders: true
      rewriteRules:
        "https://bucket.s3.us-west-2.amazonaws.com/MountedFolder/":
          target: "https://other-bucket.s3.us-west-2.amazonaws.com/"

Here ``https://bucket.s3.us-west-2.amazonaws.com/`` is a top-level address, and listing it returns the bucket's real contents plus the synthetic ``MountedFolder/`` folder. The mount is reachable by browsing into the bucket, so it is not also surfaced as a standalone top-level address.

Rejected (``omniverse://main/public-assets/`` is referenced as a parent path by both children but is neither a rule nor a configured endpoint):

.. code-block:: yaml

  config:
    urlRemapper:
      enabled: true
      exposeChildRulesAsFolders: true
      rewriteRules:
        "omniverse://main/":                     "https://some.bucket.com/bucket1/"
        "omniverse://main/public-assets/1/":     "https://some.bucket.com/bucket2/"
        "omniverse://main/public-assets/2/":     "https://some.bucket.com/bucket3/"

To fix the second config, add an explicit rule at ``omniverse://main/public-assets/`` mapping it to a real backend.

Collisions
^^^^^^^^^^

When a real folder address and a synthetic mount-point share the same URL, the rule wins -- the real entry is dropped and a warning is logged so operators can fix the configuration.

Mount-point folder operations
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

``create_folder`` and ``delete_folder`` requests that target a mount-point URL exactly are rejected with ``INVALID_ARGUMENT``. Mount points are defined here in the chart configuration and must be edited here, not via the folder APIs.
