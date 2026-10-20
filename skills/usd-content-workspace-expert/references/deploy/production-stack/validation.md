# Validating the production stack

> **Applies to:** Storage Service, Auto Discovery, Contour ingress, auth bootstrap, Kit clients · **Released:** Developer Guide 1.3.0 inventory · **This repo's snapshot:** routes read from the operator ([`src/api/rest.rs`](../../../../../discovery/reference/discovery-service/operator/src/api/rest.rs)) and Storage Core / Storage Service REST handlers · **Sources:** the [source skill package](../../../NOTICE.md)

Validate outcomes, not exit codes. A pod in `Running`, a `helm install` that returned 0, or an HTTP 200 from a capabilities route does not prove that a client can read and write with its intended identity.

## 1. Inside the cluster

```bash
NS=<ns>
kubectl get pods,svc -n "$NS"

# Storage
kubectl -n "$NS" port-forward service/storage-service 8012:8012 &
curl -fsS http://127.0.0.1:8012/v1alpha/capabilities/services | jq .
curl -fsS http://127.0.0.1:8012/v2alpha/services | jq .                       # discovery capabilities (REST form)

# Discovery, internal catalog
kubectl -n "$NS" port-forward service/auto-discovery-service 8080:8080 8081:8081 &
curl -fsS http://127.0.0.1:8080/api/v2/services | jq '{v: ."schema-version", names: [.services[].name]}'
curl -fsS http://127.0.0.1:8081/api/v2/services | jq '{v: ."schema-version", names: [.services[].name]}'   # external view
```

Expect `schema-version` 2 and the Storage Service (and any other annotated services) in the internal catalog with `http://<svc>.<ns>.svc.cluster.local:<port>` URLs. The external catalog lists a service only when it has an external endpoint annotation or matching HTTPProxy route. Use the Service names your releases rendered.

Write/read round trip with the backend identity, using a declared endpoint:

```bash
ADDR=$(printf 'https://%s.s3.%s.amazonaws.com/validation/hello.txt' "<bucket>" "<region>" | jq -sRr @uri)
payload='Hello from Storage API'
size=$(printf '%s' "$payload" | wc -c | tr -d ' ')
curl -fsS -X PUT "http://127.0.0.1:8012/v1beta/fileobject/by-address/${ADDR}?data_object_size=${size}" \
  -H "Content-Type: application/octet-stream" --data-binary "$payload"
curl -fsS "http://127.0.0.1:8012/v1beta/fileobject/by-address/${ADDR}"
```

Compute `data_object_size` from the bytes you send, as above. A snapshot build (unreleased Storage Core changes) rejects a shorter body with 400 and silently truncates a longer one, so a wrong size can pass the `PUT` and still store the wrong bytes; compare the read-back body with the payload.

Objects above `readRedirectSize` (snapshot default 256 KiB) are served through presigned/SAS redirects; follow redirects (`curl -L`) and check that the redirect host is reachable from the client.

## 2. Through ingress, from a client network

```bash
curl -fsS https://<domain>/api/v2/services | jq .
curl -fsS https://storage.<domain>/v1alpha/capabilities/services
kubectl get httpproxy -n <ns>        # every proxy "valid"
```

The catalog URLs must be reachable from the network that receives them; cluster DNS names must not appear in the external catalog.

## 3. With authentication enabled

```bash
curl -fsS https://<domain>/api/v1/auth-config | jq .                                # public bootstrap
curl -sS -o /dev/null -w '%{http_code}\n' https://storage.<domain>/v1alpha/capabilities/services   # expect 401
```

The auth-config response has `openid_configuration` and a `clients` map (`client_library`, `navigator`, and `default` when a `default` registration exists) with `client_id` and `scope`. It proves only that bootstrap is configured. Then call a protected route with a real user token and confirm both an allowed and a denied operation when the Permission Service is enabled ([../permissions-and-auth.md](../permissions-and-auth.md)).

## 4. Kit

Kit 109.0.1 or later. Set the Discovery base URL (no `/api/v*/services`) before launch:

```bash
export OMNI_STORAGE_DISCOVERY=https://<domain>
./omni.app.full.sh
```

Add a connection with your storage URL, browse, then create, open, edit, and save a file. Client Library versions differ in Discovery v2 and auth support; check `omni.client.get_version()` in the running process ([../../components/client-library.md](../../components/client-library.md), [../../use/usd-workflows.md](../../use/usd-workflows.md)).

## 5. Optional layers

- Notifications: subscribe and trigger a write ([../../operate/notifications.md](../../operate/notifications.md)).
- Navigator: fetch its root page and confirm the Navigator HTML is served, not an IdP login page; then browse and upload as a real user ([../navigator.md](../navigator.md#validate)).
- Integration tests: [../../test/integration-tests.md](../../test/integration-tests.md). Record skipped and deselected tests.
- Discovery conformance runner against the external catalog: [../discovery.md](../discovery.md#verify).
