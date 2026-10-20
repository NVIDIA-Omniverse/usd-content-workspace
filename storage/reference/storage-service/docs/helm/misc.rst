Miscellaneous
=============

OpenAPI
-------

``service.rest.serveOpenapi``
  :Required: no
  :Type: boolean
  :Default: ``true``

  Serves the OpenAPI documents on the REST port. ``false`` also removes the Swagger UI, which renders from them.

Service account
---------------

The following values configure the Kubernetes ServiceAccount used by the Deployment.

``serviceAccount.create``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Creates a ServiceAccount for the Deployment. When ``false``, the Deployment uses the existing ServiceAccount named by ``serviceAccount.name``. A created account uses the release-qualified chart fullname unless ``serviceAccount.name`` is set.

``serviceAccount.automount``
  :Required: no
  :Type: boolean
  :Default: ``false``

  Controls whether Kubernetes pod credentials are mounted in running pods through ``automountServiceAccountToken`` on the ServiceAccount created by the chart. Keep it set to ``false`` unless Kubernetes credentials are needed.

``serviceAccount.annotations``
  :Required: no
  :Type: object
  :Default: ``{}``

  Annotations for the ServiceAccount created by the chart, for example cloud-provider IAM annotations.

``serviceAccount.name``
  :Required: no
  :Type: string

  Name of the ServiceAccount to create or use. When ``serviceAccount.create`` is ``false``, this names the existing ServiceAccount used by the Deployment. When creating an account, leave this value unset to use the release-qualified chart fullname.

Container probes
----------------

The chart configures Kubernetes liveness and readiness probes for the Storage Service container. The probe configurations are passed to the Deployment verbatim, except that the chart adds the required gRPC probe handler and sets its port to the named ``probe`` container port (8100).

``probes.liveness``
  :Required: no
  :Type: `Probe <https://kubernetes.io/docs/reference/kubernetes-api/core/pod-v1/#Probe>`_
  :Default: ``{periodSeconds: 5, failureThreshold: 6}``

  Configuration for the Storage Service container's liveness probe.

``probes.readiness``
  :Required: no
  :Type: `Probe <https://kubernetes.io/docs/reference/kubernetes-api/core/pod-v1/#Probe>`_
  :Default: ``{periodSeconds: 5, failureThreshold: 6}``

  Configuration for the Storage Service container's readiness probe.

Configure probe timing and threshold fields, such as ``periodSeconds`` and ``failureThreshold``, under the applicable object.

.. warning::

  Do not configure another probe handler, such as ``exec`` or ``httpGet``. The chart always configures the required gRPC probe handler; using an alternative handler results in undefined behavior.

Pod customization
-----------------

The following root-level values customize the Deployment pod and its containers. Environment variables and volume mounts are added to the chart-managed Storage Service and bootstrap containers only. Custom init containers and sidecars must declare their own environment variables, resource requirements, security context, and volume mounts.

``podAnnotations``
  :Required: no
  :Type: object
  :Default: ``{}``

  Annotations added to the pod template, for example annotations used by monitoring, service mesh, or secret-injection systems.

``podLabels``
  :Required: no
  :Type: object
  :Default: ``{}``

  Labels added to the pod template in addition to the chart labels.

``extraEnvs``
  :Required: no
  :Type: list[`EnvVar <https://kubernetes.io/docs/reference/kubernetes-api/core/pod-v1/#EnvVar>`_]
  :Default: ``[]``

  Environment variables added to the Storage Service and bootstrap containers. Values may come directly from the values file or from a ConfigMap or Secret reference.

``extraEnvFrom``
  :Required: no
  :Type: list[`EnvFromSource <https://kubernetes.io/docs/reference/kubernetes-api/core/pod-v1/#EnvFromSource>`_]
  :Default: ``[]``

  ConfigMap and Secret sources used to populate environment variables in the Storage Service and bootstrap containers.

``extraContainers``
  :Required: no
  :Type: list[`Container <https://kubernetes.io/docs/reference/kubernetes-api/core/pod-v1/#Container>`_]
  :Default: ``[]``

  Sidecar containers appended to the pod. Each sidecar must declare its own container-level settings.

``extraInitContainers``
  :Required: no
  :Type: list[`Container <https://kubernetes.io/docs/reference/kubernetes-api/core/pod-v1/#Container>`_]
  :Default: ``[]``

  Init containers appended to the pod. Each custom init container must declare its own container-level settings.

``extraVolumes``
  :Required: no
  :Type: list[`Volume <https://kubernetes.io/docs/reference/kubernetes-api/core/pod-v1/#Volume>`_]
  :Default: ``[]``

  Volumes appended to the pod specification.

``extraVolumeMounts``
  :Required: no
  :Type: list[`VolumeMount <https://kubernetes.io/docs/reference/kubernetes-api/core/pod-v1/#VolumeMount>`_]
  :Default: ``[]``

  Volume mounts added to the Storage Service and bootstrap containers. A custom container must declare its own mounts, and every mount must reference a pod volume.

``podSecurityContext``
  :Required: no
  :Type: `PodSecurityContext <https://kubernetes.io/docs/reference/kubernetes-api/workload-resources/pod-v1/#security-context>`_
  :Default: ``{runAsNonRoot: true, runAsUser: 1000, runAsGroup: 1000}``

  Security context applied to the pod. The value is replaced verbatim rather than merged.

``securityContext``
  :Required: no
  :Type: `SecurityContext <https://kubernetes.io/docs/reference/kubernetes-api/workload-resources/pod-v1/#security-context-1>`_
  :Default: ``{allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}``

  Security context applied to the Storage Service and bootstrap containers. The value is replaced verbatim rather than merged.

The image declares its user by name (``USER nvs:nvs``, uid and gid ``1000``). The kubelet cannot check a named user against ``runAsNonRoot`` and refuses to start the pod with ``container has runAsNonRoot and image has non-numeric user`` unless the numeric id is also given, which is why ``podSecurityContext.runAsUser`` is set rather than left to the image. If you run a rebuilt image whose uid differs, override ``runAsUser`` and ``runAsGroup`` together with ``runAsNonRoot``.

For example, the following configuration adds a pod annotation, an environment variable sourced from a Secret, a shared empty directory, and a sidecar that declares its own volume mount and resources:

.. code-block:: yaml

  podAnnotations:
    example.com/configured: "true"
  extraEnvs:
    - name: API_TOKEN
      valueFrom:
        secretKeyRef:
          name: storage-service-secrets
          key: api-token
  extraVolumes:
    - name: shared-data
      emptyDir: {}
  extraVolumeMounts:
    - name: shared-data
      mountPath: /var/lib/storage-service
  extraContainers:
    - name: sidecar
      image: example.com/sidecar:latest
      volumeMounts:
        - name: shared-data
          mountPath: /var/lib/sidecar
      resources:
        requests:
          cpu: 100m
          memory: 128Mi
        limits:
          cpu: 200m
          memory: 256Mi
