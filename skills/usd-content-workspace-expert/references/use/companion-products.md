# Companion products

> **Applies to:** products deployed beside this stack · **Released:** versions as recorded in the [source skill package](../../NOTICE.md) · **This repo's snapshot:** none of these products is in this repository · **Sources:** the source skill package; [usd-caches](https://github.com/NVIDIA-Omniverse/usd-caches) and [ovpackage](https://github.com/NVIDIA-Omniverse/ovpackage) READMEs

These are separate Omniverse Content Services products with their own versions, charts, and documentation. What each product's own agent experts cover, and when to hand off to them: [related-experts.md](related-experts.md). They work with any Storage API-compatible backend. Recommend one only for the workload it helps, link its current documentation, and keep chart and image versions separate.

| Product | What it does | Recommend when | Where to learn more |
| --- | --- | --- | --- |
| Omniverse Content Cache (OVCC), formerly USD Content Cache | Shared server-side cache for content read through Kit. The first read populates it for later users. | Many users read the same assets | [OVCC docs](https://docs.omniverse.nvidia.com/ovcaches/ovcontentcache/latest/index.html), [usd-caches](https://github.com/NVIDIA-Omniverse/usd-caches) |
| Omniverse Derived Cache (OVDC), formerly Derived Data Cache Service | Shared cache for RTX derived data such as compiled materials, so other users skip regeneration | Many users render the same scenes | [OVDC docs](https://docs.omniverse.nvidia.com/ovcaches/ovderivedcache/latest/index.html), [usd-caches](https://github.com/NVIDIA-Omniverse/usd-caches) |
| Hub Workstation Cache | Local cache on a workstation, not on Kubernetes | Individual users want faster local iteration | [Hub Workstation Cache docs](https://docs.omniverse.nvidia.com/utilities/latest/cache/hub-workstation.html) |
| ovpackage (formerly WRAPP; the `wrapp` command remains) | Publishes and installs versioned asset packages on any Storage API backend; library and CLI | Teams need reproducible, versioned asset sets | [ovpackage](https://github.com/NVIDIA-Omniverse/ovpackage) |
| ovstorage | Embeddable storage library and bindings for applications | An application needs to reach the Storage Service without a full client stack | [ovstorage](https://github.com/NVIDIA-Omniverse/ovstorage) |
| ovlive | Live Edit client runtime | Applications join Live Edit sessions | [ovlive](https://github.com/NVIDIA-Omniverse/ovlive), [../components/live-edit.md](../components/live-edit.md) |

## Cache versions recorded with the release

- **OVCC:** chart `ovcontentcache` 4.0.4 with image `usd-content-cache:3.0.2`. The chart defaults to image 3.0.1, so set `image.tag: "3.0.2"` in your maintained values file. Do not use the chart version as the image tag.
- **OVDC:** chart `ovderivedcache` 6.0.1 with image `ovderivedcache:6.0.1`. Use the 6.0.1 chart's `presets.cpu` and `presets.ram` profiles for sizing. Do not use the legacy `ddcs` chart or its values for OVDC 6.x.

Confirm these against the cache products' own current documentation before deploying; cache releases do not follow this stack's releases.

## Known interaction

An upstream compatibility note describes an HTTP-to-HTTPS issue between Kit 110.1 and the content cache, using the historical labels "Content Cache 3.x/4.x". Those labels are not chart or image versions. Verify the affected combination, and apply the documented `NO_PROXY=".svc.cluster.local"` workaround only to it. See [../operate/known-issues.md](../operate/known-issues.md).
