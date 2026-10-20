# discovery-chart

A Helm chart wrapper for deplying an example discovery service on a Simple NGINX Web Server

## Usage

To launch this Helm chart, follow these steps:

1. **Prerequisites**
   - Make sure you have [Helm](https://helm.sh/) installed on your system (version 3.x or higher).
   - You need access to a Kubernetes cluster.

2. **Clone or Download this Repository**

   ```shell
   git clone https://github.com/NVIDIA-Omniverse/usd-content-workspace.git
   cd usd-content-workspace/discovery/reference/discovery-service/chart
   ```

3. **Configure Values**
   
   Customize your Discovery services list.

   Your values file should contain a list of services to be registered by the discovery service.

   Each service should define:
    - id: Unique string identifier for the service
    - name: Human readable name of the service
    - type: Logical type/category of the service ('storage', 'permission', 'live-service', etc.)
    - grpc: gRPC endpoint URL for the service
    - rest: REST endpoint URL for the service

   For example, using `custom-values.yaml` as your values file you can add services as follows:

   ```yaml
   discovery:
      services:
         - id: "storage-service-02"
           name: "My Storage service"
           type: "storage"
           grpc: grpc://storage-service:8011
           rest: http://storage-service:8012
         - id: "permission-service-02"
           name: "My Permission service"
           type: "permission"
           grpc: grpc://permission-service:8080
           rest: http://permission-service:8081  
   ```


4. **Install the Chart**

   Change to the `helm` directory and run install with a custom values file:

   ```shell
   helm install discovery-service . -f custom-values.yaml
   ```

5. **Check Your Deployment**

   ```shell
   kubectl get pods
   kubectl get svc
   ```
6. **Testing the release**

   Run the Helm test to verify the discovery service is working correctly.

   **Enable smoke tests** by setting the following in your values.

   ```yaml
   smokeTest:
      enabled: true
   ```

   Then run the Helm test:

   ```shell
   helm test discovery-service --logs
   ```

   This will run smoke tests against the deployed service and display the logs.

### Full local Kubernetes deployment test

To exercise chart rendering, installation, health and API endpoints, the Helm
smoke test, the conformance Job, an upgrade rollout, and uninstall cleanup in
an isolated kind cluster, run:

```shell
./scripts/test-local-kind.sh
```

The production-compatible Discovery Service image must already exist in the
local Docker image store. See
[`tests/local/README.md`](tests/local/README.md) for image selection,
prerequisites, coverage, and debugging options.

7. **Uninstall the Chart**

   ```
   helm uninstall discovery-service
   ```

For more customization options, refer to the comments and instructions in `helm/values.yaml`.

## Third-Party Software Notice

    This project will download and install additional third-party open source software projects. Review the license terms of these open source projects before use.