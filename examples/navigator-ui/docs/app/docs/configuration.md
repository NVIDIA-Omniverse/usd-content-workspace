(storage-navigator-app-configuration-configuration)=
# Configuration

Storage Navigator reads a single runtime configuration file, `config.json`, once when the page loads. The chart renders this file from Helm values into a ConfigMap and mounts it into the container's static content root; there is no other runtime configuration surface. When the file is absent, for example on a local development server with no configuration deployed, Storage Navigator falls back to built-in defaults: no default discovery, no reference links, a 30-second request timeout, and a batch size of 25.

(storage-navigator-app-configuration-configuration-settings)=
## Configuration Settings

| Setting | Helm value (`config.*`) | Default | Description |
|---|---|---|---|
| Default discovery URL | `defaultDiscovery` | `""` (unset) | When set, Storage Navigator auto-connects to this discovery URL on load, hides the manual "Add Connection" UI, and surfaces every storage service the discovery endpoint advertises as a top-level node in the path tree. When unset, the app falls back to the manual "Add Connection" UX, letting the user enter a discovery URL themselves. |
| Reference links | `references` | `[]` | A list of external links shown to end users in a dropdown next to the application menu. Each entry has a `label`, a `url` opened in a new tab, and an optional `icon` (a Tabler icon name, without the `ti-` prefix). An entry with an omitted or unrecognized icon is shown without a glyph. |
| Content-Security-Policy | `csp` | `""` (unset) | When set, the nginx server emits this value as the page's `Content-Security-Policy` response header, along with an HSTS header. When unset, neither header is emitted. |

```{note}
The runtime configuration schema also defines a request timeout (`requestTimeout`, default 30000 milliseconds) and a batch size for parallel batch operations like delete and copy/paste (`batchSize`, default 25). The chart does not currently expose Helm values for these two settings. Deployments that need non-default values must supply their own `config.json` rendering outside this chart's ConfigMap.
```

### Configuring `config.json` in the Helm Chart

```yaml
config:
  csp: "default-src 'self'"
  defaultDiscovery: "https://discovery.example.com"
  references:
    - label: "Documentation"
      url: "https://docs.example.com"
      icon: "book"
    - label: "Support"
      url: "https://support.example.com"
      icon: "lifebuoy"
```

navigator-values.yaml

The rendered `config.json` is mounted at the container's static content root and fetched by the application once per session. The chart does not roll existing pods automatically when only these values change. An upgrade that changes `config.*` updates the ConfigMap, but running pods keep serving the previously mounted file until they restart. Trigger a rollout explicitly (for example `kubectl rollout restart`) after an upgrade that changes only configuration values. See {ref}`Installing the Chart <storage-navigator-app-deployment-installing-the-chart>` for the install and upgrade commands.

(storage-navigator-app-configuration-see-next)=
## See next

- {ref}`Features <storage-navigator-app-features-features>`. The folder tree, content views, uploads, tags, and versioning driven by the connected storage stack.
