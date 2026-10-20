# Component map: Omniverse Client Library (Kit integration)

> **Applies to:** Omniverse Client Library (`omni.client`, `OmniClient.h`) used by Kit and by standalone tools to reach this stack · **Released:** the Developer Guide 1.3.0 docs manifest records Client Library **2.72**; examples here target **2.74** · **This repo's snapshot:** not present; the Client Library is a separate NVIDIA product · **Sources:** upstream `omni-client-library` skill (`SKILL.md`, `references/configuration.md`, `python-api.md`, `c-api.md`, `runtime-behavior.md`) at the revision pinned by the [source skill package](../../NOTICE.md), with that package's 2.74 and authentication adaptations, and its Kit and discovery deployment notes; [2.72 documentation](https://docs.omniverse.nvidia.com/kit/docs/client_library/2.72.0/index.html)

Scope: runtime use of the library against this stack. Building the library is out of scope. For new standalone (Kit-free) integrations, also consider [ovstorage](https://github.com/NVIDIA-Omniverse/ovstorage), whose `services-client` plugin reaches the Storage Service ([use/usd-workflows.md](../use/usd-workflows.md)).

## Versions: 2.72 baseline, 2.74 examples

- 2.72 is the release provenance in the Developer Guide 1.3.0 manifest. Its documentation is the published reference above.
- The examples on this page use 2.74 behavior: `StorageRegistration` objects, safe `join()`, Discovery v2, and environment discovery that never launches browser sign-in.
- Check the loaded version in the actual process (`omni.client.get_version()`) before using 2.74 examples. The source package observed Kit `110.4.0+feature.379084.fb2ab997.gl` loading Client `2.74.0-release.7316+gl.ec99a64b`, with native file operations and localhost registration passing scoped checks. Authenticated use against a deployed stack and compatibility with the 1.3.0 inventory were not verified, and no other Kit build is certified.
- The guide's compatibility table covers Kit 109.x, 110.1 and 110.2 with usd-content-workspace releases 1.0–1.2 only ([operate/known-issues.md](../operate/known-issues.md#clients-and-compatibility)).
- A public 2.74 documentation endpoint is unverified; do not apply 2.72's older registration return types to 2.74 code.

## Connect Kit to the stack

Kit SDK 109.0.1 or later. Set `OMNI_STORAGE_DISCOVERY` to the Discovery **base URL**, without `/api/v1/services` or `/api/v2/services`, before Kit starts, for example in the launch script before its `exec` line:

```bash
export OMNI_STORAGE_DISCOVERY=https://<discovery-host>   # HTTPS deployments
# export OMNI_STORAGE_DISCOVERY=http://localhost:8080    # local development only
```

Then in Kit: **Add New Connection**, enter a storage URL served by the deployment, browse in the Content Browser, and create, open, edit and save a file to verify read and write access.

What makes this work on the server side:

- Discovery lists the Storage Service ([deploy/discovery.md](../deploy/discovery.md)). The snapshot operator serves `/api/v2/services` and, through `config.v1Endpoints`, v1 entries for older clients.
- With `openId.enabled: true`, Discovery serves `/api/v1/auth-config`, which must be reachable without a token. Its response has a top-level `openid_configuration` URL and a `clients` object keyed by client type, each entry with `client_id` and `scope`; the Client Library entry is keyed `client_library`. These are response fields, not Helm values ([deploy/permissions-and-auth.md](../deploy/permissions-and-auth.md)).
- For device flow, the IdP's OpenID configuration must include `device_authorization_endpoint`.

## Authentication against the stack

| Registration path | Behavior in 2.74 |
| --- | --- |
| Environment discovery (`OMNI_STORAGE_DISCOVERY`) | Never launches browser sign-in. Uses a registered storage auth callback if present; device flow if a device-flow callback is registered or `OMNICLIENT_NO_BROWSER=1`; otherwise fails with `ERROR_CONNECTION` |
| Explicit API registration (`register_storage_from_discovery`, `register_storage_direct`) | May use browser sign-in when no non-interactive credential exists; device-flow selection still applies |
| Manual bearer (`OMNI_STORAGE_AUTHORIZATION`) | Separate path; scope the process to one deployment |

`OMNICLIENT_NO_BROWSER=1` selects device flow; it still needs a person. There is no variable that disables interactive sign-in. For unattended processes, register an origin-scoped token callback before first access.

Rules for any credential callback:

- Compare the exact origin (scheme, lowercase host, port with HTTP/HTTPS defaults filled in). Never match on a hostname substring or suffix.
- Return nothing for unknown origins.
- Get tokens from the application's credential provider for that origin's audience; never hard-code tokens or passwords, and avoid `OMNI_USER`/`OMNI_PASS` outside throwaway tests.
- Keep the `Registration` object alive for the task; on token failure cancel the attempt instead of reporting success.

```python
import omni.client
from urllib.parse import urlsplit


def credential_origin(value):
    if not isinstance(value, str) or not value:
        return None
    if any(ord(c) < 33 or ord(c) == 127 for c in value) or "\\" in value:
        return None
    try:
        parsed = urlsplit(value)
        if (not parsed.scheme or not parsed.hostname or parsed.username is not None
                or parsed.password is not None or "%" in parsed.netloc
                or parsed.netloc.endswith(":")):
            return None
        port = parsed.port
    except ValueError:
        return None
    if port is None:
        port = {"https": 443, "http": 80}.get(parsed.scheme.lower())
    return parsed.scheme.lower(), parsed.hostname.lower(), port


STORAGE_ORIGIN = credential_origin("https://storage.example.com")


def storage_auth(auth_handle: int, server: str) -> bool:
    if STORAGE_ORIGIN is None or credential_origin(server) != STORAGE_ORIGIN:
        return False                      # not ours; other methods may run
    try:
        token = get_storage_token()       # application provider scoped to STORAGE_ORIGIN
    except Exception:
        omni.client.authentication_cancel(auth_handle)
        return True                       # handled by cancelling
    return omni.client.set_storage_access_token(auth_handle, token)


storage_auth_registration = omni.client.register_storage_auth_callback(storage_auth)
```

`auth_handle` (the integer passed to the callback) is not the `Registration` returned by `register_storage_auth_callback`, nor a `StorageRegistration`.

## Register storage and read (2.74)

Blocking and async registration return `StorageRegistration | None`. Check for `None` and a non-OK `.result`; hold the object until dependent work and subscriptions finish (`.stop()` or leave the context). After it stops, URLs fall back to other providers (`omniverse://` to Nucleus).

```python
def read_from_storage(discovery_url, file_url):
    registration = omni.client.register_storage_from_discovery(discovery_url)
    if registration is None:
        raise RuntimeError("Storage registration did not start")
    with registration:
        if registration.result != omni.client.Result.OK:
            raise RuntimeError(f"Storage registration failed: {registration.result}")
        result, version, content = omni.client.read_file(file_url)
        if result != omni.client.Result.OK:
            raise RuntimeError(f"Storage read failed: {result}")
        return memoryview(content).tobytes()
```

In Kit, `await omni.client.register_storage_from_discovery_async(...)` and `read_file_async` on the existing event loop; never `asyncio.run()` and never blocking calls on the UI loop. Inside Kit the host owns `initialize()`/`shutdown()`; an extension must not shut the shared client down. `_with_callback` variants return a `Request` that must be kept and stopped explicitly.

## Routing and configuration

The storage provider registers with scheme `*` and gets the first chance at any URL it recognizes from discovery or direct registration; then `omniverse://` goes to Nucleus, `http(s)://` to HTTP (S3 for `*.amazonaws.com` and configured hosts, Azure for `*.blob.core.windows.net`), and `file://` or bare paths to the file provider.

| Variable | Meaning | Default |
| --- | --- | --- |
| `OMNI_STORAGE_DISCOVERY` | Discovery base URL | unset |
| `OMNI_STORAGE_CLIENT_NAME` | Client name for the OIDC configuration (the default matches the `client_library` key of the auth-config `clients` object) | `client_library` |
| `OMNI_STORAGE_HOST` / `OMNI_STORAGE_PORT` / `OMNI_STORAGE_SECURE` | Direct storage endpoint without discovery | none / `50051` / TLS when port 443 |
| `OMNI_STORAGE_AUTHORIZATION` | Static `Authorization` header (avoid in shared environments) | unset |
| `OMNI_STORAGE_DEADLINE` | Operation deadline, seconds (`0` = none) | `0` |
| `OMNI_STORAGE_RETRY_MAX`, `_RETRY_BASE_MS`, `_RETRY_MAX_DELAY_MS`, `_RETRY_FORMULA`, `_RETRY_JITTER` | Retry policy | `10`, `10`, `5000`, `2` (exponential), `2` (equal) |
| `OMNICLIENT_PROVIDERS` | Providers to register (`file`, `http`, `nucleus`, `storage`) | all |
| `OMNICLIENT_NO_BROWSER` | Prefer device flow | unset |
| `OMNICLIENT_HTTP_TIMEOUT`, `_HTTP_RETRIES`, `_HTTP_CONCURRENT_OPS` | HTTP providers | `60`, `8`, `20` |
| `OMNICLIENT_HUB_MODE`, `_HUB_EXE`, `_HUB_CACHE_DIR` | Hub cache process | auto-detect |
| `OMNI_CONFIG_PATH` | Directory holding `omniverse.toml` | platform default |

These defaults come from the upstream skill; confirm them for the loaded version. `omniverse.toml` can also hold aliases and S3/Azure settings; keep secrets out of it where a secret store is available. Hub: newer Hub works with older clients, not the reverse; caching products are in [use/companion-products.md](../use/companion-products.md).

## Live collaboration

The library has **live sessions** (create, list and join on a base layer URL; presence and messages) and Nucleus-only **live objects** (`live_process()` from the main thread). Whether a given 2.7x build speaks LiveService 0.1.1 for live-layer editing against this stack is unverified in this snapshot. ovlive is the Live Edit client documented for this stack, and its `.live` layers are separate from Nucleus live layers served by `omni.client` ([apis/live-edit.md](../apis/live-edit.md#clients)).

## USD files

The library moves bytes. Parsing USD needs `usd-core` (`pxr`), and resolving `omniverse://` paths inside USD needs the NVIDIA USD Resolver plugin, which uses the Client Library underneath. `.usda` can be inspected as text; `.usd`/`.usdc` need `pxr`. See [use/usd-workflows.md](../use/usd-workflows.md).

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Registration fails with `ERROR_CONNECTION` from environment discovery | No usable token callback or device-flow path; register an origin-scoped callback before first access |
| Connection added but browse fails | `OMNI_STORAGE_DISCOVERY` includes `/api/v*/services`; Discovery or auth-config not reachable from the workstation; advertised URLs internal-only (`visibility`) |
| 2.74 example raises on tuple unpacking | Loaded client is older than 2.74; check `get_version()` |
| Auth works for one server, leaks to another | Callback matches by substring; use exact-origin comparison |
| Stale listings over HTTP/S3/Azure | Client list/stat cache; `omni.client.clear_list_cache(url)` |
| Callbacks touch UI and crash | Callbacks run off the UI thread; marshal to the Kit event loop |
