URL normalizer
==============

The URL normalizer rewrites the path component of resource and folder addresses before requests reach the permission, routing and backend layers. It collapses empty path segments caused by repeated ``/`` characters and resolves the path traversal segments ``.`` and ``..``. The scheme, authority, query and fragment components remain unchanged.

Path segments are collapsed after percent-decoding. Encoded path separators ``%2f`` and ``%2F`` are decoded and treated as the path separator ``/``. Encoded dots ``%2e`` and ``%2E`` are decoded and treated as ``.`` when resolving ``.`` and ``..`` segments. These rules also apply to segments containing a mixture of literal and percent-encoded slashes or dots: ``a/%2F/b`` becomes ``a/b``, and ``a/.%2E/b`` becomes ``b``.

In the Helm chart, URL normalizer settings are located under the ``config.urlNormalizer`` element.

``config.urlNormalizer.enabled``
  :Required: no
  :Type: bool
  :Default: ``true``

  Enable URL normalization. When disabled, resource addresses are forwarded to the backends exactly as the caller supplied them.

  .. warning::

     This setting defaults to ``true``, so normalization is applied unless a deployment opts out. Read `Effect on addressing`_ before upgrading a deployment that already holds data: normalization changes which object some addresses resolve to.

.. code-block:: yaml

  config:
    urlNormalizer:
      enabled: false

Effect on addressing
--------------------

Normalization is not transparent. Two of its rules change which object a request addresses:

* Repeated slashes in the path are collapsed, so ``https://account.blob.core.windows.net/container/a//b`` addresses the same blob as ``https://account.blob.core.windows.net/container/a/b``. Azure Blob Storage and S3 namespaces are flat, and ``a//b`` is a legal object key distinct from ``a/b``. Objects stored under keys containing empty path segments become unreachable, and writes to such addresses overwrite the collapsed key instead.
* ``.`` and ``..`` path segments are resolved against the address, so ``https://account.blob.core.windows.net/container/folder/../object`` addresses ``https://account.blob.core.windows.net/container/object``.

Because this setting defaults to ``true``, audit the stored object keys of every configured endpoint for empty path segments *before upgrading* a deployment that already holds data. Set ``config.urlNormalizer.enabled`` to ``false`` to keep the previous behaviour while that audit is carried out.
