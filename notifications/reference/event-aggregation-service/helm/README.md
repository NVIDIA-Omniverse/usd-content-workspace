This is the helm chart for the Event Aggregation Service.

# Prerequisites
## RabbitMQ
RabbitMQ should be set up in the cluster. The following are required for this service
- direct exchange for notifications
- user (with username and password) with permissions to publish to the above exchange
- dedicated vhost for notificatons

### Dev Setup
For our testing purposes, RabbitMQ was set up as follows:

```
helm install rabbitmq bitnami/rabbitmq-cluster-operator -n rabbitmq
kubectl apply -f rabbitmq-definitions-configmap.yaml -n rabbitmq
kubectl apply -f rabbitmq/rabbitmq-cluster.yaml -n rabbitmq
```
Note that this cluster is not currently production ready. See the [RabbitMQ cluster operator documentation](https://www.rabbitmq.com/kubernetes/operator/using-operator) for configuration options.

To retrieve the admin credentials for the deployed instance, run:
```
kubectl -n rabbitmq get secret rabbitmq-cluster-default-user -o jsonpath="{.data.username}" | base64 --decode
kubectl -n rabbitmq get secret rabbitmq-cluster-default-user -o jsonpath="{.data.password}" | base64 --decode
```

The configmap under `rabbitmq-definitions-configmap.yaml` defines the vhost and users for the notification services.

## Secrets
### Registry Credentials
The images are stored in NGC. To create the expected imagePullSecret, run:
```
kubectl create secret docker-registry regcred --docker-server="nvcr.io" --docker-username='$oauthtoken' --docker-password=$NGC_TOKEN --namespace=$NAMESPACE
```
Where $NGC_TOKEN is an NGC API key with access to the image registry, and $NAMESPACE is the namespace where the Event Aggregation Service will be deployed.

### Sensitive Environment Variables
Some environment variables are sensitive and stored in a secret. Currently, this includes the RabbitMQ URI, which may need to contain a username and password. To create the expected secret, run:
```
kubectl create secret generic event-aggregation-env -n $NAMESPACE --from-literal=rabbitmq_uri=$RABBITMQ_URI
```
Where $RABBITMQ_URI is the uri, in the format of `amqp://<username>:<password>@<host>:<port>/<vhost>`, and $NAMESPACE is the namespace where the Event Aggregation Service will be deployed.

# Deployment
The chart does not render with its default values: it needs an authentication
configuration.

## Authentication
Either configure OpenID, so the service verifies bearer tokens itself, or set
`principalSource` explicitly:

- **OpenID (`jwt`).** Leave `principalSource` unset and set
  `openId.openIdConfigurationUri` (the identity provider's
  `/.well-known/openid-configuration` URL) plus at least one JWT audience: a
  `default` entry in `openId.clientRegistrations`, whose `clientId` is the
  audience, or `openId.additionalJwtAudience` (a list). The chart then selects
  `jwt`. The pods need egress to the identity provider's discovery and JWKS
  endpoints.
- **An explicit `principalSource`.** `disabled` turns authentication off, for
  a test install without tokens; it cannot be combined with
  `permissions.enabled`. `opaque` and `userInfoService` have their own
  required values.
- **`jwtUnverified`** reads the publisher from the token without verifying
  it. It renders only with both `principalSource: jwtUnverified` and
  `allowUnverifiedJwt: true`, and is safe only when every path to the pods
  verifies tokens first: the HTTPProxy's auth extension covers only requests
  through the HTTPProxy, so the Service must also be unreachable directly
  (for example through a NetworkPolicy).

The chart fails the render, with a message naming the value to set, for every
authentication configuration the service would refuse at startup.

## CORS
`rest.corsAllowOrigins` lists the browser origins the service's own CORS
middleware admits, with credentials allowed; the default `[]` adds no CORS.
Write each origin exactly as a browser sends it, for example
`https://app.example.com`. `*` and `null` fail the render.

See `principalSource`, `allowUnverifiedJwt`, `openId`, and
`rest.corsAllowOrigins` in `values.yaml` for the details.

## Install
From the repository root, with OpenID:
```
helm install event-aggregation ./helm/event-aggregation-service -n $NAMESPACE \
  --set openId.openIdConfigurationUri=$OPENID_CONFIGURATION_URI \
  --set 'openId.clientRegistrations[0].name=default' \
  --set "openId.clientRegistrations[0].clientId=$CLIENT_ID"
```
Where $OPENID_CONFIGURATION_URI is the identity provider's
`/.well-known/openid-configuration` URL and $CLIENT_ID is the JWT audience. Or,
for a test install that accepts requests without a token:
```
helm install event-aggregation ./helm/event-aggregation-service -n $NAMESPACE \
  --set principalSource=disabled
```
