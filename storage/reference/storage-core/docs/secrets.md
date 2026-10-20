# Secrets


## Secrets Format

The secrets for both S3 and Azure backends are a collection of key value pairs, where the key of each value is the authority (or endpoint: see [S3 Storage Service Endpoint URLs](#s3-storage-service-endpoint-urls)) that the secrets will apply to, and the value is an object specifying the secrets i.e.:

```json
 "my-bucket.s3.us-west-1.amazonaws.com": {
    "AWS_ACCESS_KEY_ID": "...",
    "AWS_SECRET_ACCESS_KEY": "...",
  }
```

In this example, any requests to url with `my-bucket.s3.us-west-1.amazonaws.com` as the authority would be signed with the provided access key.

If the storage service receives a request and does not have a secrets value for the authority of the requested URL, it will assume that the URL is publicly accessible and attempt to access the URL without credentials.

All configuration options are described below.

## Kubernetes Deployment

The storage service reads secrets from `/var/secrets/secrets.json`. In Kubernetes, create a Secret containing your credentials and mount it into the pod.

### Step 1: Create the Kubernetes Secret

```bash
kubectl create secret generic storage-service-creds \
  --from-file=secrets.json=/path/to/your/secrets.json \
  -n <namespace>
```

Or from a literal JSON string:

```bash
kubectl create secret generic storage-service-creds -n <namespace> \
  --from-literal=secrets.json='{
  "storage-service-s3": {
    "storage.cloud.google.com/my-bucket": {
      "AWS_ACCESS_KEY_ID": "GOOG...",
      "AWS_SECRET_ACCESS_KEY": "...",
      "AWS_ENDPOINT_URL": "https://storage.googleapis.com"
    }
  }
}'
```

### Step 2: Mount via Helm values

The storage-service Helm chart supports `volumes` and `volumeMounts` values for injecting arbitrary volumes. Use these to mount the Secret:

```yaml
# In your Helm values or overrides file:
storage-service:
  volumes:
    - name: storage-creds
      secret:
        secretName: storage-service-creds
  volumeMounts:
    - name: storage-creds
      mountPath: /var/secrets
      readOnly: true
```

## S3
All secrets for the S3 backend are stored under the `storage-service-s3` key.

Secrets options available for S3 backend:
* `AWS_REGION`:
  + The AWS region the bucket is located in. This value is optional when using a fully qualified S3 domain, e.g. `<bucket>.s3.<region>.amazonaws.com`. If a custom domain is being used, this value must be provided.
* `AWS_BUCKET`:
  + The AWS bucket.  This value is optional when using a fully qualified S3 domain, e.g. `<bucket>.s3.<region>.amazonaws.com`. If a custom domain is being used, this value must be provided.
* `AWS_ACCESS_KEY_ID`:
  + Specifies an AWS access key associated with an IAM account. This value is optional. If it is not specified, requests to the bucket will not be signed.
* `AWS_SECRET_ACCESS_KEY`:
  + Specifies the secret key associated with the access key. This is essentially the "password" for the access key. This value is optional. If it is not specified, requests to the bucket will not be signed.
* `AWS_SESSION_TOKEN`:
  + Specifies the session token value that is required if you are using temporary security credentials that you retrieved directly from AWS STS operations. This value is optional.
* `AWS_ASSUMED_ROLE_ARN`:
  + The ARN of the IAM role to assume. Optional. When provided along with access key credentials, the storage service will use AWS STS to assume this role and use the resulting temporary credentials.
* `AWS_ASSUMED_ROLE_SESSION_NAME`:
  + The session name for the assumed role session. Optional (only relevant when `AWS_ASSUMED_ROLE_ARN` is set).
* `AWS_ASSUMED_ROLE_SESSION_LENGTH`:
  + Duration of the assumed role session (e.g. `2h`). Optional (only relevant when `AWS_ASSUMED_ROLE_ARN` is set).
* `AWS_EXTERNAL_ID`:
  + External ID for the assume role request (used for cross-account access). Optional (only relevant when `AWS_ASSUMED_ROLE_ARN` is set).
* `AWS_ENDPOINT_URL`:
  + See [ENDPOINTS.md](ENDPOINTS.md#configuration-settings) for details.
* `AWS_ADDRESSING_STYLE`:
  + See [ENDPOINTS.md](ENDPOINTS.md#configuration-settings) for details.
* `CLOUDFRONT`:
  + Optional object to serve reads through an Amazon CloudFront distribution. When set, read requests return signed or presigned URLs that point at the distribution instead of S3, so clients are served from the CDN. All sub-fields are optional except `DISTRIBUTION_URL` when `CLOUDFRONT` is present.
  + `DISTRIBUTION_URL`: The CloudFront distribution URL (e.g. `https://d6x43ndjxh9bo.cloudfront.net`). Required when `CLOUDFRONT` is specified.
  + `KEY_PAIR_ID`: The CloudFront public key pair ID used for signing URLs. When provided together with `PRIVATE_KEY_PEM`, read URLs are signed with a canned policy (Expires, Signature, Key-Pair-Id).
  + `PRIVATE_KEY_PEM`: The PEM-encoded private key used to sign CloudFront URLs. Must be provided with `KEY_PAIR_ID` for signed URLs; if only `DISTRIBUTION_URL` is set, URLs are not signed.
* `VERSIONING_ENABLED`:
  + Whether object versioning is enabled on this endpoint's bucket. `false` makes `EnumerateVersions` for this endpoint fail fast with `FAILED_PRECONDITION` (HTTP 412) instead of a confusing backend error. Overrides the backend-wide `s3.versioning_enabled` config; when unset, the backend-wide value (or enabled) applies. There is no runtime detection.
  + Resolution order matches `READ_ONLY`: this per-endpoint entry, then backend-wide `s3.versioning_enabled`, then the `VERSIONING_ENABLED` **environment variable**, then enabled. As with `READ_ONLY`, the environment variable applies only to an endpoint absent from the secrets map when `allow_unconfigured_endpoints` is true, and this fallback is S3-only.
  + In strict mode (`allow_unconfigured_endpoints = false`), `IsVersioned` for an endpoint with no secrets entry fails with `NOT_FOUND` (`endpoint not configured`), as `IsReadOnly` does, instead of reporting a state for an address no operation can use. A backend-wide `s3.versioning_enabled` is an answer for every endpoint, so when it is set `IsVersioned` reports it instead of rejecting.
* `READ_ONLY`:
  + Whether this endpoint is read-only. `IsReadOnly` reports it; **nothing enforces it** — a write to a `READ_ONLY` endpoint is not rejected on the strength of this setting. It is a declaration for clients that want to know before they try. Overrides the backend-wide `s3.read_only` config; when unset, the backend-wide value (or writable) applies. There is no runtime detection.
  + Resolution order: this per-endpoint entry, then backend-wide `s3.read_only`, then the `READ_ONLY` **environment variable**, then writable. The environment variable is consulted **only** for an endpoint that has no secrets entry at all and only when `allow_unconfigured_endpoints` is true — that is the same condition under which the endpoint's client is built from environment credentials. An endpoint that has a secrets entry but omits `READ_ONLY` does **not** inherit the process-wide value.
  + **This environment fallback is S3-only.** Azure has no environment-credential path, so `READ_ONLY` in the environment does nothing for Azure addresses. In a mixed S3+Azure deployment, setting `READ_ONLY=true` makes unconfigured S3 endpoints read-only and leaves Azure endpoints writable. Set `azure.read_only` or a per-endpoint entry to cover Azure.

When `CLOUDFRONT` is configured, the storage service will always return redirect URLs for read requests (regardless of client `download_preference`), so that clients are served from the CDN.

More details about these environment variables can be found at https://docs.aws.amazon.com/cli/v1/userguide/cli-configure-envvars.html 

### Possible Configurations:
1. Standard domain, public access: 
    + No configuration needed. The storage service will parse the region and bucket from the URL and issue the request with no credentials.

2. Standard domain, private access: 
    + Here, the key must be specified

        ```json
        "my-bucket.s3.us-west-1.amazonaws.com": {
            "AWS_ACCESS_KEY_ID": "...",
            "AWS_SECRET_ACCESS_KEY": "..."
        }
        ```

3. Custom domain, public access
    + Here, the region and bucket must be specified

        ```json
        "my-content.example.com": {
            "AWS_REGION": "us-west-1",
            "AWS_BUCKET": "my-bucket"
        }
        ```

4. Custom domain, private access
    + Here, all values must be specified

        ```json
        "my-content.example.com": {
            "AWS_REGION": "us-west-1",
            "AWS_BUCKET": "my-bucket",
            "AWS_ACCESS_KEY_ID": "...",
            "AWS_SECRET_ACCESS_KEY": "...",
        }
        ```

5. With CloudFront (CDN), unsigned or presigned
    + Read requests are served via the CloudFront distribution. Only the distribution URL is required:

        ```json
        "my-bucket.s3.us-west-1.amazonaws.com": {
            "AWS_ACCESS_KEY_ID": "...",
            "AWS_SECRET_ACCESS_KEY": "...",
            "CLOUDFRONT": {
                "DISTRIBUTION_URL": "https://d6x43ndjxh9bo.cloudfront.net"
            }
        }
        ```

6. With CloudFront (CDN), signed URLs
    + To use CloudFront signed URLs (canned policy with Expires, Signature, Key-Pair-Id), add the key pair ID and private key:

        ```json
        "my-bucket.s3.us-west-1.amazonaws.com": {
            "AWS_ACCESS_KEY_ID": "...",
            "AWS_SECRET_ACCESS_KEY": "...",
            "CLOUDFRONT": {
                "DISTRIBUTION_URL": "https://d6x43ndjxh9bo.cloudfront.net",
                "KEY_PAIR_ID": "K2ABCDEFGHIJKL",
                "PRIVATE_KEY_PEM": "-----BEGIN RSA PRIVATE KEY-----\n...\n-----END RSA PRIVATE KEY-----"
            }
        }
        ```

7. With Assume Role
    + Use AWS STS to assume an IAM role. The storage service will use the provided access key credentials to call `AssumeRole` and use the resulting temporary credentials for S3 requests:

        ```json
        "my-bucket.s3.us-west-1.amazonaws.com": {
            "AWS_ACCESS_KEY_ID": "...",
            "AWS_SECRET_ACCESS_KEY": "...",
            "AWS_ASSUMED_ROLE_ARN": "arn:aws:iam::123456789012:role/my-role",
            "AWS_ASSUMED_ROLE_SESSION_NAME": "my-session",
            "AWS_ASSUMED_ROLE_SESSION_LENGTH": "2h",
            "AWS_EXTERNAL_ID": "my-external-id"
        }
        ```

These could all be combined into one large secrets configuration:

```json
{
  "storage-service-s3": {
    "my-content.example.com": {
      "AWS_REGION": "us-west-1",
      "AWS_BUCKET": "my-bucket",
      "AWS_ACCESS_KEY_ID": "...",
      "AWS_SECRET_ACCESS_KEY": "...",
      "CLOUDFRONT": {
        "DISTRIBUTION_URL": "https://d6x43ndjxh9bo.cloudfront.net",
        "KEY_PAIR_ID": "K2ABCDEFGHIJKL",
        "PRIVATE_KEY_PEM": "-----BEGIN RSA PRIVATE KEY-----\n...\n-----END RSA PRIVATE KEY-----"
      }
    },
    "my-bucket-2.s3.us-west-2.amazonaws.com": {
      "AWS_ACCESS_KEY_ID": "...",
      "AWS_SECRET_ACCESS_KEY": "..."
    },
    "my-bucket-3.s3.eu-north-1.amazonaws.com": {
      "AWS_ACCESS_KEY_ID": "...",
      "AWS_SECRET_ACCESS_KEY": "..."
    },
    "my-public-assets.example.com": {
      "AWS_REGION": "us-east-1",
      "AWS_BUCKET": "public-assets-bucket"
    },
    "my-bucket-4.s3.us-west-2.amazonaws.com": {
      "AWS_ACCESS_KEY_ID": "...",
      "AWS_SECRET_ACCESS_KEY": "...",
      "AWS_ASSUMED_ROLE_ARN": "arn:aws:iam::123456789012:role/my-role",
      "AWS_ASSUMED_ROLE_SESSION_NAME": "my-session",
      "AWS_ASSUMED_ROLE_SESSION_LENGTH": "2h",
      "AWS_EXTERNAL_ID": "my-external-id"
    }
  }
}
```

### Azure
All secrets for the Azure backend are stored under the `storage-service-azure` key.

Secrets options available for Azure backend:

**General:**
* `AZURE_STORAGE_ACCOUNT`
    + The account name. This value is optional, if it is not specified, the name will be parsed from the authority. If a custom domain is being used, this value must be provided.

**Access key authentication:**
* `AZURE_STORAGE_KEY`
    + The access key of the account. This value is optional. If it is not specified, requests to this account will not be signed (unless service principal credentials are provided). Presigned URLs use account-key SAS tokens.

**[Service principal authentication](https://learn.microsoft.com/en-us/azure/storage/blobs/authorize-access-azure-active-directory)** (all three required):

When all three fields are present, the storage service authenticates using the [OAuth2 client credentials flow](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-client-creds-grant-flow) and generates [User Delegation SAS](https://learn.microsoft.com/en-us/rest/api/storageservices/create-user-delegation-sas) tokens for presigned URLs instead of account-key SAS tokens.

The service principal must be assigned the **[Storage Blob Data Contributor](https://learn.microsoft.com/en-us/azure/role-based-access-control/built-in-roles/storage#storage-blob-data-contributor)** [Azure RBAC role](https://learn.microsoft.com/en-us/azure/storage/blobs/assign-azure-role-data-access) on each storage account it accesses. This role grants read, write, and delete permissions for blob data, as well as the ability to obtain a [user delegation key](https://learn.microsoft.com/en-us/rest/api/storageservices/get-user-delegation-key) for signing User Delegation SAS tokens.

* `AZURE_TENANT_ID`
    + The Azure AD tenant ID.
* `AZURE_CLIENT_ID`
    + The Azure AD client (application) ID.
* `AZURE_CLIENT_SECRET`
    + The Azure AD client secret.
* `AZURE_AUTHORITY_HOST`
    + The Azure AD authority host URL. Defaults to `https://login.microsoftonline.com`. Override this for sovereign clouds (e.g. `https://login.microsoftonline.us` for Azure Government). Optional.

**CDN / Front Door:**
* `AZURE_FRONT_DOOR_URL`
    + The Front Door URL for presigned URLs. When configured, the storage service will issue presigned URLs through Front Door instead of directly to the storage account. This value is optional. It can be specified as:
        - A hostname only: `frontdoor.azurefd.net` (https scheme is assumed)
        - A full URL with optional path prefix: `https://frontdoor.azurefd.net/storage-prefix`
    + When a path prefix is specified, it will be prepended to the blob path in presigned URLs. The Front Door or proxy must be configured to strip this prefix before forwarding requests to storage.

**Versioning:**
* `VERSIONING_ENABLED`
    + Whether object versioning is enabled on this endpoint's account. `false` makes `EnumerateVersions` for this endpoint fail fast with `FAILED_PRECONDITION` (HTTP 412) instead of a confusing backend error. Overrides the backend-wide `azure.versioning_enabled` config; when unset, the backend-wide value (or enabled) applies. There is no runtime detection.
    + In strict mode (`allow_unconfigured_endpoints = false`), `IsVersioned` for an endpoint with no secrets entry fails with `NOT_FOUND` (`endpoint not configured`), as `IsReadOnly` does, instead of reporting a state for an address no operation can use. A backend-wide `azure.versioning_enabled` is an answer for every endpoint, so when it is set `IsVersioned` reports it instead of rejecting.
* `READ_ONLY`:
    + Whether this endpoint is read-only. `IsReadOnly` reports it; **nothing enforces it** — writes are not rejected on the strength of this setting. Overrides the backend-wide `azure.read_only` config; when unset, the backend-wide value (or writable) applies. There is no runtime detection.
    + Unlike S3, there is **no environment-variable fallback** for Azure: `READ_ONLY` in the process environment is ignored for Azure addresses, because Azure has no environment-credential path. Use this per-endpoint entry or `azure.read_only`.

#### Authentication Priority

When both service principal credentials and an access key are configured for the same account, service principal authentication takes precedence and the access key is ignored (a warning is logged). If only some of the three service principal fields are provided, a warning is logged and the storage service falls back to access key or anonymous authentication.

#### Possible Configurations:
1. Standard domain, public access:
    + No configuration needed. The storage service will parse the region and bucket from the URL and issue the request with no credentials.

2. Standard domain, private access (access key):
    + Here, the key must be specified

       ```json
        "ovdevinfracoreapi.blob.core.windows.net": {
            "AZURE_STORAGE_KEY": "..."
        }
        ```

3. Standard domain, private access (service principal):
    + Uses Azure AD service principal authentication with User Delegation SAS for presigned URLs.

        ```json
        "ovdevinfracoreapi.blob.core.windows.net": {
            "AZURE_TENANT_ID": "...",
            "AZURE_CLIENT_ID": "...",
            "AZURE_CLIENT_SECRET": "..."
        }
        ```

4. Custom domain, public access
    + Here, the account must be specified

        ```json
        "ov-content-azure.nvidia.com": {
            "AZURE_STORAGE_ACCOUNT": "mystorageaccount"
        }
        ```

5. Custom domain, private access
    + Here, all values must be specified

        ```json
        "ov-content-azure.nvidia.com": {
            "AZURE_STORAGE_ACCOUNT": "mystorageaccount",
            "AZURE_STORAGE_KEY": "..."
        }
        ```

6. With Front Door (CDN)
    + Front Door can be configured with any of the above configurations. The presigned URLs will be issued through Front Door instead of directly to the storage account.

        ```json
        "ovdevinfracoreapi.blob.core.windows.net": {
            "AZURE_STORAGE_KEY": "...",
            "AZURE_FRONT_DOOR_URL": "fd-ovdevinfra-abc123.azurefd.net"
        }
        ```

    + With a path prefix (useful when Front Door serves multiple storage accounts):

        ```json
        "ovdevinfracoreapi.blob.core.windows.net": {
            "AZURE_STORAGE_KEY": "...",
            "AZURE_FRONT_DOOR_URL": "https://cdn.example.com/storage-prefix"
        }
        ```

7. Service principal with Front Door
    + Service principal auth can also be combined with Front Door:

        ```json
        "ovdevinfracoreapi.blob.core.windows.net": {
            "AZURE_TENANT_ID": "...",
            "AZURE_CLIENT_ID": "...",
            "AZURE_CLIENT_SECRET": "...",
            "AZURE_FRONT_DOOR_URL": "fd-ovdevinfra-abc123.azurefd.net"
        }
        ```

These could all be combined into one large secrets configuration:

```json
{
  "storage-service-azure": {
    "ov-content-azure.nvidia.com": {
      "AZURE_STORAGE_ACCOUNT": "mystorageaccount",
      "AZURE_STORAGE_KEY": "..."
    },
    "ovdevinfracoreapi.blob.core.windows.net": {
      "AZURE_TENANT_ID": "...",
      "AZURE_CLIENT_ID": "...",
      "AZURE_CLIENT_SECRET": "...",
      "AZURE_FRONT_DOOR_URL": "fd-ovdevinfra-abc123.azurefd.net"
    },
    "ovdevinfracoreapiswe.blob.core.windows.net": {
      "AZURE_STORAGE_KEY": "..."
    }
  }
}
```

## S3 Storage Service Endpoint URLs

The S3 Storage Service can be configured to use different endpoint URLs to access different S3-compatible storage backends.

### Configuration Settings

* `AWS_ENDPOINT_URL`:
  + The endpoint URL to send requests to. This value must be provided to use a custom endpoint URL. More details about the endpoint URL can be found at https://docs.aws.amazon.com/cli/v1/userguide/cli-configure-endpoints.html.
* `AWS_ADDRESSING_STYLE`:
  + The addressing style to use when sending requests to the endpoint URL. Valid values are `path` or `virtual`. `path` style URLs take the form of `http://<endpoint>/<bucket>/<key>`. `virtual` style URLs take the form of `http://<bucket>.<endpoint>/<key>`. The default value is `virtual`. This value will be inferred from the configuration style if not specified. More details about the addressing style can be found at https://docs.aws.amazon.com/cli/v1/userguide/cli-configure-files.html.
* `AWS_REGION`:
  + The region to use for this bucket. 
* `AWS_BUCKET`:
  + The bucket to use for the endpoint URL. This value will be inferred from the configuration if not specified.

All other options from the [S3 Storage Service](README.md#s3) are also available.


### Path and Virtual Addressing Style Configuration

The Storage Services allows the configuration keys to either be `virtual` style where the bucket name is the host, or `path` style where the bucket name is a path segment. Additionally, the scheme can be set in the configuration key, and if not set, will default to `https`. For example, the following configuration would be used to configure multiple buckets on the same local endpoint:

```json
// Virtual style, https scheme assumed, bucket and region parsed from hostname
"example-bucket.s3.us-west-2.amazonaws.com": {
    "AWS_ACCESS_KEY_ID": "access-key-id",
    "AWS_SECRET_ACCESS_KEY": "secret-access-key",
},
// Path style, bucket parsed as the first path segment
"http://127.0.0.1:9000/example-bucket": {
    "AWS_ACCESS_KEY_ID": "access-key-id",
    "AWS_SECRET_ACCESS_KEY": "secret-access-key",
    "AWS_ENDPOINT_URL": "http://127.0.0.1:9000",
    "AWS_REGION": "test-region"
},
 ```

By default when the Storage Service encounters a configuration with a bucket name in the key, it will take the bucket name from configuration key and assume the addressing style is `path`. This can be overridden by specifying the `AWS_BUCKET` and `AWS_ADDRESSING_STYLE` settings.

When a path style configuration is detected, the `AWS_REGION` field is optional. If the `AWS_REGION` field is not provided, the Storage Service will use the region `us-east-1`.

When the Storage Service receives a request URL such as `http://127.0.0.1:9000/example-bucket/unittests/stat/sub-folder/splash.jpg`, it will first look up the authority, `127.0.0.1:9000`, in the configuration map. It will then see that the configuration value is a map of bucket names, so it will look at the first path segment, `example-bucket`, and use that to look up the configuration for that bucket. If it finds a match it will use the configuration for that bucket to process the request.

### Tested Backends

#### MinIO

The Storage Service can be configured to use a local MinIO instance for testing purposes. The following configuration would be used to configure the Storage Service to use a local MinIO instance:

```json
{
  "storage-service-s3": {
    "http://127.0.0.1:9000/example-bucket": {
      "AWS_ACCESS_KEY_ID": "access-key-id",
      "AWS_SECRET_ACCESS_KEY": "secret-access-key",
      "AWS_ENDPOINT_URL": "http://127.0.0.1:9000",
      "AWS_REGION": "test-region"
    },
    "http://127.0.0.1:9000/example-bucket-public": {
      "AWS_ENDPOINT_URL": "http://127.0.0.1:9000",
      "AWS_REGION": "test-region"
    },
    "http://127.0.0.1:9000/example-bucket-versioned": {
      "AWS_ACCESS_KEY_ID": "access-key-id",
      "AWS_SECRET_ACCESS_KEY": "secret-access-key",
      "AWS_ENDPOINT_URL": "http://127.0.0.1:9000",
      "AWS_REGION": "test-region"
    }
  }
}
```

This configuration would allow you to access the `example-bucket` and `example-bucket-versioned` buckets using the MinIO API. Any requests that come to this Storage Service with a url starting with `http://127.0.0.1:9000/example-bucket/`, `http://127.0.0.1:9000/example-bucket-public/`, or `http://127.0.0.1:9000/example-bucket-versioned/`, will be routed to the MinIO API. For instance, the following URLs would be routed to the MinIO server:

```
http://127.0.0.1:9000/example-bucket/unittests/stat/sub-folder/splash.jpg
http://127.0.0.1:9000/example-bucket-public/unittests/stat/sub-folder/splash.jpg
http://127.0.0.1:9000/example-bucket-versioned/unittests/stat/sub-folder/splash.jpg
```

Additionally, the `top_level_address` function will return `http://127.0.0.1:9000/example-bucket/`, `http://127.0.0.1:9000/example-bucket-public/`, and `http://127.0.0.1:9000/example-bucket-versioned/` as top level addresses for this Storage Service.

#### Google Cloud Storage

Google Cloud Storage has an [S3-compatible API](https://cloud.google.com/storage/docs/interoperability). To use the Storage Service with Google Cloud Storage, the `AWS_ENDPOINT_URL` should be set to the endpoint URL for the Google Cloud Storage API. The `AWS_REGION` can be set to any value. If your bucket is private, you will need to generate an [HMAC Key](https://cloud.google.com/storage/docs/authentication/hmackeys) to provide to the S3 Storage Service.

The configuration for the Storage Service to use Google Cloud Storage is as follows:

```json
{
  "storage-service-s3": {
    "storage.cloud.google.com/example-bucket": {
      "AWS_ACCESS_KEY_ID": "HMAC Key ID",
      "AWS_SECRET_ACCESS_KEY": "HMAC Secret Key",
      "AWS_ENDPOINT_URL": "https://storage.googleapis.com"
    },
    "storage.cloud.google.com/example-bucket-versioned": {
      "AWS_ACCESS_KEY_ID": "HMAC Key ID",
      "AWS_SECRET_ACCESS_KEY": "HMAC Secret Key",
      "AWS_ENDPOINT_URL": "https://storage.googleapis.com"
    }
  }
}
```

This configuration would allow you to access the `example-bucket` and `example-bucket-versioned` buckets using the Google Cloud Storage API. Any requests that come to this Storage Service with a url starting with `https://storage.cloud.google.com/example-bucket/` or `https://storage.cloud.google.com/example-bucket-versioned/`, will be routed to the Google Cloud Storage API. For instance, the following URLs would be routed to the Google Cloud Storage API:

```
https://storage.cloud.google.com/example-bucket/unittests/stat/sub-folder/splash.jpg
https://storage.cloud.google.com/example-bucket-versioned/unittests/stat/sub-folder/splash.jpg
```

Additionally, the `top_level_address` function will return `https://storage.cloud.google.com/example-bucket/` and `https://storage.cloud.google.com/example-bucket-versioned/` as top level addresses for this Storage Service.

When using the Storage Service with Google Cloud Storage, there are some limitations to be aware of:

* You will not be able access specific object versions in a versioned bucket. You will only be able to access the latest version of the object. The Google Cloud Storage API does not support the standard S3 `VersionId` and `x-amz-version-id` headers needed to access a specific version of an object (https://cloud.google.com/storage/docs/migrating#custommeta).

* You will not be able to perform conditional writes. If you provide a previous version object as part of the write request, Google Cloud Storage will not honor the condition and will overwrite the object.


